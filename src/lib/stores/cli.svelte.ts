import { invoke } from "@tauri-apps/api/core";

const INITIAL_DELAY_MS = 10_000;
const INTERVAL_MS = 6 * 60 * 60 * 1000;

export type CliStatus = {
  installed: boolean;
  path: string;
  bundled: boolean;
  installed_version: string | null;
  expected_version: string;
  needs_update: boolean;
};

export type CliState =
  | { kind: "unknown" }
  | { kind: "checking" }
  | { kind: "ready"; status: CliStatus }
  | { kind: "error" };

let _state = $state<CliState>({ kind: "unknown" });
let _scheduled = false;
let _initialTimer: ReturnType<typeof setTimeout> | null = null;
let _intervalTimer: ReturnType<typeof setInterval> | null = null;

export function state(): CliState { return _state; }
export function status(): CliStatus | null {
  return _state.kind === "ready" ? _state.status : null;
}
export function needsAttention(): boolean {
  return _state.kind === "ready" && _state.status.needs_update;
}

export async function runCheck(opts?: { silent?: boolean }): Promise<void> {
  if (_state.kind === "checking") return;
  const previous = _state;
  _state = { kind: "checking" };
  try {
    _state = { kind: "ready", status: await invoke<CliStatus>("cli_status") };
  } catch (error) {
    console.error("cli.runCheck failed:", error);
    _state = opts?.silent ? previous : { kind: "error" };
  }
}

export function startBackgroundChecks(): void {
  if (_scheduled) return;
  _scheduled = true;
  _initialTimer = setTimeout(() => {
    runCheck({ silent: true });
    _intervalTimer = setInterval(() => runCheck({ silent: true }), INTERVAL_MS);
  }, INITIAL_DELAY_MS);
}

export function stopBackgroundChecks(): void {
  if (_initialTimer) { clearTimeout(_initialTimer); _initialTimer = null; }
  if (_intervalTimer) { clearInterval(_intervalTimer); _intervalTimer = null; }
  _scheduled = false;
}
