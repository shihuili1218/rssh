import { describe, it, expect, vi, beforeEach } from "vitest";

// hoisted: 在 i18n 模块 import 之前注入 globals —— 模块顶层
// `let _locale = $state(detectLocale())` 立刻读 localStorage。
// Node 21+ 上 navigator 是 read-only global, 直接赋值会炸；用 vi.stubGlobal。
vi.hoisted(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
    clear: () => store.clear(),
    key: () => null,
    length: 0,
  });
  // 不 stub navigator —— Node 21+ 自带；detectLocale() 拿到的 default 取决
  // 于宿主系统 locale（en 或 zh），测试不钉死它，只钉 setLocale 行为。
});

import {
  t,
  errMsg,
  locale,
  setLocale,
  AVAILABLE_LOCALES,
} from "./index.svelte.ts";

beforeEach(() => {
  // 每个测试开始前回到 "en"，避免上一个测试 setLocale("zh") 留毒
  setLocale("en");
});

/* ─── errMsg ──────────────────────────────────────────────────── */

describe("errMsg", () => {
  it("returns null/undefined as empty string", () => {
    expect(errMsg(null)).toBe("");
    expect(errMsg(undefined)).toBe("");
  });

  it("returns plain strings as-is when no protocol prefix", () => {
    expect(errMsg("network down")).toBe("network down");
    expect(errMsg("")).toBe("");
  });

  it("unwraps Error instances and falls through if no prefix", () => {
    expect(errMsg(new Error("boom"))).toBe("boom");
  });

  it("translates a known protocol-coded error", () => {
    // error.ssh_session_not_found 是 en 字典里固定的 key
    const wire = `__rssh_err__|${JSON.stringify({
      code: "ssh_session_not_found",
      params: {},
    })}`;
    expect(errMsg(wire)).toBe("SSH session not found");
  });

  it("substitutes placeholders in a translated message", () => {
    // error.session_already_exists 含 {target}
    const wire = `__rssh_err__|${JSON.stringify({
      code: "session_already_exists",
      params: { target: "host:22" },
    })}`;
    expect(errMsg(wire)).toContain("host:22");
    expect(errMsg(wire)).not.toContain("{target}");
  });

  it("malformed JSON after prefix returns the original string", () => {
    const garbled = "__rssh_err__|not-json{";
    // 不抛、不错把它当合法协议——原样返回让用户至少看到点什么
    expect(errMsg(garbled)).toBe(garbled);
  });

  it("unknown code falls back to the key string", () => {
    // t() 三层 fallback 的最末层：key 原样返回
    const wire = `__rssh_err__|${JSON.stringify({
      code: "this_code_definitely_does_not_exist",
      params: {},
    })}`;
    expect(errMsg(wire)).toBe("error.this_code_definitely_does_not_exist");
  });

  it("missing params object still returns translation (no placeholders)", () => {
    // params 缺省时 t() 走"无替换"分支
    const wire = `__rssh_err__|${JSON.stringify({
      code: "ssh_session_not_found",
    })}`;
    expect(errMsg(wire)).toBe("SSH session not found");
  });
});

/* ─── end-to-end 协议测试（跨语言契约）─────────────────────────
 *
 * 下面 wire 字符串必须和 `src-tauri/src/error.rs` 的字节级测试
 * 钉住的字节**完全相同**——这是协议两端的真握手测试：
 *
 *   后端 Rust：AppError → CodedMsg::Display → wire bytes
 *   前端 TS:   wire bytes → errMsg() → 最终面向用户的字符串
 *
 * 哪一端改了序列化格式或翻译模板，这里都会红。
 * ───────────────────────────────────────────────────────────── */

describe("errMsg — end-to-end protocol with backend wire format", () => {
  it("profile_not_found → English user-facing string", () => {
    // wire 与 error.rs::tests::app_error_serialize_for_tauri_exact_value 同源
    const wire = `__rssh_err__|{"code":"profile_not_found","params":{"id":"abc"}}`;
    setLocale("en");
    expect(errMsg(wire)).toBe("Profile 'abc' not found");
  });

  it("profile_not_found → 中文 user-facing string", () => {
    const wire = `__rssh_err__|{"code":"profile_not_found","params":{"id":"abc"}}`;
    setLocale("zh");
    expect(errMsg(wire)).toBe("Profile 'abc' 不存在");
  });

  it("ssh_connect_timeout with multi placeholders", () => {
    // host/port/secs 三个占位都要替换
    const wire =
      `__rssh_err__|{"code":"ssh_connect_timeout","params":{"host":"h","port":22,"secs":10}}`;
    setLocale("en");
    expect(errMsg(wire)).toBe("h:22 connect timed out (10s)");
  });

  it("ssh_connect_failed forwards inner error text", () => {
    // wire 与 error.rs::tests::coded_msg_display_string_param_exact_value 同源
    const wire =
      `__rssh_err__|{"code":"ssh_connect_failed","params":{"err":"timeout"}}`;
    setLocale("en");
    expect(errMsg(wire)).toBe("Connect failed: timeout");
  });

  it("ssh_auth_rejected has no placeholders, output verbatim", () => {
    const wire = `__rssh_err__|{"code":"ssh_auth_rejected","params":{}}`;
    setLocale("en");
    expect(errMsg(wire)).toBe("Auth rejected");
  });

  it("lock_poisoned (Lock variant rendered via thiserror template)", () => {
    // wire 与 error.rs::tests::app_error_lock_exact_value 同源
    const wire = `__rssh_err__|{"code":"lock_poisoned","params":{}}`;
    setLocale("en");
    expect(errMsg(wire)).toBe("Lock poisoned (internal error)");
    setLocale("zh");
    expect(errMsg(wire)).toBe("锁中毒（内部错误）");
  });

  it("io_error from std::io::Error roundtrip", () => {
    // wire 与 error.rs::tests::from_io_error_exact_value 同源
    const wire = `__rssh_err__|{"code":"io_error","params":{"err":"boom"}}`;
    setLocale("en");
    expect(errMsg(wire)).toBe("IO error: boom");
    setLocale("zh");
    expect(errMsg(wire)).toBe("IO 错误：boom");
  });
});

/* ─── t (translation) ─────────────────────────────────────────── */

describe("t", () => {
  it("looks up a known key in the active locale", () => {
    expect(t("common.save" as any)).toBe("Save");
  });

  it("substitutes {placeholder} from params", () => {
    // error.session_already_exists = "Target {target} already has an AI session — stop it first"
    const out = t("error.session_already_exists" as any, { target: "h:22" });
    expect(out).toContain("Target h:22");
    expect(out).not.toContain("{target}");
  });

  it("leaves placeholder when param missing (escape hatch)", () => {
    const out = t("error.session_already_exists" as any, {});
    // 无 target 时 {target} 原样保留——便于排查
    expect(out).toContain("{target}");
  });

  it("converts numeric params to string", () => {
    // ai.audit.toggle.output 含 {bytes}
    const out = t("ai.audit.toggle.output" as any, { bytes: 1024 });
    expect(out).toContain("1024");
  });

  it("unknown key returns the key string itself (last-resort fallback)", () => {
    const out = t("totally.bogus.key" as any);
    expect(out).toBe("totally.bogus.key");
  });

  it("respects locale switch — same key returns localized strings", () => {
    expect(t("tab.home" as any)).toBe("Home");
    setLocale("zh");
    expect(locale()).toBe("zh");
    expect(t("tab.home" as any)).toBe("首页");
  });
});

/* ─── locale getter / setter ─────────────────────────────────── */

describe("locale + setLocale", () => {
  it("default locale is one of the supported codes", () => {
    // detectLocale 取决于宿主 navigator.language，不钉死值——只钉合法集合
    expect(["en", "zh"]).toContain(locale());
  });

  it("setLocale flips locale() reactively", () => {
    setLocale("zh");
    expect(locale()).toBe("zh");
    setLocale("en");
    expect(locale()).toBe("en");
  });

  it("setLocale persists to localStorage", () => {
    setLocale("zh");
    expect(localStorage.getItem("rssh.locale")).toBe("zh");
  });
});

/* ─── AVAILABLE_LOCALES metadata ─────────────────────────────── */

describe("AVAILABLE_LOCALES", () => {
  it("lists en + zh with display labels", () => {
    expect(AVAILABLE_LOCALES.length).toBe(2);
    const codes = AVAILABLE_LOCALES.map((l) => l.code);
    expect(codes).toContain("en");
    expect(codes).toContain("zh");
    for (const l of AVAILABLE_LOCALES) {
      expect(l.label).toBeTruthy();
    }
  });
});
