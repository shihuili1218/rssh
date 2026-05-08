//! 交互式 IO + 致命错误退出。CLI 不接 i18n catalog —— die() 直接英文输出。

use std::io::{self, Write};

/// 致命错误：英文打印到 stderr 后 exit(1)。返回 `!` 可填进 `unwrap_or_else` 闭包。
pub fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

pub fn prompt(label: &str) -> String {
    eprint!("{}", label);
    io::stderr().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
    buf.trim().to_string()
}

pub fn prompt_default(label: &str, default: &str) -> String {
    eprint!("{} [{}]: ", label, default);
    io::stderr().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
    let val = buf.trim();
    if val.is_empty() {
        default.to_string()
    } else {
        val.to_string()
    }
}

pub fn prompt_optional(label: &str) -> Option<String> {
    let val = prompt(label);
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

/// 打印编号列表让用户选一项。`0` 或无效输入返回 `None`（跳过）。
pub fn menu_select<'a, T, F>(
    header: &str,
    label: &str,
    items: &'a [T],
    empty_hint: &str,
    fmt: F,
) -> Option<&'a T>
where
    F: Fn(&T) -> String,
{
    if items.is_empty() {
        if !empty_hint.is_empty() {
            println!("{}", empty_hint);
        }
        return None;
    }
    println!("{}", header);
    println!("  0 - none");
    for (i, item) in items.iter().enumerate() {
        println!("  {} - {}", i + 1, fmt(item));
    }
    let choice = prompt_default(&format!("{} #", label), "0");
    choice
        .parse::<usize>()
        .ok()
        .and_then(|n| if n == 0 { None } else { items.get(n - 1) })
}

pub fn read_password(label: &str) -> String {
    eprint!("{}", label);
    io::stderr().flush().unwrap();
    rpassword::read_password().unwrap_or_default()
}

/// 敏感字段（token / password）的 prompt：不 echo 当前值，避免被屏幕录制 /
/// 终端历史抓走。占位显示 `(stored)`；用户回车保留旧，输入新值则覆盖。
/// 输入本身走 rpassword 不回显字符。
pub fn prompt_secret_default(label: &str, current: &str) -> String {
    let placeholder = if current.is_empty() {
        "(none)"
    } else {
        "(stored, press Enter to keep)"
    };
    eprint!("{} [{}]: ", label, placeholder);
    io::stderr().flush().unwrap();
    let val = rpassword::read_password().unwrap_or_default();
    if val.is_empty() {
        current.to_string()
    } else {
        val
    }
}

pub fn read_multiline() -> String {
    let mut lines = Vec::new();
    loop {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).unwrap();
        if buf.trim().is_empty() {
            break;
        }
        lines.push(buf);
    }
    lines.concat().trim_end().to_string()
}

pub fn confirm(label: &str, default: bool) -> bool {
    let hint = if default { "Y/n" } else { "y/N" };
    eprint!("{} [{}]: ", label, hint);
    io::stderr().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
    let val = buf.trim().to_lowercase();
    if val.is_empty() {
        default
    } else {
        val == "y" || val == "yes"
    }
}

pub fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(128);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(128);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(128);
        (r, g, b)
    } else {
        (128, 128, 128)
    }
}
