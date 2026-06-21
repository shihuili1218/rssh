use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// One half of a window split, in physical pixels (outer geometry).
#[cfg(desktop)]
#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// Split `r` into `(stays, opened)` halves. `dir` names where the NEW window
/// goes: "left" | "right" | "up" | "down". The second half takes `total - half`,
/// so the two halves tile `r` exactly — no gap or overlap even on odd sizes.
#[cfg(desktop)]
fn split_rect(r: Rect, dir: &str) -> Option<(Rect, Rect)> {
    let hw = r.w / 2;
    let hh = r.h / 2;
    let left = Rect {
        x: r.x,
        y: r.y,
        w: hw,
        h: r.h,
    };
    let right = Rect {
        x: r.x + hw as i32,
        y: r.y,
        w: r.w - hw,
        h: r.h,
    };
    let top = Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: hh,
    };
    let bottom = Rect {
        x: r.x,
        y: r.y + hh as i32,
        w: r.w,
        h: r.h - hh,
    };
    match dir {
        // (window that stays, window that opens)
        "right" => Some((left, right)),
        "left" => Some((right, left)),
        "down" => Some((top, bottom)),
        "up" => Some((bottom, top)),
        _ => None,
    }
}

/// Read the caller window's outer rect plus its decoration delta (outer - inner),
/// then split it. Returns `(stays, opened, (deco_w, deco_h))` in physical pixels,
/// or `None` if the direction is unknown or the geometry can't be read.
#[cfg(desktop)]
fn compute_split(win: &tauri::WebviewWindow, dir: &str) -> Option<(Rect, Rect, (u32, u32))> {
    let pos = win.outer_position().ok()?;
    let outer = win.outer_size().ok()?;
    let inner = win.inner_size().ok()?;
    let deco = (
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    );
    let (stays, opened) = split_rect(
        Rect {
            x: pos.x,
            y: pos.y,
            w: outer.width,
            h: outer.height,
        },
        dir,
    )?;
    Some((stays, opened, deco))
}

/// Move + resize `win` so its OUTER box fills `r`. `set_position` sets the outer
/// top-left, but `set_size` sets the INNER size — so subtract the decoration
/// delta to land the outer box exactly on `r` (gap-free tiling).
#[cfg(desktop)]
fn place(win: &tauri::WebviewWindow, r: Rect, deco: (u32, u32)) -> AppResult<()> {
    use tauri::{PhysicalPosition, PhysicalSize};
    win.set_position(PhysicalPosition::new(r.x, r.y))
        .map_err(|e| {
            AppError::other(
                "window_place_failed",
                serde_json::json!({ "op": "pos", "err": e.to_string() }),
            )
        })?;
    win.set_size(PhysicalSize::new(
        r.w.saturating_sub(deco.0).max(1),
        r.h.saturating_sub(deco.1).max(1),
    ))
    .map_err(|e| {
        AppError::other(
            "window_place_failed",
            serde_json::json!({ "op": "size", "err": e.to_string() }),
        )
    })?;
    Ok(())
}

/// Open a new in-process Tauri window with a clone payload.
/// The new window boots the same frontend; `AppShell` reads
/// `window.__rssh_clone` on mount and auto-creates the cloned tab.
///
/// Windows share `AppState` (sessions, DB, PTY registry) via `Arc<Mutex<..>>`,
/// so spawning a new window is cheap and does not fork the backend.
///
/// `split` is `None` for a plain new window (OS-positioned, 1200×800 — the
/// original behavior). "left"/"right"/"up"/"down" tiles the CALLER window into
/// one half and opens the new window in the other half of the same screen.
///
/// MUST stay `async`: on Windows, `WebviewWindowBuilder::build()` deadlocks when
/// called from a synchronous command — WebView2 needs the main thread's message
/// loop to create the webview controller, but a sync command is itself running
/// on that thread and blocks it, so the new window opens but never renders
/// (blank, no UI). async commands run off the main event-loop thread, so the
/// build completes. macOS/Linux don't have this reentrancy, so it only bites on
/// Windows. See tauri `WebviewWindowBuilder` docs / wry#583.
#[tauri::command]
pub async fn open_tab_in_new_window(
    app: AppHandle,
    window: tauri::WebviewWindow,
    clone: String,
    split: Option<String>,
) -> AppResult<()> {
    #[cfg(not(desktop))]
    let _ = (&window, &split); // only consumed on desktop; silence mobile warnings

    // `clone` is a JSON string from the frontend; embed it as a JS string literal.
    // Frontend reads window.__rssh_clone as a string and JSON.parses it once.
    // Do NOT JSON.parse here — that would store an object, and the frontend's
    // JSON.parse(object) would coerce to "[object Object]" and throw.
    let json_literal = serde_json::to_string(&clone).map_err(|e| {
        AppError::other(
            "window_clone_encode_failed",
            serde_json::json!({ "err": e.to_string() }),
        )
    })?;
    let init_script = format!("window.__rssh_clone = {};", json_literal);

    let label = format!("rssh-{}", Uuid::new_v4().simple());
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("RSSH")
        .initialization_script(&init_script);

    // Desktop only: when a split direction is given and the caller geometry is
    // readable, build the new window hidden, tile both halves, then show it —
    // so it never flashes at the OS default position first. Any miss (unknown
    // direction / geometry unavailable) falls through to the plain window below.
    #[cfg(desktop)]
    if let Some(dir) = split.as_deref() {
        if let Some((stays, opened, deco)) = compute_split(&window, dir) {
            let new_win = builder.visible(false).build().map_err(|e| {
                AppError::other(
                    "window_open_failed",
                    serde_json::json!({ "err": e.to_string() }),
                )
            })?;
            // Best-effort tiling: a failed move/resize (e.g. a fullscreen window
            // that can't be repositioned) must not orphan the hidden window or
            // abort the whole action — opening the window is the contract,
            // perfect placement is the bonus. So we always show() regardless.
            place(&new_win, opened, deco).ok();
            place(&window, stays, deco).ok();
            new_win.show().map_err(|e| {
                AppError::other(
                    "window_open_failed",
                    serde_json::json!({ "op": "show", "err": e.to_string() }),
                )
            })?;
            return Ok(());
        }
    }

    builder.inner_size(1200.0, 800.0).build().map_err(|e| {
        AppError::other(
            "window_open_failed",
            serde_json::json!({ "err": e.to_string() }),
        )
    })?;
    Ok(())
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;

    // Odd width/height on purpose: proves the halves still tile with no gap.
    fn rect() -> Rect {
        Rect {
            x: 100,
            y: 200,
            w: 1201,
            h: 801,
        }
    }

    #[test]
    fn right_keeps_left_opens_right_gap_free() {
        let (stays, opened) = split_rect(rect(), "right").unwrap();
        assert_eq!((stays.x, stays.y, stays.h), (100, 200, 801));
        assert_eq!(opened.x, stays.x + stays.w as i32); // abut, no gap
        assert_eq!(stays.w + opened.w, 1201); // exact tile (600 + 601)
    }

    #[test]
    fn left_mirrors_right() {
        let (stays, opened) = split_rect(rect(), "left").unwrap();
        assert_eq!(opened.x, 100); // new window on the left
        assert_eq!(stays.x, opened.x + opened.w as i32);
        assert_eq!(stays.w + opened.w, 1201);
    }

    #[test]
    fn down_keeps_top_opens_bottom_gap_free() {
        let (stays, opened) = split_rect(rect(), "down").unwrap();
        assert_eq!((stays.x, stays.y, stays.w), (100, 200, 1201));
        assert_eq!(opened.y, stays.y + stays.h as i32);
        assert_eq!(stays.h + opened.h, 801); // 400 + 401
    }

    #[test]
    fn up_mirrors_down() {
        let (stays, opened) = split_rect(rect(), "up").unwrap();
        assert_eq!(opened.y, 200); // new window on top
        assert_eq!(stays.y, opened.y + opened.h as i32);
        assert_eq!(stays.h + opened.h, 801);
    }

    #[test]
    fn unknown_direction_is_none() {
        assert!(split_rect(rect(), "sideways").is_none());
    }
}

/// One `arboard::Clipboard` for the whole process, created lazily.
///
/// On X11 the clipboard is a *selection ownership* protocol, not a store: the
/// process that wrote the text must stay alive to serve other apps' (and our
/// own paste's) `SelectionRequest`s. arboard owns the CLIPBOARD selection only
/// while at least one `Clipboard` instance is alive; the last one to drop tears
/// down its X11 window and hands the data off to a clipboard manager on a
/// best-effort basis — a race it usually loses ("Clipboard was dropped very
/// quickly after writing"). Creating a fresh `Clipboard` per call therefore
/// relinquished the selection the instant the call returned, so the next paste
/// read an empty clipboard.
///
/// Keeping one instance alive for the process lifetime means we stay the
/// selection owner: reads short-circuit to local data and external pastes are
/// served, with no per-call teardown/handoff race. `Clipboard` is `Send + Sync`
/// on every desktop platform, so a `static` behind a `Mutex` is sound.
static CLIPBOARD: std::sync::OnceLock<std::sync::Mutex<Option<arboard::Clipboard>>> =
    std::sync::OnceLock::new();

/// Run `op` against the process-wide clipboard, creating it on first use.
fn with_clipboard<R>(
    op: &'static str,
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<R, arboard::Error>,
) -> AppResult<R> {
    let cell = CLIPBOARD.get_or_init(|| std::sync::Mutex::new(None));
    // A panic while holding the lock can't leave the clipboard in an unsafe
    // state, so recover from poisoning rather than failing the operation.
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| {
            AppError::other(
                "window_clipboard_failed",
                serde_json::json!({ "op": "init", "err": e.to_string() }),
            )
        })?);
    }
    let cb = guard.as_mut().expect("clipboard initialized above");
    f(cb).map_err(|e| {
        AppError::other(
            "window_clipboard_failed",
            serde_json::json!({ "op": op, "err": e.to_string() }),
        )
    })
}

/// Read the system clipboard as text.
/// Goes through Rust (arboard) to bypass WebKit's permission prompt on
/// externally-sourced clipboard content — `navigator.clipboard.readText()`
/// pops a dialog every time on macOS unless the content was written by the
/// same page in this session.
#[tauri::command]
pub fn clipboard_read() -> AppResult<String> {
    with_clipboard("read", |cb| cb.get_text())
}

/// Write text to the system clipboard.
/// Mirrors `clipboard_read`: goes through Rust (arboard) because in the
/// WKWebView `navigator.clipboard.writeText` is unreliable from a right-click
/// (contextmenu) / unfocused context — it silently rejects.
#[tauri::command]
pub fn clipboard_write(text: String) -> AppResult<()> {
    with_clipboard("write", |cb| cb.set_text(text))
}
