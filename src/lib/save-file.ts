/**
 * Save text to a user-chosen file. One path for desktop, mobile and browser —
 * the write side mirror of pickTextFile.
 *
 * - real Tauri (desktop + mobile): native save dialog (`@tauri-apps/plugin-dialog`)
 *   + write (`@tauri-apps/plugin-fs`). fs accepts both desktop paths and the
 *   `content://` URIs Android's SAF returns, so one call covers every target.
 * - plain browser: a Blob download to the downloads folder.
 * - JCEF (IDE plugin): downloads are silently dropped, so reject with a clear,
 *   localizable error instead of doing nothing.
 *
 * Resolves the chosen path/name, or null if the user cancelled.
 */
export interface SaveOpts {
  defaultName: string;
  filters?: { name: string; extensions: string[] }[];
}

export async function saveTextFile(content: string, opts: SaveOpts): Promise<string | null> {
  // Off-Tauri: no plugin runtime. The IDE plugin (JCEF) drops downloads, so it
  // gets a clear message; a plain browser rides a Blob download.
  if (!(window as any).__TAURI_INTERNALS__) {
    if (typeof (window as any).__RSSH_PICK__ === "function")
      return Promise.reject(
        `__rssh_err__|${JSON.stringify({ code: "file_save_unsupported_in_plugin", params: {} })}`,
      );
    downloadTextBlob(content, opts.defaultName);
    return opts.defaultName;
  }

  // Loaded lazily so a browser build never pulls the plugin modules.
  const { save } = await import("@tauri-apps/plugin-dialog");
  const { writeTextFile } = await import("@tauri-apps/plugin-fs");
  const path = await save({ defaultPath: opts.defaultName, filters: opts.filters });
  if (path == null) return null; // user cancelled
  await writeTextFile(path, content);
  return path;
}

/** Trigger a browser download of `content` saved as `filename`. */
function downloadTextBlob(content: string, filename: string): void {
  const url = URL.createObjectURL(new Blob([content]));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** `YYYYMMDD-HHMMSS` stamp for default export filenames. */
export function fileStamp(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}
