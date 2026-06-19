import type { Terminal } from "@xterm/xterm";

/**
 * Mobile touch-scroll for an xterm terminal. xterm 6.0.0 vendors a VS Code
 * touch-gesture service (with inertia!) but never calls addTarget(), so
 * touch-drag is wired to nothing — desktop has the wheel, touch had no scroll
 * path at all. This adds drag-to-scroll plus the fling momentum that every
 * native scroll surface has.
 *
 * `accumulateScroll` is the pure, unit-tested core (px travel → whole lines),
 * shared by both the live drag and the inertia frames. `setupTouchScroll` is
 * the DOM glue, kept here so both terminal hosts (TerminalPane, PlaybackScreen)
 * share one implementation.
 */

/**
 * Convert accumulated travel (px) into whole terminal lines, carrying the
 * sub-line remainder so motion stays 1:1 and never loses precision over a long
 * swipe — or across the drag→fling handoff (both feed the same remainder).
 *
 * trunc (toward zero), not floor: it keeps the remainder's sign matching the
 * travel, so up and down behave symmetrically — no special case per direction.
 */
export function accumulateScroll(
  remainder: number,
  deltaPx: number,
  rowHeight: number,
): { lines: number; remainder: number } {
  if (rowHeight <= 0) return { lines: 0, remainder };
  const total = remainder + deltaPx;
  const lines = Math.trunc(total / rowHeight);
  return { lines, remainder: total - lines * rowHeight };
}

// Tunables (device-feel; adjust on real hardware). Velocity is px/ms.
// Keep TAKEOVER_PX == the soft-keyboard handler's moveSlopPx (12, in TerminalPane)
// AND match its boundary: we claim only when travel EXCEEDS it (the check below uses
// `<=`), exactly when that handler flips to "moved" (hypot > slop). Otherwise a small
// drag could scroll yet still pop the keyboard on release.
const TAKEOVER_PX = 12;   // claim the gesture as a scroll once travel exceeds this
const FLING_MIN_V = 0.12; // release faster than this (~120 px/s) starts a fling
const STOP_V = 0.02;      // fling ends once it decays below this (~20 px/s)
const FRICTION = 0.94;    // per-60fps-frame velocity decay (frame-rate normalized)
const PAUSE_MS = 60;      // finger paused longer than this before release → no fling

/**
 * Wire one-finger vertical drag → scrollback on `host` (the element passed to
 * terminal.open()), with fling momentum on release. Returns a cleanup fn.
 *
 * Takes over only after the finger travels past a threshold, so a stationary
 * tap (focus / soft keyboard) and a stationary long-press (native text
 * selection) still pass through untouched; only a real drag scrolls. A new
 * touch cancels any in-flight fling (grab-to-stop, like native lists).
 * Caller decides when to install it (e.g. mobile only).
 */
export function setupTouchScroll(host: HTMLElement, terminal: Terminal): () => void {
  let startY = 0;
  let lastY = 0;
  let remainder = 0;   // sub-row px carried across moves AND into the fling
  let rowH = 0;        // px, sampled once per gesture (see measureRowHeight)
  let active = false;  // gesture claimed as a scroll (finger down)
  let ignore = false;  // gesture disqualified (multi-touch) until all fingers lift
  let velocity = 0;    // px/ms, recent-biased; sign = drag direction (dy)
  let lastMoveTime = 0;
  let inertiaRaf = 0;  // rAF handle, 0 = no fling running (invariant I1)

  // Row height is constant within a gesture (font can't change mid-drag), so sample
  // it ONCE at takeover, not per frame: reading offsetHeight while xterm is repainting
  // rows forces a synchronous layout reflow on every touchmove/fling frame (layout
  // thrash). Cached in rowH.
  function measureRowHeight(): number {
    const row = host.querySelector(".xterm-rows")?.firstElementChild as HTMLElement | null;
    return row?.offsetHeight ?? 0;
  }

  // Finger down (dy>0) = reveal earlier output = scroll up → negate.
  function scrollByPx(px: number) {
    const r = accumulateScroll(remainder, px, rowH);
    remainder = r.remainder;
    if (r.lines !== 0) terminal.scrollLines(-r.lines);
  }

  function cancelInertia() {
    if (inertiaRaf) {
      cancelAnimationFrame(inertiaRaf);
      inertiaRaf = 0;
    }
  }

  function inertiaStep(now: number) {
    const dt = Math.max(now - lastMoveTime, 1);
    lastMoveTime = now;
    velocity *= Math.pow(FRICTION, dt / 16.6667); // frame-rate independent decay
    if (Math.abs(velocity) < STOP_V) { inertiaRaf = 0; return; }
    scrollByPx(velocity * dt);
    inertiaRaf = requestAnimationFrame(inertiaStep);
  }

  function onTouchStart(e: TouchEvent) {
    cancelInertia();          // a new touch grabs and stops the glide (I2)
    active = false;
    velocity = 0;
    // Only a clean single-finger start tracks. A finger added mid-gesture (pinch)
    // disqualifies the gesture until ALL fingers lift and a fresh single touch
    // begins — never scroll from stale coordinates.
    ignore = e.touches.length !== 1;
    if (ignore) return;
    startY = lastY = e.touches[0].clientY;
    lastMoveTime = performance.now();
    remainder = 0;
  }

  function onTouchMove(e: TouchEvent) {
    if (ignore) return;
    if (e.touches.length !== 1) {  // a second finger joined → abandon this scroll
      ignore = true;
      active = false;
      velocity = 0;
      return;
    }
    const y = e.touches[0].clientY;
    if (!active) {
      if (Math.abs(y - startY) <= TAKEOVER_PX) return; // dead zone: tap / long-press
      active = true; // claim it; lastY & lastMoveTime stay at the touch start, so THIS
                     // frame scrolls the full travel and times velocity honestly
      rowH = measureRowHeight(); // sample once; reused for this drag + its fling
    }
    // Claimed: block native selection/scroll so it can't fight us.
    e.preventDefault();
    const now = performance.now();
    const dy = y - lastY;
    const dt = Math.max(now - lastMoveTime, 1);
    velocity = velocity * 0.2 + (dy / dt) * 0.8; // EMA, recent-biased for fling
    lastMoveTime = now;
    lastY = y;
    scrollByPx(dy);
  }

  function onTouchEnd(e: TouchEvent) {
    if (!active) return;      // wasn't a scroll drag → nothing to fling
    active = false;
    // Other fingers still down (e.g. a 2nd finger that landed off-host, so we never
    // saw its touchstart) → not a clean single-finger release, don't fling.
    if (e.touches.length > 0) return;
    // Paused before lifting, or barely moving → no fling (native behavior).
    if (performance.now() - lastMoveTime > PAUSE_MS) return;
    if (Math.abs(velocity) < FLING_MIN_V) return;
    lastMoveTime = performance.now();
    inertiaRaf = requestAnimationFrame(inertiaStep);
  }

  // Interrupted gesture (system takeover, extra touch): stop, never fling.
  function onTouchCancel() {
    active = false;
    cancelInertia();
  }

  host.addEventListener("touchstart", onTouchStart, { passive: true });
  host.addEventListener("touchmove", onTouchMove, { passive: false });
  host.addEventListener("touchend", onTouchEnd, { passive: true });
  host.addEventListener("touchcancel", onTouchCancel, { passive: true });

  return () => {
    cancelInertia(); // must run before terminal.dispose() — callers order it so (I4)
    host.removeEventListener("touchstart", onTouchStart);
    host.removeEventListener("touchmove", onTouchMove);
    host.removeEventListener("touchend", onTouchEnd);
    host.removeEventListener("touchcancel", onTouchCancel);
  };
}
