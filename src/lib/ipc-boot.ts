// Side-effect entry: install the Tauri IPC shim as early as possible — before
// any store or component module evaluates and calls invoke/listen. Kept separate
// from ipc-shim.ts so the shim itself stays a pure, unit-testable function.
import { installTauriShim } from "./ipc-shim.ts";

installTauriShim();
