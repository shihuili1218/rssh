//! file_ops 子模块：match_file / patch_file 完整实现。
//!
//! 拆出来的理由：file_ops 是一坨自成体系的复杂逻辑 ——
//! - 远端能力探测（python3 / perl / diff）
//! - 多行脚本 + 多行参数装进单行 ASCII 命令（`$'...'` ANSI-C quoting）
//! - 5 张审批卡片编排（match × 1，patch × 4：cp → modify → diff → mv）
//!
//! 与会话主框架（dialogue_turn / run_command / SFTP）耦合面很窄（只通过 Actor 的几个
//! pub(super) 方法和字段），按职责分离更利于阅读。
//!
//! **核心不变量**：所有送进 PTY 的 file_ops 命令在 shell 视角下必须是**单行**（无字面
//! `\n` / `\r`），避开 zsh ZLE multi-line quote race（详见 `ansi_c_quote` 注释）。
//! 非 ASCII 字符（中文 / emoji）在 `$'...'` 内直接透传 —— bash / zsh 都按 UTF-8
//! 解释字面字节，只有控制字符 / DEL / 单引号 / 反斜杠需要 `\xHH` 编码。

use serde_json::json;

use crate::ai::audit::AuditKind;
use crate::ai::llm::{ChatMessage, ToolCall};
use crate::ai::sanitize;
use crate::ai::tools::{MatchFileInput, PatchFileInput, MATCH_CONTEXT_DEFAULT, MATCH_CONTEXT_MAX};
use crate::error::{AppError, AppResult};

use super::{Actor, CommandOutcome};

// ─── 远端能力探测 ─────────────────────────────────────────────────

/// 远端 file_ops 工具能力。lazy 探测一次后缓存到 session 生命周期。
///
/// 设计原则：rssh 后端不再 cat 整文件回 PTY（避免 ANSI/scrollback/buffer 丢内容），
/// 改为让远端预制脚本读文件 + 算 count/context + 写 tmp，只回小 JSON。
/// patch_file 的 unified diff 走独立的 `diff -u` 命令算。
/// 因此 file_ops 整体硬依赖 python3 或 (perl + diff)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RemoteCapabilities {
    /// `python3`（首选）
    pub(super) python3: bool,
    /// `perl`（降级）—— `\Q...\E` 字面匹配
    pub(super) perl: bool,
    /// `diff -u`（patch_file 出审批 diff 必备）
    pub(super) diff: bool,
}

impl RemoteCapabilities {
    fn none() -> Self {
        Self {
            python3: false,
            perl: false,
            diff: false,
        }
    }
}

/// 探测命令：一行拿三个工具的可用性。输出形如 `py3=1 perl=1 diff=1`。
const PROBE_CMD: &str = r#"echo "py3=$(command -v python3 >/dev/null 2>&1 && echo 1 || echo 0) perl=$(command -v perl >/dev/null 2>&1 && echo 1 || echo 0) diff=$(command -v diff >/dev/null 2>&1 && echo 1 || echo 0)""#;

/// 解析探测命令输出 —— 找含 `py3=` `perl=` `diff=` 的那一行。
fn parse_capabilities(output: &str) -> RemoteCapabilities {
    let mut caps = RemoteCapabilities::none();
    for line in output.lines() {
        let line = line.trim();
        if !(line.contains("py3=") && line.contains("perl=") && line.contains("diff=")) {
            continue;
        }
        for token in line.split_whitespace() {
            if let Some(v) = token.strip_prefix("py3=") {
                caps.python3 = v == "1";
            } else if let Some(v) = token.strip_prefix("perl=") {
                caps.perl = v == "1";
            } else if let Some(v) = token.strip_prefix("diff=") {
                caps.diff = v == "1";
            }
        }
        return caps;
    }
    caps
}

// ─── JSON marker ─────────────────────────────────────────────────

/// JSON 输出包裹 marker：脚本把结果包在两个 marker 之间输出，
/// rssh 后端用此 marker 从 PTY 字节流里精准切出 JSON，规避 shell prompt /
/// ANSI 序列 / 命令回显的干扰。
const JSON_MARKER: &str = "__RSSH_JSON__";

/// 从 PTY 原始输出里抽出由 `JSON_MARKER` 包裹的内容。
/// 找不到一对独立成行的 marker 返回 None。
///
/// **按"独立成行"匹配**，不是 rfind 子串。脚本字面量里有 `M="__RSSH_JSON__"` 这行，
/// 脚本协议把 marker 输出为**整行**（前后都是 `\n`）。如果用 rfind：
///   1. 脚本源码 `M = "..."` 那行 echo 回显（marker 子串嵌在 `M = "<marker>"`）
///   2. 脚本输出的开头 marker（整行）
///   3. 脚本输出的结尾 marker（整行）
///   4. **更危险**：match_file 的 context 字段如果含 marker 字面（用户在文件里写了
///      `__RSSH_JSON__` 注释），子串扫到的位置会在 JSON 内部切，破坏解析。
///
/// 按行匹配杜绝以上风险：只有"整行 trim 后等于 marker"的位置才算锚点。脚本输出的
/// marker 一定独占一行；echo 回显里的字面量永远嵌在更长的源码行里，不会误匹配；
/// JSON payload 里的 marker（包在引号 + 上下文里）也不会独占一行。
fn extract_json_payload(pty_output: &str) -> Option<&str> {
    // 找最后两个"整行等于 marker"的行，记录字节起止 offset，回切原 &str。
    // line_indices 提供每行起始 offset + 内容；行尾 `\r\n` / `\r` / `\n` 都被 lines() 吃掉。
    let mut last: Option<(usize, usize)> = None;
    let mut prev: Option<(usize, usize)> = None;
    let mut cursor = 0;
    for raw_line in pty_output.split('\n') {
        let start = cursor;
        let end = cursor + raw_line.len();
        cursor = end + 1; // 跳过 `\n`
        // line trim 后等于 marker（兼容 `\r\n` —— raw_line 含末尾 `\r`，trim 掉）
        if raw_line.trim() == JSON_MARKER {
            prev = last;
            last = Some((start, end));
        }
    }
    let (_, prev_end) = prev?;
    let (last_start, _) = last?;
    // prev_end 是前一个 marker 行的结束（不含 `\n`）；下一字符是 `\n`，跳过再取
    let body_start = prev_end + 1;
    if body_start > last_start {
        return None;
    }
    Some(pty_output[body_start..last_start].trim_matches('\n').trim())
}

// ─── 解释器脚本 ───────────────────────────────────────────────────

/// match_file 的 python3 脚本。位置参数：path find before after
///
/// 不走 base64：find 直接作为 argv 字符串透传。shell 端 `ansi_c_quote(find)` 保证
/// 引号 / 空格 / 换行等都安全（`$'...'` 把真换行编为 `\n` 字面 escape，命令仍单行）。
/// Python 拿到的 sys.argv[2] 就是原始 UTF-8 字符串。
///
/// **远端 cap = 50 matches**：count 仍记真实总命中数，但 matches 数组只装前 50 个 +
/// `truncated` 标识。理由：后端只在解析后才截断，但前端 PTY 流要在 sentinel 出现前
/// 把全部 output buffer 住 —— 短 find（如 "a"）配大文件 (`/var/log/*`) 可产 MB 级 JSON，
/// 在 backend 拿到 payload 前先把前端 UI OOM。在远端就 cap 才彻底安全。
const PYTHON_MATCH_SCRIPT: &str = r#"import sys,json
M="__RSSH_JSON__"
def o(x):sys.stdout.write(M+"\n"+json.dumps(x,ensure_ascii=False)+"\n"+M+"\n")
p=sys.argv[1];f=sys.argv[2];b=int(sys.argv[3]);a=int(sys.argv[4])
try:t=open(p,"rb").read().decode("utf-8")
except FileNotFoundError:o({"error":"file_not_found"});sys.exit(0)
except Exception as e:o({"error":"io_error","message":str(e)});sys.exit(0)
m=[];c=0;i=0;n=len(f);L=len(t);K=50
while True:
 j=t.find(f,i)
 if j<0:break
 c+=1
 if len(m)<K:
  m.append({"line":t.count("\n",0,j)+1,"context":t[max(0,j-b):min(L,j+n+a)]})
 i=j+n
r={"count":c,"matches":m}
if c>K:r["truncated"]=True;r["matches_shown"]=K
o(r)
"#;

/// patch_file Stage 2 的 python3 脚本（in-place 改 tmp）。位置参数：tmp_path find replace expected
/// 前置：shell 已 `cp path tmp`。脚本读 tmp → 校验 count → in-place 替换 → JSON {count}。
/// count 不匹配时删除 tmp，避免半成品残留。
const PYTHON_PATCH_SCRIPT: &str = r#"import sys,json,os
M="__RSSH_JSON__"
def o(x):sys.stdout.write(M+"\n"+json.dumps(x,ensure_ascii=False)+"\n"+M+"\n")
p=sys.argv[1];f=sys.argv[2];r=sys.argv[3];e=int(sys.argv[4])
try:t=open(p,"rb").read().decode("utf-8")
except Exception as ex:o({"error":"io_error","message":str(ex)});sys.exit(0)
c=t.count(f)
if c!=e:
 try:os.unlink(p)
 except Exception:pass
 o({"error":"count_mismatch","actual":c,"expected":e});sys.exit(0)
try:open(p,"wb").write(t.replace(f,r).encode("utf-8"))
except Exception as ex:o({"error":"tmp_write_failed","message":str(ex)});sys.exit(0)
o({"count":c})
"#;

/// match_file 的 perl 脚本（降级路径）。位置参数：path find before after
/// `\Q...\E` 把 find 当字面量匹配，跳过 regex 元字符。utf8::decode 让长度按字符算。
/// 同 PYTHON_MATCH_SCRIPT：远端 cap = 50 matches，count 仍记真实总数。
const PERL_MATCH_SCRIPT: &str = r#"use strict;use warnings;use JSON::PP;
my $M="__RSSH_JSON__";
sub o{print $M,"\n",encode_json($_[0]),"\n",$M,"\n"}
my($p,$f,$b,$a)=@ARGV;utf8::decode($f);
open(my $h,'<:raw',$p)or do{o({error=>"file_not_found"});exit 0};
local $/;my $t=<$h>;close $h;utf8::decode($t);
my @m;my $c=0;my $i=0;my $n=length($f);my $L=length($t);my $K=50;
while((my $j=index($t,$f,$i))>=0){
 $c++;
 if(scalar(@m)<$K){
  my $line=1+(()=substr($t,0,$j)=~/\n/g);
  my $pre=$j-$b;$pre=0 if $pre<0;my $post=$j+$n+$a;$post=$L if $post>$L;
  push @m,{line=>$line,context=>substr($t,$pre,$post-$pre)};
 }
 $i=$j+$n;
}
my %r=(count=>$c,matches=>\@m);
if($c>$K){$r{truncated}=JSON::PP::true;$r{matches_shown}=$K}
o(\%r);
"#;

/// patch_file Stage 2 的 perl 脚本（降级路径）。位置参数：tmp_path find replace expected
const PERL_PATCH_SCRIPT: &str = r#"use strict;use warnings;use JSON::PP;
my $M="__RSSH_JSON__";
sub o{print $M,"\n",encode_json($_[0]),"\n",$M,"\n"}
my($p,$f,$r,$e)=@ARGV;utf8::decode($f);utf8::decode($r);
open(my $h,'<:raw',$p)or do{o({error=>"io_error",message=>"$!"});exit 0};
local $/;my $t=<$h>;close $h;utf8::decode($t);
my $c=()=$t=~/\Q$f\E/g;
if($c!=$e){unlink($p);o({error=>"count_mismatch",actual=>$c,expected=>$e});exit 0}
my $new=$t;$new=~s/\Q$f\E/$r/g;utf8::encode($new);
open(my $w,'>:raw',$p)or do{o({error=>"tmp_write_failed",message=>"$!"});exit 0};
print $w $new;close $w;o({count=>$c});
"#;

// ─── Interpreter ─────────────────────────────────────────────────

/// file_ops 解释器：python3 优先，perl 降级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interpreter {
    Python3,
    Perl,
}

impl Interpreter {
    fn binary(self) -> &'static str {
        match self {
            Self::Python3 => "python3",
            Self::Perl => "perl",
        }
    }
    /// `-c`（python3 inline script）或 `-e`（perl inline script）。
    fn script_flag(self) -> &'static str {
        match self {
            Self::Python3 => "-c",
            Self::Perl => "-e",
        }
    }
    /// 脚本字面量和位置参数之间的分隔符。
    ///
    /// Perl `-e` 之后 `@ARGV` 首元素若以 `-` 开头会被当未知 switch（"Unrecognized switch"），
    /// 必须 `--` 强制结束 switch 解析。
    ///
    /// Python `-c` 不消耗 `--` —— 加了反而让 `sys.argv[1]` 是字面 `--`，整个 argv 错位 1，
    /// 路径变成 `'--'` 让脚本读不到文件。
    fn args_prefix(self) -> &'static str {
        match self {
            Self::Python3 => "",
            Self::Perl => "-- ",
        }
    }
    fn match_script(self) -> &'static str {
        match self {
            Self::Python3 => PYTHON_MATCH_SCRIPT,
            Self::Perl => PERL_MATCH_SCRIPT,
        }
    }
    fn patch_script(self) -> &'static str {
        match self {
            Self::Python3 => PYTHON_PATCH_SCRIPT,
            Self::Perl => PERL_PATCH_SCRIPT,
        }
    }
}

/// 选 file_ops 解释器：python3 优先，perl 降级。都没就 None。
/// （diff 只有 patch_file 需要，单独在 handle_patch_file 校验。）
fn select_interpreter(caps: RemoteCapabilities) -> Option<Interpreter> {
    if caps.python3 {
        Some(Interpreter::Python3)
    } else if caps.perl {
        Some(Interpreter::Perl)
    } else {
        None
    }
}

// ─── Quoting & path helpers ──────────────────────────────────────

/// ANSI-C quoting (`$'...'`)：shell 把 `\n` `\t` `\\` 等转义序列展开为真字符，
/// 但**整段字面量在 shell 视角是单行 ASCII** —— 因此长多行脚本能塞进一条单行命令，
/// 不触发 zsh ZLE 的 multi-line quote race（粘贴长命令进 PTY 时 p10k 会渲染连续
/// quote> prompt、ZLE buffer 错乱、命令永不执行）。
///
/// **单引号编码为 `\x27`，不是 `\'`** —— 实测含多个 `\'` 的长命令在 zsh + p10k
/// 下偶发卡死（同一命令含 0 个 `\'` 时稳定通过）。怀疑 ZLE 在 `$'...'` 状态机内
/// 对 `\'` 的转义切换有 race。两者语义等价，但 transition 路径分离 —— hardening。
///
/// **其他 C0 控制字符（0x00-0x1F 中除 `\n`/`\r`/`\t` 外的）和 DEL (0x7F) 也 hex 化** ——
/// 安全：LLM 可控的 `find` / `replace` 含 ESC (`\x1b`) / BEL (`\x07`) / CSI 序列等会通过
/// PTY echo 触发 terminal escape injection（隐藏文本、伪造 prompt、改终端 title 等）。
/// 让 on-wire 命令保持 printable ASCII，回程的 PTY echo 也就不会被解释为 escape sequence。
/// 程序拿到的 argv 仍是原字节（shell 把 `\xHH` 解码回原 byte）。
///
/// 适用 bash / zsh（远端常见 shell）；POSIX 纯 sh / busybox sh / dash 不支持。
/// file_ops 已经硬依赖 python3 / perl，那些极简环境一般连解释器都没有，能力探测
/// 阶段就被拒。
fn ansi_c_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    out.push('$');
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\'' => out.push_str("\\x27"),
            '\\' => out.push_str("\\\\"),
            // 其他 C0 controls (0x00-0x1F 中未在上面特处理的) + DEL (0x7F) → \xHH
            c if (c as u32) < 0x20 || (c as u32) == 0x7F => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// POSIX 单引号转义。所有 rssh 后端拼装的 PTY 命令都走这个，避免路径含
/// 空格/特殊字符时被 shell 错误解析。
///
/// 转义规则：把整个串包在单引号里；输入中的每个单引号替换为 `'\''`
/// （关闭引号 → 转义字面单引号 → 重开引号）。这是 POSIX shell 通用形态，
/// bash/dash/zsh/ash/busybox 都正确解析，且能可靠覆盖所有 shell 元字符
/// （$ ` * ? & | ; < > 等单引号内都按字面）。
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 校验 path 不含会破坏单行命令的字符。
///
/// 必须拒：
/// - `\n` / `\r`：shell_quote 用 POSIX 单引号包，单引号内换行**字面保留**，命令字节流就有
///   真 `\n` byte，重新触发 zsh ZLE multi-line quote race —— 这正是本 PR 拿 ansi_c_quote 把
///   脚本/find/replace 全单行化要规避的问题。path 走的是 shell_quote 不经 ansi_c_quote，必须
///   在入口直接拒。
/// - 其他 C0 控制字符 (0x00-0x1F) 和 DEL (0x7F)：terminal escape injection 防御（同 ansi_c_quote
///   注释里 ESC/BEL 的理由）。Unix 文件名理论上允许这些字符，但 rssh 用户场景里它们没合理用法。
fn validate_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("empty path");
    }
    for c in path.chars() {
        let code = c as u32;
        if c == '\n' || c == '\r' {
            return Err("path contains newline (\\n or \\r) — would break the single-line command");
        }
        if code < 0x20 || code == 0x7F {
            return Err("path contains control character");
        }
    }
    Ok(())
}

/// 远端文件路径准备：处理 `~` / `~/...` 前缀展开，其余走 shell_quote。
///
/// 直接 shell_quote `~/foo` → `'~/foo'`，单引号禁掉 `~` 展开，shell 看到字面波浪号
/// → 文件找不到。这里把 `~/` 替换为 `"$HOME"/`，让 `$HOME` 在双引号里展开，
/// 而 rest 部分仍走单引号保护特殊字符。
///
/// `~user/...` 形态（其他用户的 home）不支持 —— 这种用例罕见，按字面单引号处理
/// （LLM 想用就报"路径不存在"，明确的错误胜过悄悄改写）。
fn prepare_remote_path(path: &str) -> String {
    if path == "~" {
        "\"$HOME\"".to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("\"$HOME\"/{}", shell_quote(rest))
    } else {
        shell_quote(path)
    }
}

// ─── 命令构造 ────────────────────────────────────────────────────

/// 拼一条 `<interp> -c|-e $'...' [-- ] arg1 arg2 ...` 形态的解释器命令。
/// 单行 ASCII（脚本里的换行靠 `$'...'` 的 `\n` 编码），避免 PTY ZLE quote race。
fn build_script_cmd(interp: Interpreter, script: &str, args: &[String]) -> String {
    let args_part = if args.is_empty() {
        String::new()
    } else {
        format!(" {}{}", interp.args_prefix(), args.join(" "))
    };
    format!(
        "{} {} {}{}",
        interp.binary(),
        interp.script_flag(),
        ansi_c_quote(script),
        args_part,
    )
}

/// match_file 命令：搜索 path 中 find 出现的位置，回 JSON。
///
/// `find` 走 `ansi_c_quote` —— LLM 提供的 find 文本可能含真换行（如多行 YAML 块），
/// shell_quote 包后字面 `\n` byte 仍会出现在 shell 输入流里，触发 zsh ZLE 的
/// multi-line quote race。`$'...'` 把所有换行编码为 `\n` 转义序列，让命令在
/// shell 视角下保持单行 ASCII。
fn build_match_cmd(interp: Interpreter, path: &str, find: &str, before: u32, after: u32) -> String {
    build_script_cmd(
        interp,
        interp.match_script(),
        &[
            prepare_remote_path(path),
            ansi_c_quote(find),
            shell_quote(&before.to_string()),
            shell_quote(&after.to_string()),
        ],
    )
}

/// patch_file modify 命令：脚本在 tmp 上 in-place 替换，校验 count，回 JSON。
/// 前置：调用方已经独立跑过 `cp_cmd` 把原文复制到 tmp。
///
/// `find` / `replace` 都走 `ansi_c_quote`（理由同 `build_match_cmd`）。
fn build_modify_cmd(
    interp: Interpreter,
    tmp: &str,
    find: &str,
    replace: &str,
    expected: u32,
) -> String {
    build_script_cmd(
        interp,
        interp.patch_script(),
        &[
            prepare_remote_path(tmp),
            ansi_c_quote(find),
            ansi_c_quote(replace),
            shell_quote(&expected.to_string()),
        ],
    )
}

/// `\cp -- path tmp`：patch_file 第一步，把原文复制到同目录 tmp（保证后续 mv 单 rename(2)）。
///
/// **`\` 前缀**：远端用户可能配 `alias cp='cp -i'`/`alias mv='mv -i'`，互动询问会让
/// 命令挂到 timeout。bash/zsh 的转义命令头（`\cp` / `\mv`）跳过 alias 展开但仍命中
/// 真正可执行文件。`\diff` 同理（虽然 `alias diff='diff --color'` 一般不影响 exit）。
fn build_cp_cmd(path: &str, tmp: &str) -> String {
    format!(
        "\\cp -- {} {}",
        prepare_remote_path(path),
        prepare_remote_path(tmp),
    )
}

/// `\diff -u path tmp`：patch_file 第三步，给用户审批用的 unified diff。
/// exit 0=无差异 / 1=有差异（**正常**）/ ≥2=工具失败 —— 调用方必须按此判，不能套
/// "exit != 0 ⇒ 错误"的通用规则。
fn build_diff_cmd(path: &str, tmp: &str) -> String {
    format!(
        "\\diff -u -- {} {}",
        prepare_remote_path(path),
        prepare_remote_path(tmp),
    )
}

/// `\mv -- tmp path`：patch_file 最后一步，原子覆盖。
fn build_mv_cmd(tmp: &str, path: &str) -> String {
    format!(
        "\\mv -- {} {}",
        prepare_remote_path(tmp),
        prepare_remote_path(path),
    )
}

// ─── Actor 上的 file_ops 方法 ─────────────────────────────────────

impl Actor {
    /// 第一次 file_ops 时探测远端能力（python3 / perl / diff），结果缓存到 session 结束。
    /// 后续 file_ops 直接读缓存。
    ///
    /// probe 走 `internal_command`（不弹卡片）—— 一行只读 echo，对用户透明，
    /// 没必要让用户为它单独点一次同意。实际的 cp/python/diff/mv 都走 `command_proposed` 弹卡片。
    async fn ensure_remote_caps(&mut self) -> AppResult<RemoteCapabilities> {
        if let Some(c) = self.remote_caps {
            return Ok(c);
        }
        let probe_tc_id = uuid::Uuid::new_v4().to_string();
        let cmd_id = uuid::Uuid::new_v4().to_string();
        // PROBE_CMD is a POSIX-only shell script (python3/perl/diff `which` probes).
        // On Windows targets the probe itself can't succeed, but the sentinel
        // template still has to match shell_kind so the front-end finds the marker.
        let (sentinel, full_cmd) = self.cfg.shell_kind.sentinel_command(PROBE_CMD);

        self.audit_push(AuditKind::Note {
            message: "file_ops: probing remote capabilities (python3 / perl / diff)".into(),
        });
        self.emit(
            "internal_command",
            json!({
                "id": cmd_id,
                "tool_call_id": probe_tc_id,
                "cmd": PROBE_CMD,
                "full_cmd": full_cmd,
                "sentinel": sentinel,
            }),
        );

        let output = match self.wait_command_outcome(&probe_tc_id).await? {
            CommandOutcome::Result { output, .. } => output,
            CommandOutcome::Rejected { reason } => {
                return Err(AppError::other(
                    "caps_probe_aborted",
                    json!({ "reason": reason }),
                ));
            }
        };
        let caps = parse_capabilities(&output);
        self.audit_push(AuditKind::Note {
            message: format!(
                "file_ops: caps probed — python3={} perl={} diff={}",
                caps.python3, caps.perl, caps.diff
            ),
        });
        self.remote_caps = Some(caps);
        Ok(caps)
    }

    /// 跑一条 file_ops 命令：弹 `command_proposed` 卡片 → 等用户审批 / 执行 → 审计 + 返回结果。
    ///
    /// 跟 `handle_run_command` 的区别：
    /// 1. **不走 `sanitize::validate`**。cp / mv / python3 等会被 DESTRUCTIVE / INTERPRETERS_DENIED
    ///    拒掉，但 file_ops 的命令是 rssh 后端构造的固定模板（不是 LLM 直传），由 rssh 自己负责安全。
    /// 2. `explain` / `side_effect` 由调用方硬编码传入，不来自 LLM。
    /// 3. 返回原始 `CommandOutcome` 给上层做错误分支（多步流程要根据 exit / JSON marker
    ///    决定是否中断、是否提示 tmp 残留等）。
    ///
    /// `kind` 透传给前端 dialog —— 前端按 kind 查 per-tool 自动批准设置：
    /// `patch_cp` / `patch_modify` / `patch_diff` / `patch_mv` 四张 patch_file 卡片各自一档；
    /// `match_file` 一张卡片自己一档（read-only，默认自动批）。danger_mode 关时全部需人审。
    ///
    /// `diff` 透传给前端：patch_file 第 4 张 mv 卡片把第 3 张 diff 命令的输出当作审批
    /// 材料展示在卡片上 —— 用户审批 mv 时不用回滚翻第 3 张结果区域。其他卡片传 None。
    async fn run_file_op(
        &mut self,
        tool_call_id: &str,
        cmd: String,
        explain: String,
        side_effect: String,
        timeout_s: u32,
        kind: Option<&str>,
        diff: Option<&str>,
    ) -> AppResult<CommandOutcome> {
        let cmd_id = uuid::Uuid::new_v4().to_string();
        let (sentinel, full_cmd) = self.cfg.shell_kind.sentinel_command(&cmd);

        self.audit_push(AuditKind::CommandProposed {
            id: cmd_id.clone(),
            cmd: cmd.clone(),
            explain: explain.clone(),
            side_effect: side_effect.clone(),
        });
        let started_at = std::time::Instant::now();

        let mut payload = json!({
            "id": cmd_id,
            "tool_call_id": tool_call_id,
            "cmd": cmd,
            "full_cmd": full_cmd,
            "sentinel": sentinel,
            "explain": explain,
            "side_effect": side_effect,
            "timeout_s": timeout_s,
        });
        if let Some(k) = kind {
            payload["kind"] = json!(k);
        }
        if let Some(d) = diff {
            payload["diff"] = json!(d);
        }
        self.emit("command_proposed", payload);

        match self.wait_command_outcome(tool_call_id).await? {
            CommandOutcome::Rejected { reason } => {
                self.record_rejection(&cmd_id, &reason);
                Ok(CommandOutcome::Rejected { reason })
            }
            CommandOutcome::Result {
                exit_code,
                output,
                timed_out,
                early_terminated,
            } => {
                let redacted = sanitize::redact(&output, &self.cfg.redact_rules);
                let trunc = sanitize::truncate(&redacted, self.cfg.max_output_bytes);
                self.emit(
                    "command_completed",
                    json!({
                        "id": cmd_id,
                        "exit_code": exit_code,
                        "timed_out": timed_out,
                        "early_terminated": early_terminated,
                        "output": trunc.text,
                        "original_bytes": trunc.original_bytes,
                        "truncated_bytes": trunc.truncated_bytes,
                        "duration_ms": started_at.elapsed().as_millis() as u64,
                    }),
                );
                self.audit_push(AuditKind::CommandExecuted {
                    id: cmd_id,
                    exit_code,
                    output_redacted: trunc.text.clone(),
                    original_bytes: trunc.original_bytes,
                    truncated_bytes: trunc.truncated_bytes,
                    duration_ms: started_at.elapsed().as_millis() as u64,
                });
                // 返回 redact 前的 raw output —— 上层要从中 extract_json_payload，
                // 脱敏后的版本里 marker 可能被改写。脱敏只作用于 audit / UI 显示，
                // 不影响 rssh 内部解析。
                Ok(CommandOutcome::Result {
                    exit_code,
                    output,
                    timed_out,
                    early_terminated,
                })
            }
        }
    }

    pub(super) async fn handle_match_file(&mut self, tc: ToolCall) -> AppResult<ChatMessage> {
        let input: MatchFileInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => return Ok(self.make_tool_error(&tc.id, &format!("Failed to parse input: {e}"))),
        };
        if let Err(e) = validate_path(&input.path) {
            return Ok(self.make_tool_error(&tc.id, &format!("match_file: invalid path — {e}")));
        }
        if input.find.is_empty() {
            return Ok(self.make_tool_error(
                &tc.id,
                "match_file: `find` must not be empty (it would match the entire file).",
            ));
        }
        let before = input
            .before
            .unwrap_or(MATCH_CONTEXT_DEFAULT)
            .min(MATCH_CONTEXT_MAX);
        let after = input
            .after
            .unwrap_or(MATCH_CONTEXT_DEFAULT)
            .min(MATCH_CONTEXT_MAX);

        let caps = self.ensure_remote_caps().await?;
        let interp = match select_interpreter(caps) {
            Some(i) => i,
            None => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    "match_file: remote system lacks python3 / perl — rssh cannot inspect the file. \
                     Tell the user to install python3 (preferred) or perl.",
                ));
            }
        };

        let cmd = build_match_cmd(interp, &input.path, &input.find, before, after);
        let outcome = self
            .run_file_op(
                &tc.id,
                cmd,
                format!("match_file: search `{}` (read-only)", input.path),
                "read-only".into(),
                60,
                Some("match_file"),
                None,
            )
            .await?;

        let (exit_code, output) = match outcome {
            CommandOutcome::Rejected { reason } => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!("User rejected match_file. Reason: {reason}."),
                ));
            }
            CommandOutcome::Result {
                exit_code, output, ..
            } => (exit_code, output),
        };

        let payload = match (exit_code, extract_json_payload(&output)) {
            (0, Some(p)) => p.to_string(),
            _ => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "match_file: remote script failed (exit {exit_code}). Output: {}",
                        output.chars().take(400).collect::<String>()
                    ),
                ));
            }
        };
        // Remote returns `{"error": ...}` for file_not_found / io_error —
        // bubble those as tool errors so the LLM doesn't treat count=0 as
        // "no matches" and barrel into a patch_file that also fails.
        let parsed: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!("match_file: malformed JSON ({e}). Raw: {payload}"),
                ));
            }
        };
        if let Some(err) = parsed.get("error").and_then(|e| e.as_str()) {
            let msg = match err {
                "file_not_found" => format!("match_file: file not found: {}", input.path),
                "io_error" => format!(
                    "match_file: io_error reading {} ({})",
                    input.path,
                    parsed.get("message").and_then(|m| m.as_str()).unwrap_or("")
                ),
                other => format!("match_file: remote error {other}: {payload}"),
            };
            return Ok(self.make_tool_error(&tc.id, &msg));
        }
        let count = parsed.get("count").and_then(|c| c.as_u64());
        let matches_shown = parsed
            .get("matches")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let remote_truncated = parsed
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Remote script caps `matches` at K=50; we forward as-is.
        self.audit_push(AuditKind::Note {
            message: format!(
                "match_file: {} interp={} count={} matches_shown={}{}",
                input.path,
                interp.binary(),
                count.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                matches_shown,
                if remote_truncated { " (remote-truncated)" } else { "" }
            ),
        });
        // Redact at the insertion boundary so `redact_message` in dialogue_turn
        // can skip a second pass on the JSON payload (which would corrupt
        // structured hashes — sha256/git-oid — inside string fields).
        let redacted_payload = sanitize::redact(&payload, &self.cfg.redact_rules);
        Ok(Self::make_tool_result(&tc.id, redacted_payload, true))
    }

    pub(super) async fn handle_patch_file(&mut self, tc: ToolCall) -> AppResult<ChatMessage> {
        let input: PatchFileInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => return Ok(self.make_tool_error(&tc.id, &format!("Failed to parse input: {e}"))),
        };
        if let Err(e) = validate_path(&input.path) {
            return Ok(self.make_tool_error(&tc.id, &format!("patch_file: invalid path — {e}")));
        }
        if input.find.is_empty() {
            return Ok(self.make_tool_error(
                &tc.id,
                "patch_file: `find` must not be empty (use match_file to discover what to change).",
            ));
        }
        if input.expected_count == 0 {
            return Ok(self.make_tool_error(
                &tc.id,
                "patch_file: `expected_count` must be >= 1. Use match_file first to discover the actual count, then pass it here.",
            ));
        }

        let caps = self.ensure_remote_caps().await?;
        let interp = match select_interpreter(caps) {
            Some(i) => i,
            None => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    "patch_file: remote system lacks python3 / perl — rssh cannot patch the file. \
                     Tell the user to install python3 (preferred) or perl.",
                ));
            }
        };
        if !caps.diff {
            return Ok(self.make_tool_error(
                &tc.id,
                "patch_file: remote system lacks `diff` — rssh cannot show the diff for approval. \
                 Tell the user to install diffutils.",
            ));
        }

        // tmp 与 path 同目录，保证后续 mv 走单 rename(2)（同 filesystem，原子）。
        let tmp_suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        let tmp_path = format!("{}.rssh-{}", input.path, tmp_suffix);

        // ── Card 1/4: cp 原文到 tmp ──────────────────────────────
        let outcome = self
            .run_file_op(
                &tc.id,
                build_cp_cmd(&input.path, &tmp_path),
                format!("patch_file 1/4: copy `{}` to staging `{}`", input.path, tmp_path),
                format!("Create {}", tmp_path),
                30,
                Some("patch_cp"),
                None,
            )
            .await?;
        match outcome {
            CommandOutcome::Rejected { reason } => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!("User rejected step 1/4 (cp). Reason: {reason}."),
                ));
            }
            CommandOutcome::Result {
                exit_code, output, ..
            } if exit_code != 0 => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "patch_file 1/4 (cp) failed (exit {exit_code}). Output: {}",
                        output.chars().take(400).collect::<String>()
                    ),
                ));
            }
            _ => {}
        }

        // ── Card 2/4: 解释器 in-place 改 tmp（校验 count → 替换 → 回 {count}） ──
        let outcome = self
            .run_file_op(
                &tc.id,
                build_modify_cmd(interp, &tmp_path, &input.find, &input.replace, input.expected_count),
                format!(
                    "patch_file 2/4: replace {} occurrence(s) in `{}` (via {})",
                    input.expected_count, tmp_path, interp.binary()
                ),
                format!("Modify {} in place", tmp_path),
                60,
                Some("patch_modify"),
                None,
            )
            .await?;
        let (modify_exit, modify_output) = match outcome {
            CommandOutcome::Rejected { reason } => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "User rejected step 2/4 (modify). Reason: {reason}. Tmp at {tmp_path} (user can inspect / rm)."
                    ),
                ));
            }
            CommandOutcome::Result {
                exit_code, output, ..
            } => (exit_code, output),
        };
        if modify_exit != 0 {
            return Ok(self.make_tool_error(
                &tc.id,
                &format!(
                    "patch_file 2/4 (modify) failed (exit {modify_exit}). Tmp at {tmp_path}. Output: {}",
                    modify_output.chars().take(400).collect::<String>()
                ),
            ));
        }
        let payload = match extract_json_payload(&modify_output) {
            Some(p) => p.to_string(),
            None => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "patch_file 2/4 (modify): no JSON marker in output. Tmp at {tmp_path}. Output: {}",
                        modify_output.chars().take(400).collect::<String>()
                    ),
                ));
            }
        };
        let parsed: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!("patch_file 2/4 (modify): malformed JSON ({e}). Tmp at {tmp_path}. Raw: {payload}"),
                ));
            }
        };
        if let Some(err) = parsed.get("error").and_then(|e| e.as_str()) {
            let msg = match err {
                "count_mismatch" => format!(
                    "patch_file: count mismatch — file has {} occurrence(s), expected {}. Re-run match_file to refresh, then call patch_file with the correct expected_count.",
                    parsed.get("actual").and_then(|a| a.as_u64()).unwrap_or(0),
                    parsed.get("expected").and_then(|a| a.as_u64()).unwrap_or(0),
                ),
                "io_error" | "tmp_write_failed" => format!(
                    "patch_file: remote {} ({}). Tmp may exist at {tmp_path}.",
                    err,
                    parsed.get("message").and_then(|m| m.as_str()).unwrap_or("")
                ),
                other => format!("patch_file: remote error {other}: {payload}"),
            };
            return Ok(self.make_tool_error(&tc.id, &msg));
        }
        let count = parsed
            .get("count")
            .and_then(|c| c.as_u64())
            .unwrap_or(input.expected_count as u64) as u32;

        // ── Card 3/4: diff -u path tmp（输出展示给用户）─────────
        // exit 0=无差异 / 1=有差异（正常）/ >=2=工具失败
        let outcome = self
            .run_file_op(
                &tc.id,
                build_diff_cmd(&input.path, &tmp_path),
                format!("patch_file 3/4: review diff of `{}` vs staged tmp", input.path),
                "read-only (display diff for review)".into(),
                30,
                Some("patch_diff"),
                None,
            )
            .await?;
        let (diff_exit, diff_raw) = match outcome {
            CommandOutcome::Rejected { reason } => {
                return Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "User rejected step 3/4 (diff). Reason: {reason}. Tmp at {tmp_path} (user can inspect / rm)."
                    ),
                ));
            }
            CommandOutcome::Result {
                exit_code, output, ..
            } => {
                if exit_code >= 2 {
                    return Ok(self.make_tool_error(
                        &tc.id,
                        &format!(
                            "patch_file 3/4 (diff) failed (exit {exit_code}). Tmp at {tmp_path}. Output: {}",
                            output.chars().take(400).collect::<String>()
                        ),
                    ));
                }
                (exit_code, output)
            }
        };

        // diff exit 0 = modify step was a no-op (replacement equals original).
        // Skip mv so we don't churn mtime/inode for tools (make / caches / backups)
        // that key off them. Leave tmp for the user to inspect / rm.
        if diff_exit == 0 {
            self.audit_push(AuditKind::Note {
                message: format!(
                    "patch_file no-op: {} matched count={} but replacement equals original (interp={})",
                    input.path, count, interp.binary()
                ),
            });
            let result = json!({
                "diff": "",
                "diff_truncated_bytes": 0,
                "changed": 0,
                "no_op": true,
                "note": format!("patch_file: no-op (replacement matches original; mv skipped). Tmp at {tmp_path}, user may rm if not needed."),
            })
            .to_string();
            // Redact at insertion boundary, mark pre_redacted so redact_message
            // doesn't run a second pass on this structured JSON.
            let redacted = sanitize::redact(&result, &self.cfg.redact_rules);
            return Ok(Self::make_tool_result(&tc.id, redacted, true));
        }

        // diff 走 max_output_bytes 截断 —— 原始 diff 可能很长（大文件差异、二进制差异等），
        // 不截断会同时撑爆 (a) mv 卡片的 emit payload，导致前端 UI 渲染卡顿；(b) ToolResult
        // 进 LLM history，吃掉 context window。截断后保留前缀供人审 / LLM 概览，丢失尾部
        // 由 `diff_truncated_bytes` 显式告知。
        let diff_trunc = sanitize::truncate(&diff_raw, self.cfg.max_output_bytes);
        let diff = diff_trunc.text;
        let diff_truncated_bytes = diff_trunc.truncated_bytes;

        // ── Card 4/4: mv tmp → path（原子覆盖） ─────────────────
        let outcome = self
            .run_file_op(
                &tc.id,
                build_mv_cmd(&tmp_path, &input.path),
                format!("patch_file 4/4: apply via `mv {} -> {}`", tmp_path, input.path),
                format!("Atomic rename {} -> {}", tmp_path, input.path),
                30,
                Some("patch_mv"),
                Some(&diff),
            )
            .await?;
        match outcome {
            CommandOutcome::Rejected { reason } => {
                Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "User rejected step 4/4 (mv). Reason: {reason}. Tmp at {tmp_path} (still staged, user can inspect / rm)."
                    ),
                ))
            }
            CommandOutcome::Result {
                exit_code, output, ..
            } if exit_code != 0 => {
                Ok(self.make_tool_error(
                    &tc.id,
                    &format!(
                        "patch_file 4/4 (mv) failed (exit {exit_code}). Tmp at {tmp_path}. Output: {}",
                        output.chars().take(400).collect::<String>()
                    ),
                ))
            }
            CommandOutcome::Result { .. } => {
                self.audit_push(AuditKind::Note {
                    message: format!(
                        "patch_file: {} interp={} changed={}",
                        input.path,
                        interp.binary(),
                        count
                    ),
                });
                let result = json!({
                    "diff": diff,
                    "diff_truncated_bytes": diff_truncated_bytes,
                    "changed": count,
                })
                .to_string();
                // Redact at insertion boundary, mark pre_redacted to skip
                // the second pass that would substitute `<REDACTED:hex>`
                // into sha256/git-oid strings embedded in the diff.
                let redacted = sanitize::redact(&result, &self.cfg.redact_rules);
                Ok(Self::make_tool_result(&tc.id, redacted, true))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── shell_quote ────────────────────────────────────────────────

    #[test]
    fn shell_quote_plain_string() {
        // 无元字符的字符串，直接套单引号即可
        assert_eq!(shell_quote("hello"), "'hello'");
        assert_eq!(shell_quote("/tmp/foo.yml"), "'/tmp/foo.yml'");
    }

    #[test]
    fn shell_quote_empty_string() {
        // 空串也合法 —— shell 看到 '' 等同于空参数
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn shell_quote_with_spaces() {
        // 路径含空格是真实场景（用户主目录 / 含空格的文件名）
        assert_eq!(shell_quote("/tmp/has space/foo"), "'/tmp/has space/foo'");
    }

    #[test]
    fn shell_quote_single_quote_escape() {
        // 核心规则：单引号本身用 '\'' 拼出（关闭引号→反斜杠转义字面引号→重开引号）
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("'"), "''\\'''");
        assert_eq!(shell_quote("a'b'c"), "'a'\\''b'\\''c'");
    }

    #[test]
    fn shell_quote_double_quote_passthrough() {
        // 双引号在单引号内按字面，不转义
        assert_eq!(shell_quote(r#"say "hi""#), r#"'say "hi"'"#);
    }

    #[test]
    fn shell_quote_dollar_and_backtick_neutralized() {
        // $VAR / `cmd` 在单引号内不展开 —— 这是用单引号而非双引号的原因
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("`whoami`"), "'`whoami`'");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn shell_quote_shell_metacharacters_inert() {
        // |, &, ;, <, >, * 等在单引号内全部按字面
        assert_eq!(shell_quote("a|b&c;d"), "'a|b&c;d'");
        assert_eq!(shell_quote("> /etc/passwd"), "'> /etc/passwd'");
        assert_eq!(shell_quote("*"), "'*'");
        assert_eq!(shell_quote("~/foo"), "'~/foo'");
    }

    #[test]
    fn shell_quote_backslash_passthrough() {
        // 反斜杠在单引号内不是转义字符 —— 按字面输出
        assert_eq!(shell_quote(r"a\b"), r"'a\b'");
        assert_eq!(shell_quote(r"\n"), r"'\n'");
    }

    #[test]
    fn shell_quote_newline_preserved() {
        // 内容含换行也安全 —— shell 在单引号内保留 \n
        assert_eq!(shell_quote("line1\nline2"), "'line1\nline2'");
    }

    #[test]
    fn shell_quote_multibyte_chars() {
        // 非 ASCII（中文 / emoji 等）原样保留
        assert_eq!(shell_quote("文件名.txt"), "'文件名.txt'");
        assert_eq!(shell_quote("emoji 🦀"), "'emoji 🦀'");
    }

    #[test]
    fn shell_quote_idempotent_when_no_single_quote() {
        // 不含单引号的字符串两次套引号会双层（不是 idempotent —— 这是正确的，
        // 因为 shell_quote 输出本身含单引号，再次套需要二次转义）。
        // 这个测试只是文档化"不要重复调用"的约束。
        let once = shell_quote("foo");
        let twice = shell_quote(&once);
        assert_ne!(once, twice);
        assert!(twice.contains("\\'"));
    }

    // ─── validate_path ──────────────────────────────────────────────

    #[test]
    fn validate_path_rejects_newline_and_cr() {
        // 防御 ZLE race：path 含真 \n / \r 会让 shell_quote 输出含真换行 byte，破坏单行命令前提。
        assert!(validate_path("foo\nbar").is_err());
        assert!(validate_path("foo\rbar").is_err());
        assert!(validate_path("a\r\nb").is_err());
    }

    #[test]
    fn validate_path_rejects_other_c0_and_del() {
        // terminal injection 防御：path 含 ESC/BEL 等 C0 / DEL 也拒
        assert!(validate_path("foo\x1bbar").is_err());
        assert!(validate_path("foo\x07bar").is_err());
        assert!(validate_path("foo\x00bar").is_err());
        assert!(validate_path("foo\x7fbar").is_err());
    }

    #[test]
    fn validate_path_rejects_empty() {
        assert!(validate_path("").is_err());
    }

    #[test]
    fn validate_path_passes_normal_paths() {
        // 常规路径：绝对路径、~ 路径、含空格、中文、glob 字符（shell_quote 处理），都允许
        assert!(validate_path("/etc/hosts").is_ok());
        assert!(validate_path("~/foo.yml").is_ok());
        assert!(validate_path("/tmp/has space/file").is_ok());
        assert!(validate_path("文件名.txt").is_ok());
        assert!(validate_path("a'b").is_ok());
        assert!(validate_path("*.log").is_ok());
    }

    // ─── prepare_remote_path（~ 路径展开） ──────────────────────────

    #[test]
    fn prepare_path_plain_passes_through_to_shell_quote() {
        assert_eq!(prepare_remote_path("/tmp/foo"), "'/tmp/foo'");
        assert_eq!(prepare_remote_path("foo"), "'foo'");
        assert_eq!(prepare_remote_path(""), "''");
    }

    #[test]
    fn prepare_path_bare_tilde_expands_to_home() {
        // 单独的 ~ → "$HOME"（不带末尾斜杠）
        assert_eq!(prepare_remote_path("~"), "\"$HOME\"");
    }

    #[test]
    fn prepare_path_tilde_slash_expands_then_quotes_rest() {
        // ~/foo → "$HOME"/'foo'，shell 拼起来等同于 $HOME/foo
        assert_eq!(prepare_remote_path("~/foo"), "\"$HOME\"/'foo'");
        assert_eq!(prepare_remote_path("~/a/b/c.yml"), "\"$HOME\"/'a/b/c.yml'");
    }

    #[test]
    fn prepare_path_tilde_with_special_chars_in_rest() {
        // rest 含空格 / 单引号 必须被 shell_quote 正确处理
        assert_eq!(
            prepare_remote_path("~/has space/file"),
            "\"$HOME\"/'has space/file'"
        );
        assert_eq!(
            prepare_remote_path("~/it's.txt"),
            r#""$HOME"/'it'\''s.txt'"#
        );
    }

    #[test]
    fn prepare_path_other_user_home_not_expanded() {
        // ~user/... 形态：我们不展开（罕见，悄悄改写不如让用户看到明确路径错）
        // 走 shell_quote 的字面处理，shell 看到 '~user/foo' 找不到文件
        assert_eq!(prepare_remote_path("~root/foo"), "'~root/foo'");
        assert_eq!(prepare_remote_path("~user"), "'~user'");
    }

    #[test]
    fn prepare_path_tilde_in_middle_not_expanded() {
        // 只识别开头的 `~/` 或单独 `~`，路径中间的 ~ 不动
        assert_eq!(prepare_remote_path("/foo/~/bar"), "'/foo/~/bar'");
        assert_eq!(prepare_remote_path("foo~bar"), "'foo~bar'");
    }

    // ─── parse_capabilities ─────────────────────────────────────────

    #[test]
    fn parse_caps_all_present() {
        let r = parse_capabilities("py3=1 perl=1 diff=1\n");
        assert!(r.python3 && r.perl && r.diff);
    }

    #[test]
    fn parse_caps_only_python3() {
        let r = parse_capabilities("py3=1 perl=0 diff=0\n");
        assert!(r.python3 && !r.perl && !r.diff);
    }

    #[test]
    fn parse_caps_only_perl() {
        // 只有 perl —— match_file 能跑（不需要 diff），patch_file 在 handle 层另外校验 caps.diff
        let r = parse_capabilities("py3=0 perl=1 diff=0\n");
        assert!(!r.python3 && r.perl && !r.diff);
        assert_eq!(select_interpreter(r), Some(Interpreter::Perl));
    }

    #[test]
    fn parse_caps_none() {
        let r = parse_capabilities("py3=0 perl=0 diff=0\n");
        assert!(!r.python3 && !r.perl && !r.diff);
        assert_eq!(select_interpreter(r), None);
    }

    #[test]
    fn parse_caps_tolerates_pty_noise() {
        let out = "user@host:~$ echo \"py3=...\"\npy3=1 perl=1 diff=1\nuser@host:~$ \n";
        let r = parse_capabilities(out);
        assert!(r.python3 && r.perl && r.diff);
    }

    #[test]
    fn parse_caps_missing_field_defaults_false() {
        // 缺一个字段 —— 整行不匹配，全部 false
        let r = parse_capabilities("py3=1 perl=1");
        assert!(!r.python3 && !r.perl && !r.diff);
    }

    #[test]
    fn parse_caps_non_one_treated_as_false() {
        let r = parse_capabilities("py3=yes perl=2 diff=true");
        assert!(!r.python3 && !r.perl && !r.diff);
    }

    #[test]
    fn parse_caps_empty_input() {
        let r = parse_capabilities("");
        assert!(!r.python3 && !r.perl && !r.diff);
    }

    // ─── select_interpreter ─────────────────────────────────────────

    #[test]
    fn select_interp_python3_wins() {
        // python3 优先于 perl
        let caps = RemoteCapabilities { python3: true, perl: true, diff: true };
        assert_eq!(select_interpreter(caps), Some(Interpreter::Python3));
    }

    #[test]
    fn select_interp_python3_alone() {
        let caps = RemoteCapabilities { python3: true, perl: false, diff: false };
        assert_eq!(select_interpreter(caps), Some(Interpreter::Python3));
    }

    #[test]
    fn select_interp_perl_alone_ok_for_match() {
        // 没有 python3，perl 单独足够给 match_file。patch_file 在 handle 层另查 caps.diff。
        let caps = RemoteCapabilities { python3: false, perl: true, diff: false };
        assert_eq!(select_interpreter(caps), Some(Interpreter::Perl));
    }

    #[test]
    fn select_interp_none_without_interp() {
        // 有 diff 但没 python3 / perl —— file_ops 整体不可用
        let caps = RemoteCapabilities { python3: false, perl: false, diff: true };
        assert_eq!(select_interpreter(caps), None);
    }

    // ─── extract_json_payload ───────────────────────────────────────

    #[test]
    fn extract_json_basic() {
        let pty = "shell echo\n__RSSH_JSON__\n{\"count\":3,\"matches\":[]}\n__RSSH_JSON__\nsentinel:0\n";
        assert_eq!(
            extract_json_payload(pty),
            Some("{\"count\":3,\"matches\":[]}")
        );
    }

    #[test]
    fn extract_json_with_pty_noise() {
        let pty = "\x1b[?2004l\rprefix\n__RSSH_JSON__\n{\"a\":1}\n__RSSH_JSON__\r\nsentinel\n";
        assert_eq!(extract_json_payload(pty), Some("{\"a\":1}"));
    }

    #[test]
    fn extract_json_missing_returns_none() {
        assert_eq!(extract_json_payload("no markers here"), None);
        assert_eq!(extract_json_payload("__RSSH_JSON__\nonly one marker\n"), None);
    }

    #[test]
    fn extract_json_ignores_marker_literal_in_echoed_script() {
        // Regression: shell ECHO 会把脚本源码一并回显，脚本里 `M = "__RSSH_JSON__"`
        // 那行会作为 echo 的一部分进入 buffer。此时 buffer 含 3 个 marker：
        // 1 个 echo 字面量 + 2 个脚本真正输出。必须抽脚本输出的那对。
        let pty = "\
python3 -c 'import sys\n\
M = \"__RSSH_JSON__\"\n\
print(M); print(\"payload\"); print(M)'\n\
__RSSH_JSON__\n\
{\"count\":5,\"matches\":[{\"line\":84}]}\n\
__RSSH_JSON__\n\
__rssh_done_abc:0\n";
        assert_eq!(
            extract_json_payload(pty),
            Some("{\"count\":5,\"matches\":[{\"line\":84}]}")
        );
    }

    #[test]
    fn extract_json_ignores_marker_literal_inside_json_payload() {
        // Regression：之前用 rfind 子串。如果用户文件里写了 `__RSSH_JSON__` 注释，
        // match_file 把 context 字段塞进 JSON 输出 —— rfind 会扫到 JSON 内部的 marker 子串，
        // 在 JSON 中间切，破解析。按"独立成行"匹配杜绝这个：marker 子串嵌在引号 + 上下文里
        // 时永远不会独占一行。
        let pty = "\
__RSSH_JSON__\n\
{\"count\":1,\"matches\":[{\"line\":3,\"context\":\"# note: __RSSH_JSON__ is rssh internal\"}]}\n\
__RSSH_JSON__\n\
__rssh_done_xyz:0\n";
        assert_eq!(
            extract_json_payload(pty),
            Some("{\"count\":1,\"matches\":[{\"line\":3,\"context\":\"# note: __RSSH_JSON__ is rssh internal\"}]}")
        );
    }

    #[test]
    fn extract_json_takes_last_pair_when_buffer_contains_multiple_runs() {
        // 防御性：PTY buffer 累积多次工具调用的输出时（实际不会，每次 finish 都清，
        // 但万一上游误传整段 session log），取最后一对仍然给出最新的 JSON。
        let pty = "\
__RSSH_JSON__\n{\"call\":\"first\"}\n__RSSH_JSON__\n\
some interleaved noise\n\
__RSSH_JSON__\n{\"call\":\"second\"}\n__RSSH_JSON__\n";
        assert_eq!(extract_json_payload(pty), Some("{\"call\":\"second\"}"));
    }

    // ─── ansi_c_quote ───────────────────────────────────────────────

    #[test]
    fn ansi_c_quote_empty() {
        assert_eq!(ansi_c_quote(""), "$''");
    }

    #[test]
    fn ansi_c_quote_plain_ascii() {
        // 纯 ASCII 无元字符 —— 整段原样塞进 $'...'
        assert_eq!(ansi_c_quote("hello world"), "$'hello world'");
    }

    #[test]
    fn ansi_c_quote_newline_becomes_escape() {
        // 关键不变量：真换行 → `\n` 两字符序列，shell 拿到的命令是单行 ASCII，
        // 但 shell 展开后传给程序的 argv 含真实换行。
        assert_eq!(ansi_c_quote("a\nb"), "$'a\\nb'");
        assert_eq!(ansi_c_quote("line1\nline2\nline3"), "$'line1\\nline2\\nline3'");
    }

    #[test]
    fn ansi_c_quote_single_quote_uses_hex_escape() {
        // 单引号走 `\x27` 而非 `\'`。
        // 历史教训：含多个 `\'` 的长命令在 zsh + p10k 下偶发卡死（同样命令
        // 不含 `\'` 时稳定过）。改 hex 后 ZLE 状态机不再有"backslash + 字面
        // 单引号"和"close-quote"共用 transition 的风险。两者经 shell 解码后
        // 等价（都是 byte 0x27）。
        assert_eq!(ansi_c_quote("it's"), r"$'it\x27s'");
        assert_eq!(ansi_c_quote("'"), r"$'\x27'");
        // 关键不变量：输出绝不能含 `\'` 序列
        let q = ansi_c_quote("a'b'c'd");
        assert!(!q.contains(r"\'"), "must not produce \\' (hardening): {q}");
    }

    #[test]
    fn ansi_c_quote_backslash_doubles() {
        // 反斜杠在 $'...' 内是转义引导，必须 `\\`
        assert_eq!(ansi_c_quote(r"a\b"), r"$'a\\b'");
    }

    #[test]
    fn ansi_c_quote_tab_and_cr() {
        assert_eq!(ansi_c_quote("a\tb"), "$'a\\tb'");
        assert_eq!(ansi_c_quote("a\rb"), "$'a\\rb'");
    }

    #[test]
    fn ansi_c_quote_hex_escapes_c0_controls() {
        // 安全 hardening：ESC (0x1B) / BEL (0x07) / 其他 C0 控制字符 + DEL 必须 hex 化，
        // 防止 LLM 提供含 ESC sequence 的 find/replace 通过 PTY echo 触发 terminal injection。
        assert_eq!(ansi_c_quote("\x1b[31mRED"), "$'\\x1b[31mRED'");
        assert_eq!(ansi_c_quote("\x07bell"), "$'\\x07bell'");
        assert_eq!(ansi_c_quote("\x00NUL"), "$'\\x00NUL'");
        assert_eq!(ansi_c_quote("\x7fDEL"), "$'\\x7fDEL'");
        // 已经特殊处理的 \n / \r / \t 不被这条规则覆盖（保持原有 \n / \r / \t）
        assert_eq!(ansi_c_quote("a\nb"), "$'a\\nb'");
        // 综合：CSI 序列里夹换行
        assert_eq!(
            ansi_c_quote("\x1b]0;evil title\x07\nok"),
            "$'\\x1b]0;evil title\\x07\\nok'"
        );
    }

    #[test]
    fn ansi_c_quote_multibyte_passthrough() {
        // 非 ASCII（中文 / emoji）原样塞进 $'...'，shell 当字节透传给 argv
        assert_eq!(ansi_c_quote("中文"), "$'中文'");
        assert_eq!(ansi_c_quote("🦀"), "$'🦀'");
    }

    #[test]
    fn ansi_c_quote_shell_metachars_inert() {
        // $ ` 等在 $'...' 内不展开（跟 "$..." 不同）
        assert_eq!(ansi_c_quote("$HOME"), "$'$HOME'");
        assert_eq!(ansi_c_quote("`whoami`"), "$'`whoami`'");
    }

    #[test]
    fn ansi_c_quote_script_stays_single_line_ascii() {
        // 真实脚本里全是真换行 —— ansi_c_quote 输出必须不含真换行字节，
        // 否则 zsh ZLE 在 PTY 流里看到 `\n` 进 multi-line quote race。
        let quoted = ansi_c_quote(PYTHON_MATCH_SCRIPT);
        assert!(!quoted.contains('\n'), "ansi_c_quote leaked real newline");
        assert!(!quoted.contains('\r'));
        // 也确认开头 / 结尾 marker 存在
        assert!(quoted.starts_with("$'"));
        assert!(quoted.ends_with('\''));
    }

    #[test]
    fn ansi_c_quote_preserves_leading_spaces() {
        // 回归保护：patch_file 的 replace 经常是 YAML / config 块，缩进 2/4/6 空格混合。
        // ansi_c_quote 透传空格 byte，不能压缩 / 折叠，否则 shell 解析回来给 Python argv
        // 的字符串缩进塌掉，patch 写出来的文件就乱了。
        //
        // 历史误诊场景：发现 prometheus.yml 改完缩进塌成 2 空格，第一反应怀疑这里。
        // 实际是 LLM 在 tool input 里就给了塌的缩进 —— rssh 链路本身无损。本测试钉死
        // 这个不变量，下次遇到类似怀疑直接看这测试是否还过。
        let yaml = "  - job_name: \"x\"\n    scrape_interval: 1s\n      - targets: [\"h:80\"]";
        let q = ansi_c_quote(yaml);
        // 空格 byte 个数：ansi_c_quote 只改 \n/\r/\t/'/\\，空格透传 —— 出入必须相等
        let orig = yaml.bytes().filter(|&b| b == b' ').count();
        let got = q.bytes().filter(|&b| b == b' ').count();
        assert_eq!(orig, got, "spaces lost: {orig} → {got}, quoted={q}");
        // 行首空格也得在原位（每行的字面前缀应原样保留，不与 \n 混淆）
        assert!(q.contains("$'  - job_name:"), "L1 leading 2 spaces dropped: {q}");
        assert!(q.contains("\\n    scrape_interval"), "L2 leading 4 spaces dropped: {q}");
        assert!(q.contains("\\n      - targets"), "L3 leading 6 spaces dropped: {q}");
    }

    // ─── build_match_cmd / build_modify_cmd / build_cp_cmd / build_diff_cmd / build_mv_cmd ───

    #[test]
    fn build_match_cmd_python3_form() {
        let cmd = build_match_cmd(Interpreter::Python3, "/etc/foo.yml", "old", 80, 80);
        // 形态：python3 -c $'<script>' '/etc/foo.yml' $'old' '80' '80'
        assert!(cmd.starts_with("python3 -c $'"), "got: {cmd}");
        // path 走 shell_quote / ~/ 展开
        assert!(cmd.contains("'/etc/foo.yml'"));
        // find 走 ansi_c_quote："old" → $'old'
        assert!(cmd.contains("$'old'"));
        // before / after 走 shell_quote
        assert!(cmd.contains("'80'"));
        // 整条命令必须单行（避免 PTY ZLE quote race）
        assert!(!cmd.contains('\n'), "cmd has real newline: {cmd}");
    }

    #[test]
    fn build_match_cmd_perl_form() {
        let cmd = build_match_cmd(Interpreter::Perl, "/etc/foo.yml", "old", 80, 80);
        assert!(cmd.starts_with("perl -e $'"), "got: {cmd}");
        assert!(!cmd.contains('\n'));
    }

    #[test]
    fn build_match_cmd_python3_omits_arg_separator() {
        // Regression: Python `-c` 不消耗 `--`，加上去会让 sys.argv[1] 变成字面 '--'，
        // path 错位为 argv[2]。表现为 "io_error: invalid sys.argv".
        let cmd = build_match_cmd(Interpreter::Python3, "/etc/foo.yml", "old", 80, 80);
        assert!(
            !cmd.contains(" -- '/etc/foo.yml'"),
            "Python 路径 path 前不应有 `--`：{cmd}"
        );
    }

    #[test]
    fn build_match_cmd_perl_keeps_arg_separator() {
        // Perl `-e` 之后 @ARGV 首元素若以 `-` 开头会被当未知 switch（"Unrecognized switch"），
        // 必须用 `--` 强制结束 switch 解析。
        let cmd = build_match_cmd(Interpreter::Perl, "/etc/foo.yml", "old", 80, 80);
        assert!(
            cmd.contains(" -- '/etc/foo.yml'"),
            "Perl 脚本 + path 之间必须有 `--`：{cmd}"
        );
    }

    #[test]
    fn build_match_cmd_tilde_path_expands() {
        let cmd = build_match_cmd(Interpreter::Python3, "~/foo.yml", "x", 10, 10);
        // ~/ 展开为 "$HOME"/，rest 部分单引号包
        assert!(cmd.contains("\"$HOME\"/'foo.yml'"));
    }

    #[test]
    fn build_match_cmd_find_with_newline_stays_single_line() {
        // 真实场景：LLM 提供含换行的 find（如多行 YAML 块）。命令仍须单行 ASCII。
        let cmd = build_match_cmd(Interpreter::Python3, "/p", "line1\nline2", 0, 0);
        assert!(!cmd.contains('\n'), "newline leaked into cmd: {cmd}");
        // find 应当作 ANSI-C escape：$'line1\nline2'
        assert!(cmd.contains(r"$'line1\nline2'"));
    }

    #[test]
    fn build_match_cmd_find_with_shell_metachars_no_expansion() {
        // find 含 $VAR / 反引号 —— $'...' 内不展开
        let cmd = build_match_cmd(Interpreter::Python3, "/p", "$HOME `whoami`", 0, 0);
        assert!(cmd.contains("$'$HOME `whoami`'"));
    }

    #[test]
    fn build_modify_cmd_python3_form() {
        let cmd = build_modify_cmd(Interpreter::Python3, "/p.tmp", "old", "new", 3);
        // 形态：python3 -c $'<script>' '/p.tmp' $'old' $'new' '3'
        assert!(cmd.starts_with("python3 -c $'"), "got: {cmd}");
        assert!(cmd.contains("'/p.tmp'"));
        assert!(cmd.contains("$'old'"));
        assert!(cmd.contains("$'new'"));
        assert!(cmd.contains("'3'"));
        assert!(!cmd.contains('\n'));
    }

    #[test]
    fn build_modify_cmd_perl_form() {
        let cmd = build_modify_cmd(Interpreter::Perl, "/p.tmp", "old", "new", 1);
        assert!(cmd.starts_with("perl -e $'"), "got: {cmd}");
        // Perl 路径必须 `--` 分隔
        assert!(cmd.contains(" -- '/p.tmp'"), "got: {cmd}");
        assert!(!cmd.contains('\n'));
    }

    #[test]
    fn build_modify_cmd_handles_replace_with_newline() {
        // 多行 replace（真实场景：YAML block 替换）—— 不进 ZLE quote race
        let cmd = build_modify_cmd(Interpreter::Python3, "/p.tmp", "k: 1", "k: 2\nk2: 3", 1);
        assert!(!cmd.contains('\n'), "newline leaked: {cmd}");
        assert!(cmd.contains(r"$'k: 2\nk2: 3'"));
    }

    #[test]
    fn build_modify_cmd_handles_special_chars_in_path() {
        let cmd = build_modify_cmd(Interpreter::Python3, "/has space/it's.tmp", "x", "y", 1);
        assert!(cmd.contains(r"'/has space/it'\''s.tmp'"));
    }

    #[test]
    fn build_cp_cmd_form() {
        let cmd = build_cp_cmd("/p", "/p.rssh-abc12345");
        assert_eq!(cmd, r"\cp -- '/p' '/p.rssh-abc12345'");
    }

    #[test]
    fn build_cp_cmd_with_tilde() {
        let cmd = build_cp_cmd("~/foo", "~/foo.tmp");
        assert_eq!(cmd, "\\cp -- \"$HOME\"/'foo' \"$HOME\"/'foo.tmp'");
    }

    #[test]
    fn build_cp_cmd_with_special_chars() {
        let cmd = build_cp_cmd("/has space/it's", "/has space/it's.tmp");
        assert_eq!(cmd, r"\cp -- '/has space/it'\''s' '/has space/it'\''s.tmp'");
    }

    #[test]
    fn build_diff_cmd_form() {
        let cmd = build_diff_cmd("/p", "/p.tmp");
        assert_eq!(cmd, r"\diff -u -- '/p' '/p.tmp'");
    }

    #[test]
    fn build_diff_cmd_with_tilde() {
        let cmd = build_diff_cmd("~/foo", "~/foo.tmp");
        assert_eq!(cmd, "\\diff -u -- \"$HOME\"/'foo' \"$HOME\"/'foo.tmp'");
    }

    #[test]
    fn build_mv_cmd_form() {
        let cmd = build_mv_cmd("/p.rssh-abc12345", "/p");
        assert_eq!(cmd, r"\mv -- '/p.rssh-abc12345' '/p'");
    }

    #[test]
    fn build_mv_cmd_with_tilde() {
        let cmd = build_mv_cmd("~/foo.tmp", "~/foo");
        assert_eq!(cmd, "\\mv -- \"$HOME\"/'foo.tmp' \"$HOME\"/'foo'");
    }
}
