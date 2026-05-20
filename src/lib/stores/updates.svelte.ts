import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

const REPO = "shihuili1218/rssh";
const INITIAL_DELAY_MS = 10_000;
const INTERVAL_MS = 6 * 60 * 60 * 1000; // 6h

export type UpdateState =
  | { kind: "unknown" }
  | { kind: "checking" }
  | { kind: "latest" }
  | { kind: "outdated"; latest: string }
  | { kind: "error" };

let _state = $state<UpdateState>({ kind: "unknown" });
let _scheduled = false;
let _initialTimer: ReturnType<typeof setTimeout> | null = null;
let _intervalTimer: ReturnType<typeof setInterval> | null = null;

export function state(): UpdateState { return _state; }
export function hasUpdate(): boolean { return _state.kind === "outdated"; }
export function latestVersion(): string | null {
  return _state.kind === "outdated" ? _state.latest : null;
}

function parseVersion(v: string): number[] {
  return v.replace(/^v/i, "").split(/[.\-+]/).map(s => parseInt(s, 10) || 0);
}

function compareVersion(a: string, b: string): number {
  const aa = parseVersion(a);
  const bb = parseVersion(b);
  const len = Math.max(aa.length, bb.length);
  for (let i = 0; i < len; i++) {
    const x = aa[i] ?? 0;
    const y = bb[i] ?? 0;
    if (x !== y) return x > y ? 1 : -1;
  }
  return 0;
}

/** Single check pass. Re-entrant safe: returns immediately if already checking.
 *  `silent`: background callers pass true so a network failure doesn't flip
 *  the About screen into a visible error state. Manual button presses omit
 *  it and DO surface "error" — users expect feedback when they click. */
export async function runCheck(opts?: { silent?: boolean }): Promise<void> {
  if (_state.kind === "checking") return;
  const prev = _state;
  _state = { kind: "checking" };
  try {
    const [tag, current] = await Promise.all([
      invoke<string>("fetch_latest_release_tag", { repo: REPO }),
      getVersion(),
    ]);
    if (!tag) throw new Error("empty tag");
    _state = compareVersion(tag, current) > 0
      ? { kind: "outdated", latest: tag.replace(/^v/i, "") }
      : { kind: "latest" };
  } catch (e) {
    console.error("updates.runCheck failed:", e);
    _state = opts?.silent ? prev : { kind: "error" };
  }
}

/** Start background checks. Idempotent — calling twice is a no-op.
 *  10s initial delay (out of app startup hot path), then every 6h. */
export function startBackgroundChecks(): void {
  if (_scheduled) return;
  _scheduled = true;
  _initialTimer = setTimeout(() => {
    runCheck({ silent: true });
    _intervalTimer = setInterval(() => runCheck({ silent: true }), INTERVAL_MS);
  }, INITIAL_DELAY_MS);
}

/** Test/teardown helper — stops timers and resets the schedule flag. */
export function stopBackgroundChecks(): void {
  if (_initialTimer) { clearTimeout(_initialTimer); _initialTimer = null; }
  if (_intervalTimer) { clearInterval(_intervalTimer); _intervalTimer = null; }
  _scheduled = false;
}
