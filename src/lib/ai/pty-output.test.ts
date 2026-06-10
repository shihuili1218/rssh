import { describe, it, expect } from "vitest";
import { extractOutput, findSentinel, stripAnsi } from "./pty-output.ts";

describe("extractOutput", () => {
  it("strips first line (PTY echo of command)", () => {
    const buffer = "cmd; echo marker\nactual line 1\nactual line 2\n";
    expect(extractOutput(buffer)).toBe("actual line 1\nactual line 2");
  });

  it("strips ANSI CSI escape sequences", () => {
    const buffer = "$ cmd\n\x1b[31mred text\x1b[0m\n";
    expect(extractOutput(buffer)).toBe("red text");
  });

  it("preserves leading whitespace (indented output)", () => {
    const buffer = "$ cmd\n  indented\n  output\n";
    expect(extractOutput(buffer)).toBe("  indented\n  output");
  });

  it("handles empty buffer", () => {
    expect(extractOutput("")).toBe("");
  });

  it("handles buffer with no newline (echo not yet terminated)", () => {
    expect(extractOutput("partial cmd")).toBe("partial cmd");
  });

  // Serial path: dropEchoLine=false keeps line 1 — a bare device may not echo,
  // so the first line is often real output, not a command echo.
  it("keeps the first line when dropEchoLine=false (serial, no echo)", () => {
    const buffer = "real output line 1\nreal output line 2\n";
    expect(extractOutput(buffer, undefined, false)).toBe(
      "real output line 1\nreal output line 2",
    );
  });

  it("still strips ANSI + trims with dropEchoLine=false", () => {
    expect(extractOutput("\x1b[32mok\x1b[0m\r\n", undefined, false)).toBe("ok");
  });
});

describe("findSentinel", () => {
  const SID = "__rssh_done_X";

  it("returns null when marker absent", () => {
    expect(findSentinel("$ cmd\nrunning...\n", SID)).toBeNull();
  });

  it("ignores literal sentinel in echo line (`:$?` is not digits)", () => {
    // echo 行里 sentinel 后跟 `$?`（shell 未展开的字面量），不该误判命令完成
    const buffer = `$ curl x; echo "${SID}:$?"\n`;
    expect(findSentinel(buffer, SID)).toBeNull();
  });

  it("extracts output + exit code on normal multi-line output", () => {
    const buffer = `$ ls; echo "${SID}:$?"\nfile1\nfile2\n${SID}:0\n`;
    expect(findSentinel(buffer, SID)).toEqual({
      output: "file1\nfile2",
      exitCode: 0,
    });
  });

  // 回归：原 bug 是上层用 `lastIndexOf("\n", m.index)` 当 endIndex，
  // 输出无尾换行时整段 {json} 跟 marker 一起被切掉，AI 收到 echo 行（命令本身）。
  // 修复：endIndex = m.index（marker UUID 起点），让 firstNl 处理跳 echo。
  it("preserves output when marker glues to last output line (regression)", () => {
    const echo = `$ curl -s http://x | head -c 9; echo "${SID}:$?"`;
    const output = '{"status":"success","data":[1,2,3]}';
    const buffer = `${echo}\n${output}${SID}:0\n`;
    expect(findSentinel(buffer, SID)).toEqual({
      output,
      exitCode: 0,
    });
  });

  it("handles negative exit code (Ctrl+C → 130 / shell-specific signals)", () => {
    const buffer = `$ cmd; echo "${SID}:$?"\n${SID}:-1\n`;
    expect(findSentinel(buffer, SID)?.exitCode).toBe(-1);
  });

  it("escapes regex metacharacters in sentinel uuid", () => {
    // 真实 sentinel 是 hex uuid，本不含元字符；但 escape 是契约，给个 paranoid 测试
    const weird = "__rssh_done_a.b*c";
    const buffer = `$ echo "${weird}:$?"\nok\n${weird}:0\n`;
    expect(findSentinel(buffer, weird)?.output).toBe("ok");
  });
});

describe("stripAnsi", () => {
  it("removes CSI sequences", () => {
    expect(stripAnsi("\x1b[31mred\x1b[0m")).toBe("red");
  });

  it("removes OSC sequences (BEL-terminated)", () => {
    expect(stripAnsi("\x1b]0;title\x07rest")).toBe("rest");
  });

  it("removes carriage returns", () => {
    expect(stripAnsi("line1\r\nline2")).toBe("line1\nline2");
  });
});
