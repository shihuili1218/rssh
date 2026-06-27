/**
 * Best-effort remote filename for a mobile SFTP upload, derived from the local
 * source reference the dialog plugin returns. On Android that's a SAF
 * `content://` URI whose last segment encodes the display name for user-visible
 * providers (e.g. `...%2FDownload%2Freport.pdf` → `report.pdf`); opaque
 * providers yield only an id.
 *
 * Returns the derived name, or "" when nothing usable can be recovered (the
 * caller supplies a timestamped fallback). Splits on `/`, `\` and `:` so both
 * path separators and SAF's `tree:` / `document:` id prefixes are stripped.
 */
export function remoteUploadName(ref: string): string {
  let decoded = ref;
  try {
    decoded = decodeURIComponent(ref);
  } catch {
    /* malformed %-escape — fall back to the raw string */
  }
  return (decoded.split(/[\\/:]/).pop() || "").trim();
}
