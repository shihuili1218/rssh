/**
 * 从 PTY 数据流抽出真正交给 LLM 的 output。
 *
 * 数据形态：rssh 把 `cmd; echo "<sentinel>:$?"` 粘到目标终端，shell 把整行回显
 * 一遍（PTY echo），然后跑命令产生输出，最后 `echo` 打出 marker。listener 看到的：
 *
 *   <cmd; echo "__rssh_done_X:$?">\n     <- echo 行（$? 是字面量，没展开）
 *   <实际输出>                            <- 可能不带尾换行
 *   __rssh_done_X:<exit_code>\n           <- marker 行
 *
 * 关键陷阱：当实际输出不带尾换行（curl/head 这类极常见），marker 跟最后一段输出
 * 粘在同一行。任何"找 marker 那一行的起点然后截到那里"的写法都会把真实输出一起切掉。
 * 正确的边界是 marker UUID 的起点本身（regex 的 m.index），不是 marker 所在行的起点。
 */

export const stripAnsi = (s: string) =>
  s.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")
   .replace(/\x1b\][^\x07]*\x07/g, "")
   .replace(/\r/g, "");

/**
 * 1. 截到 endIndex（sentinel 路径传 marker UUID 起点；terminate/timeout 传整段）
 * 2. strip ANSI / OSC / CR
 * 3. 跳掉首行（PTY echo 的命令本身）—— 仅 dropEchoLine=true 时
 * 4. trimEnd（保留前导空白，避免吃掉 `  indented output` 的对齐）
 *
 * `dropEchoLine` defaults to true (shell paths: the shell always echoes the
 * pasted command as line 1). Serial passes false: a bare device may NOT echo,
 * so dropping line 1 would silently eat real output. Keeping an echoed-command
 * line is harmless — the LLM is told it's reading the raw device response.
 */
export function extractOutput(
  rawBuffer: string,
  endIndex?: number,
  dropEchoLine = true,
): string {
  const end = Math.max(0, endIndex ?? rawBuffer.length);
  const stripped = stripAnsi(rawBuffer.substring(0, end));
  if (!dropEchoLine) return stripped.trimEnd();
  const firstNl = stripped.indexOf("\n");
  const out = firstNl >= 0 ? stripped.substring(firstNl + 1) : stripped;
  return out.trimEnd();
}

export interface SentinelMatch {
  output: string;
  exitCode: number;
}

/**
 * 在 PTY buffer 里找 `<sentinel>:(-?\d+)` —— 注意 echo 行里出现的字面量
 * `<sentinel>:$?` 不会匹配（`$?` 不是数字），所以总是命中真正 echo 出来的 marker。
 */
export function findSentinel(buffer: string, sentinelUuid: string): SentinelMatch | null {
  const re = new RegExp(
    sentinelUuid.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + ":(-?\\d+)"
  );
  const m = re.exec(buffer);
  if (!m) return null;
  return {
    output: extractOutput(buffer, m.index),
    exitCode: parseInt(m[1], 10),
  };
}
