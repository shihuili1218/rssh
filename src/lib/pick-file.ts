/**
 * Read a user-picked file's text via a hidden `<input type=file>`.
 *
 * One path for every webview we ship: desktop Tauri (WKWebView / WebView2 /
 * WebKitGTK all wire up the native open panel), mobile Tauri (Android's
 * `onShowFileChooser` → SAF document picker), and a plain browser. The webview
 * reads the bytes itself, so there's no Rust round-trip and no native dialog —
 * which also means no `~/.ssh` default directory (the webview can't set one).
 *
 * The lone exception is the IDEA plugin's JCEF host: it silently drops file
 * inputs (no CefDialogHandler can be bound across IDE versions), so we reject
 * up front with a localizable error instead of leaving the user with a dead
 * button.
 *
 * Resolves null when the user cancels.
 */
export interface PickedFile {
  name: string;
  text: string;
}

export function pickTextFile(
  opts: { accept?: string; maxBytes?: number } = {},
): Promise<PickedFile | null> {
  if (typeof (window as any).__RSSH_PICK__ === "function")
    return Promise.reject(
      `__rssh_err__|${JSON.stringify({ code: "file_pick_unsupported_in_plugin", params: {} })}`,
    );

  const { accept = "", maxBytes } = opts;
  return new Promise((resolve, reject) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.style.display = "none";

    let settled = false;
    const finish = (run: () => void) => {
      if (settled) return;
      settled = true;
      input.remove();
      run();
    };

    input.addEventListener("change", async () => {
      const file = input.files?.[0];
      if (!file) return finish(() => resolve(null));
      if (maxBytes != null && file.size > maxBytes)
        return finish(() =>
          reject(
            `__rssh_err__|${JSON.stringify({ code: "key_file_too_large", params: { size: file.size } })}`,
          ),
        );
      const text = await file.text();
      finish(() => resolve({ name: file.name, text }));
    });

    // Cancellation fires no reliable cross-browser event; treat "window
    // refocused without a change" as cancel — same heuristic the old shim used.
    window.addEventListener("focus", () => setTimeout(() => finish(() => resolve(null)), 500), {
      once: true,
    });

    document.body.appendChild(input);
    input.click();
  });
}
