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

/// The set of windows that move as one. "Move-together" is an equivalence
/// relation (reflexive, symmetric, transitive), so it's modeled as disjoint
/// GROUPS, not directed pairs — that erases the "who's the leader",
/// "leader window closed", and "A-B meets B-C" special cases.
///
/// `last` holds each window's last known OUTER position (physical px); a drag
/// delta is mirrored onto the dragged window's siblings.
///
/// `quiet_until` is the anti-feedback mechanism. A user drag and the OS moving a
/// window for us (placement, our own follow, an edge clamp, a `set_size` origin
/// shift) are indistinguishable except by origin — so after we move a window
/// programmatically we mark it "settling" for `SETTLE` and ignore its `Moved`
/// events until then, *however far the OS adjusted the landing*. A plain
/// zero-delta check is not enough: macOS does not place windows pixel-exact, so
/// a clamped follow reports a non-zero delta and ping-pongs across the group.
#[cfg(desktop)]
#[derive(Default)]
pub struct WindowGroups {
    of: std::collections::HashMap<String, u64>,
    members: std::collections::HashMap<u64, std::collections::HashSet<String>>,
    last: std::collections::HashMap<String, (i32, i32)>,
    quiet_until: std::collections::HashMap<String, std::time::Instant>,
    next_id: u64,
}

/// How long a window ignores its own `Moved` events after we move it. Long
/// enough to swallow the burst one placement emits (set_position, set_size
/// origin shift, show — a few frames), short enough that grabbing the partner
/// window right after still feels instant.
#[cfg(desktop)]
const SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(desktop)]
impl WindowGroups {
    /// Bind `newcomer` into `opener`'s group (creating one if `opener` is free),
    /// seeding both last-known positions. `newcomer` is always a fresh window.
    /// Both windows settle from `now`: the placement that just tiled them emits
    /// programmatic `Moved`s that must not be mistaken for drags.
    pub fn bind(
        &mut self,
        opener: &str,
        newcomer: &str,
        opener_pos: (i32, i32),
        newcomer_pos: (i32, i32),
        now: std::time::Instant,
    ) {
        let id = match self.of.get(opener) {
            Some(&id) => id,
            None => {
                let id = self.next_id;
                self.next_id += 1;
                self.members.entry(id).or_default().insert(opener.to_string());
                self.of.insert(opener.to_string(), id);
                id
            }
        };
        self.members.entry(id).or_default().insert(newcomer.to_string());
        self.of.insert(newcomer.to_string(), id);
        self.last.insert(opener.to_string(), opener_pos);
        self.last.insert(newcomer.to_string(), newcomer_pos);
        self.quiet_until.insert(opener.to_string(), now + SETTLE);
        self.quiet_until.insert(newcomer.to_string(), now + SETTLE);
    }

    /// Record that `label` moved to `new` at `now` and return the siblings that
    /// must follow, each with its new outer position. Empty if `label` is
    /// unbound, still settling from a programmatic move, or hasn't moved.
    pub fn moved(
        &mut self,
        label: &str,
        new: (i32, i32),
        now: std::time::Instant,
    ) -> Vec<(String, (i32, i32))> {
        let Some(&id) = self.of.get(label) else {
            return vec![];
        };
        let old = self.last.get(label).copied().unwrap_or(new);
        self.last.insert(label.to_string(), new);
        // Absorb the window's own programmatic moves (placement, the follow we
        // commanded, an OS clamp) — whatever the landing — until they settle.
        if self.quiet_until.get(label).is_some_and(|&t| now < t) {
            return vec![];
        }
        let delta = (new.0 - old.0, new.1 - old.1);
        if delta == (0, 0) {
            return vec![];
        }
        // Sorted for deterministic output (HashSet iteration order is not).
        let mut sibs: Vec<String> = self.members[&id]
            .iter()
            .filter(|l| l.as_str() != label)
            .cloned()
            .collect();
        sibs.sort();
        let mut out = Vec::with_capacity(sibs.len());
        for s in sibs {
            let lp = self.last.get(&s).copied().unwrap_or((0, 0));
            let target = (lp.0 + delta.0, lp.1 + delta.1);
            self.last.insert(s.clone(), target);
            // The follow we're about to command is ours: let it settle so its
            // (possibly OS-clamped) `Moved` isn't mirrored back here.
            self.quiet_until.insert(s.clone(), now + SETTLE);
            out.push((s, target));
        }
        out
    }

    /// Drop `label` (its window closed or was explicitly unbound). When a group
    /// falls to a single member it dissolves — one window can't be "bound".
    pub fn remove(&mut self, label: &str) {
        let Some(id) = self.of.remove(label) else {
            return;
        };
        self.last.remove(label);
        self.quiet_until.remove(label);
        let dissolve = {
            let set = self
                .members
                .get_mut(&id)
                .expect("group exists for a mapped label");
            set.remove(label);
            set.len() <= 1
        };
        if dissolve {
            if let Some(set) = self.members.remove(&id) {
                for m in set {
                    self.of.remove(&m);
                    self.last.remove(&m);
                    self.quiet_until.remove(&m);
                }
            }
        }
    }
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
            // Bind the two windows so dragging one drags the other (live),
            // seeding last-known positions from where we just placed them.
            {
                use tauri::Manager as _;
                app.state::<crate::state::AppState>()
                    .window_groups
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .bind(
                        window.label(),
                        &label,
                        (stays.x, stays.y),
                        (opened.x, opened.y),
                        std::time::Instant::now(),
                    );
            }
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
    use std::time::{Duration, Instant};

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

    // ---- WindowGroups: live position binding ----

    // A timestamp safely past a window's settle period (formation / follow) —
    // i.e. a genuine user drag, not placement noise.
    fn after_settle(base: Instant) -> Instant {
        base + SETTLE + Duration::from_millis(1)
    }

    #[test]
    fn move_mirrors_delta_to_sibling() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (100, 200), (700, 200), t0);
        // A genuine drag of A (past formation settle) shifts B by the same delta.
        assert_eq!(
            g.moved("A", (110, 205), after_settle(t0)),
            vec![("B".to_string(), (710, 205))]
        );
    }

    #[test]
    fn zero_delta_move_is_noop() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (100, 200), (700, 200), t0);
        let t1 = after_settle(t0);
        g.moved("A", (110, 205), t1); // B → (710, 205)
        // A re-reports the SAME position past every settle window: zero delta, no-op.
        assert!(g.moved("A", (110, 205), after_settle(t1)).is_empty());
    }

    #[test]
    fn move_unbound_window_is_noop() {
        let mut g = WindowGroups::default();
        assert!(g.moved("ghost", (10, 10), Instant::now()).is_empty());
    }

    #[test]
    fn three_windows_move_together() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (0, 0), (600, 0), t0);
        g.bind("B", "C", (600, 0), (1200, 0), t0); // C joins A/B's group via B
        let mut moves = g.moved("A", (5, 0), after_settle(t0));
        moves.sort();
        assert_eq!(
            moves,
            vec![("B".to_string(), (605, 0)), ("C".to_string(), (1205, 0))]
        );
    }

    #[test]
    fn either_window_drives_after_settle() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (0, 0), (600, 0), t0);
        // Dragging B (not A) past the settle window moves A — binding is symmetric.
        assert_eq!(
            g.moved("B", (650, 0), after_settle(t0)),
            vec![("A".to_string(), (50, 0))]
        );
    }

    #[test]
    fn formation_moves_do_not_propagate() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (0, 0), (600, 0), t0);
        // place()/set_size()/show() fire programmatic Moveds right after bind, at
        // OS-adjusted positions (≠ what we seeded). Within the settle window they
        // must be absorbed, or the two windows ping-pong at open (the flicker).
        assert!(g.moved("A", (3, 0), t0 + Duration::from_millis(10)).is_empty());
        assert!(g.moved("B", (605, 0), t0 + Duration::from_millis(20)).is_empty());
    }

    #[test]
    fn os_adjusted_landing_is_absorbed_not_propagated() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (0, 0), (600, 0), t0);
        let drag = after_settle(t0);
        // A drags; B is commanded to (610, 0) and starts settling.
        assert_eq!(g.moved("A", (10, 0), drag), vec![("B".to_string(), (610, 0))]);
        // The OS clamps B's landing to (615, 0) — its own programmatic move, not a
        // user drag. It MUST be absorbed, not mirrored back onto A (the feedback
        // loop behind the open-time flicker).
        let echo = g.moved("B", (615, 0), drag + Duration::from_millis(5));
        assert!(echo.is_empty(), "OS-adjusted landing leaked back to A: {:?}", echo);
    }

    #[test]
    fn closing_one_dissolves_a_pair() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (0, 0), (600, 0), t0);
        g.remove("B"); // B's window closed
        // A is a free window again: moving it touches nothing.
        assert!(g.moved("A", (50, 0), after_settle(t0)).is_empty());
    }

    #[test]
    fn closing_one_of_three_keeps_the_rest_bound() {
        let t0 = Instant::now();
        let mut g = WindowGroups::default();
        g.bind("A", "B", (0, 0), (600, 0), t0);
        g.bind("B", "C", (600, 0), (1200, 0), t0);
        g.remove("B"); // close the middle window
        // A and C remain one group (a group is a set, not a chain — no split).
        assert_eq!(
            g.moved("A", (5, 0), after_settle(t0)),
            vec![("C".to_string(), (1205, 0))]
        );
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
