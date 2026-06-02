use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::error::{locked, AppError, AppResult};

/// PTY output destined for the host. The Tauri command turns these into
/// `app.emit("pty:data:<id>")` / `pty:close:<id>`; the headless ws server
/// pushes them to its socket. `spawn` itself stays transport-agnostic.
pub enum PtyOut {
    Data(Vec<u8>),
    Close,
}

/// Sink the reader thread invokes for each chunk. The `&str` is the session
/// id, so one sink can serve any number of PTY sessions.
pub type PtySink = Arc<dyn Fn(&str, PtyOut) + Send + Sync>;

/// 子进程持有者：保证 PtyHandle 最后一份 clone 被 drop 时（tab 关闭 / session 结束），
/// 显式 kill + wait 子 shell。否则 Box<dyn Child> 在 spawn() 返回后立刻 drop，
/// 子进程退出后无人 reap，留 zombie 占 PID。
/// `Box<dyn Child + Send>` 不带 `Sync`：portable_pty 的 Child 实现普遍只是
/// Send。`Mutex<T>` 自身在 `T: Send` 时即是 Sync，无需 inner 也 Sync——
/// 加多余的 Sync bound 在某些平台上会编不过。
struct ChildReaper {
    child: Mutex<Option<Box<dyn Child + Send>>>,
}

impl Drop for ChildReaper {
    fn drop(&mut self) {
        // Drop 在 Arc 计数归零时跑一次。kill + wait 通常 < 100ms（SIGKILL → 内核 reap）。
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

/// 本地 PTY 会话句柄，Clone + Send + Sync。
/// `_reaper` 跟着 PtyHandle 走，最后一份 clone 消失时回收子进程。
/// `shell_path` 是 spawn 时实际使用的 shell 二进制路径——AI session 用它
/// 推断 ShellKind（无需探测，因为本地 shell 是用户在 UI 里显式选的）。
#[derive(Clone)]
pub struct PtyHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    shell_path: Arc<str>,
    _reaper: Arc<ChildReaper>,
}

impl PtyHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        locked(&self.writer)?
            .write_all(data)
            .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> AppResult<()> {
        locked(&self.master)?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))
    }

    /// spawn 时实际使用的 shell 路径（用户在 UI 选的，或 default_shell 兜底）。
    /// AI 模块用这个判定本地 PTY 的 ShellKind，无需探测。
    pub fn shell_path(&self) -> &str {
        &self.shell_path
    }
}

/// 启动本地 shell，返回 (session_id, handle)。
/// 读取线程通过 Tauri 事件 `pty:data:{id}` 推送数据。

/// 本机实际可用的 shell 路径列表。启动时扫描一次进缓存；
/// 用户在 Shell 设置页点"刷新"会重扫覆盖（用户 `brew install fish`
/// 之类的中途变化得有补救手段，否则要 restart app 才看得到）。
/// RwLock 比 OnceLock 多支持一个 write 路径 —— 读路径几乎没有竞争开销。
/// `Option<Vec>` 区分"未初始化"和"扫出来空"两种状态：未初始化时 lazy 扫一次。
static AVAILABLE_SHELLS: std::sync::RwLock<Option<Vec<String>>> =
    std::sync::RwLock::new(None);

/// 启动时由 lib.rs 调一次预热。重复调跟 refresh 一样语义。
pub fn init_available_shells() {
    refresh_available_shells();
}

/// 重新扫描并覆盖缓存。Shell 设置页"刷新"按钮 / 用户装新 shell 后调。
pub fn refresh_available_shells() {
    let scanned = scan_shells();
    if let Ok(mut g) = AVAILABLE_SHELLS.write() {
        *g = Some(scanned);
    }
}

pub fn available_shells() -> Vec<String> {
    if let Ok(g) = AVAILABLE_SHELLS.read() {
        if let Some(v) = g.as_ref() {
            return v.clone();
        }
    }
    // 还没初始化（lib.rs 没调到 init，或调用方不是桌面端）—— lazy 扫一次。
    let scanned = scan_shells();
    if let Ok(mut g) = AVAILABLE_SHELLS.write() {
        *g = Some(scanned.clone());
    }
    scanned
}

/// 真正的"shell 候选"判据：必须是普通文件 + Unix 上有执行位。
/// 比 `Path::exists()` 严：能挡掉 `/etc/shells` / PATH 里的目录、破损 symlink、
/// 纯数据文件等乱入，避免最后 spawn 报"not executable"。
#[cfg(unix)]
fn is_shell_candidate(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
        .unwrap_or(false)
}

/// Windows 没有 POSIX 执行位，靠扩展名 + is_file。我们 KNOWN 列表里全是
/// `.exe` 后缀，普通文件即可。
#[cfg(windows)]
fn is_shell_candidate(path: &std::path::Path) -> bool {
    path.is_file()
}

/// 按 canonical path 去重：保留首次出现的字符串路径。
/// canonicalize 失败时（不存在 / 权限 / NixOS store 之类）回退原路径，
/// 退化为字符串去重，不丢东西。
#[cfg(any(unix, windows))]
fn dedup_by_canonical(paths: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;
    use std::path::PathBuf;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| PathBuf::from(&p));
        if seen.insert(canon) {
            out.push(p);
        }
    }
    out
}

fn scan_shells() -> Vec<String> {
    #[cfg(unix)]
    {
        scan_unix()
    }
    #[cfg(windows)]
    {
        scan_windows()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Vec::new()
    }
}

#[cfg(unix)]
fn scan_unix() -> Vec<String> {
    use std::path::Path;
    use std::path::PathBuf;

    // 收集所有候选 —— /etc/shells 优先（系统级权威清单）、PATH 扫描补漏、
    // $SHELL 兜底。中间不去重，最后走 canonical 去重一遍。
    let mut candidates: Vec<String> = Vec::new();

    // 1) /etc/shells —— 系统级权威清单（chsh -a / 包管理装 shell 都会写这里）。
    //    每行可能带 `#` 注释 + 空行 + 不存在路径（清单陈旧），全过滤掉。
    if let Ok(content) = std::fs::read_to_string("/etc/shells") {
        for line in content.lines() {
            let s = line.split('#').next().unwrap_or("").trim();
            if !s.is_empty() && is_shell_candidate(Path::new(s)) {
                candidates.push(s.to_string());
            }
        }
    }

    // 2) 在 PATH 里 which 一组已知 shell 名，捞漏。覆盖 `/etc/shells` 没注册的：
    //    - 用户 `cargo install nu` 没 `chsh -a`
    //    - Homebrew 装 fish 在 `/opt/homebrew/bin/fish`、`/usr/local/bin/fish`
    //    - 类 Termux / NixOS 这种 `/etc/shells` 不完整或不存在的环境
    const KNOWN_UNIX: &[&str] = &[
        "bash", "zsh", "fish", "dash", "sh", "ksh", "tcsh", "csh",
        "nu", "xonsh", "elvish", "ion", "pwsh",
    ];
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':').filter(|d| !d.is_empty()) {
            for name in KNOWN_UNIX {
                let candidate = format!("{dir}/{name}");
                if is_shell_candidate(Path::new(&candidate)) {
                    candidates.push(candidate);
                }
            }
        }
    }

    // 3) $SHELL 兜底 —— 上面两步可能都漏了用户自己手编译塞到 ~/bin 的 shell。
    let preferred = std::env::var("SHELL").ok();
    if let Some(s) = preferred.as_ref() {
        if is_shell_candidate(Path::new(s)) {
            candidates.push(s.clone());
        }
    }

    // canonical 去重：macOS 上 /bin/bash 和 /usr/bin/bash 是同一个 inode，
    // 字符串去重会留两个；canonicalize 之后用真身路径作 set key，只留一个。
    let mut shells = dedup_by_canonical(candidates);
    shells.sort();

    // $SHELL 排第一（用户偏好）。可能用户的 $SHELL 是 /bin/bash 但 dedup 留下
    // 的是 /usr/bin/bash —— 走 canonical 匹配，避免字符串比对漏掉。
    if let Some(pref) = preferred {
        let pref_canon =
            std::fs::canonicalize(&pref).unwrap_or_else(|_| PathBuf::from(&pref));
        if let Some(idx) = shells.iter().position(|s| {
            std::fs::canonicalize(s).unwrap_or_else(|_| PathBuf::from(s)) == pref_canon
        }) {
            let head = shells.remove(idx);
            shells.insert(0, head);
        }
    }
    shells
}

#[cfg(windows)]
fn scan_windows() -> Vec<String> {
    use std::path::Path;

    let mut candidates: Vec<String> = Vec::new();

    // 1) 已知绝对路径 —— Windows 没有 /etc/shells 等价物，硬编码常见安装位置 + 验存在。
    //    SystemRoot 通常是 C:\Windows，但企业镜像可能改过，所以读环境变量而不写死。
    let system_root =
        std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let known: &[String] = &[
        format!("{system_root}\\System32\\cmd.exe"),
        format!("{system_root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
        format!("{system_root}\\System32\\wsl.exe"),
        "C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_string(),
        "C:\\Program Files\\Git\\bin\\bash.exe".to_string(),
        "C:\\Program Files\\Git\\usr\\bin\\bash.exe".to_string(),
    ];
    for c in known {
        if is_shell_candidate(Path::new(c)) {
            candidates.push(c.clone());
        }
    }

    // 2) PATH 扫已知名字 —— 捞 winget/scoop 装的 pwsh / nu / fish 等。
    const KNOWN_WIN: &[&str] = &[
        "pwsh.exe", "bash.exe", "nu.exe", "fish.exe", "elvish.exe", "xonsh.exe",
    ];
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(';').filter(|d| !d.is_empty()) {
            for name in KNOWN_WIN {
                let candidate = format!("{dir}\\{name}");
                if is_shell_candidate(Path::new(&candidate)) {
                    candidates.push(candidate);
                }
            }
        }
    }

    // canonical 去重 + 排序。Windows junction point 少，主要是吃掉 PATH 里
    // 重复目录导致的同一路径多次 push。
    let mut shells = dedup_by_canonical(candidates);
    shells.sort();
    shells
}

fn default_shell() -> String {
    // SHELL 仅 Unix 上可信：Windows 下 MSYS/Git Bash 常把 SHELL 设为
    // /usr/bin/bash 这种 Unix 路径，portable_pty 拿去 spawn 会直接失败。
    // Windows 走 available_shells() 的扫描结果（System32 / Program Files / PATH）。
    // 即便在 Unix，也得校验 SHELL 真有效（trim + is_shell_candidate）—— 空串、
    // 卸载残留的旧路径、user 手改坏的值都得过滤，避免拿垃圾路径去 spawn。
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(s) = std::env::var("SHELL") {
            let trimmed = s.trim();
            if !trimmed.is_empty() && is_shell_candidate(std::path::Path::new(trimmed)) {
                return trimmed.to_string();
            }
        }
        available_shells()
            .into_iter()
            .next()
            .unwrap_or_else(|| "/bin/sh".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        // Windows 没有 $SHELL 等价物 —— available_shells() 是字典序排好的，
        // 直接拿 first 会让 `C:\Program Files\Git\bin\bash.exe` 这种偏门项目
        // 在 `C:\Windows\System32\cmd.exe` 之前。显式按偏好（cmd > pwsh >
        // powershell）挑，挑不到再退到字典序首位。
        let shells = available_shells();
        const PREF_SUFFIXES: &[&str] = &["\\cmd.exe", "\\pwsh.exe", "\\powershell.exe"];
        for suf in PREF_SUFFIXES {
            if let Some(s) = shells.iter().find(|s| s.to_lowercase().ends_with(suf)) {
                return s.clone();
            }
        }
        shells.into_iter().next().unwrap_or_else(|| "cmd.exe".to_string())
    }
}

pub fn spawn(
    cols: u16,
    rows: u16,
    sink: PtySink,
    shell_override: Option<String>,
) -> AppResult<(String, PtyHandle)> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;

    let shell = shell_override
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_shell);

    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("RSSH_APP", "1");
    if !cfg!(target_os = "windows") {
        cmd.arg("-l");
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;

    let id = uuid::Uuid::new_v4().to_string();
    let handle = PtyHandle {
        writer: Arc::new(Mutex::new(writer)),
        master: Arc::new(Mutex::new(pair.master)),
        shell_path: Arc::from(shell.as_str()),
        _reaper: Arc::new(ChildReaper {
            child: Mutex::new(Some(child)),
        }),
    };

    // 读取线程：PTY stdout → Tauri 事件
    let pty_id = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut reader = reader;
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink(&pty_id, PtyOut::Data(buf[..n].to_vec())),
            }
        }
        sink(&pty_id, PtyOut::Close);
    });

    Ok((id, handle))
}
