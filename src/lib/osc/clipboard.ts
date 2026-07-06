const OSC_CLIPBOARD_ID = 52;
const MAX_OSC52_BASE64_CHARS = 1_400_000; // About 1 MiB decoded.

export interface OscParser {
  registerOscHandler(id: number, handler: (data: string) => boolean): void;
}

export interface ClipboardWriter {
  writeText(text: string): Promise<void> | void;
}

function decodesToClipboard(selector: string): boolean {
  return selector === "" || selector.includes("c");
}

function normalizeBase64(encoded: string): string | null {
  const compact = encoded.replace(/\s/g, "");
  if (compact.length > MAX_OSC52_BASE64_CHARS) return null;

  const remainder = compact.length % 4;
  if (remainder === 1) return null;
  return compact + "=".repeat((4 - remainder) % 4);
}

function decodeBase64Utf8(encoded: string): string | null {
  const normalized = normalizeBase64(encoded);
  if (normalized === null) return null;

  try {
    const binary = atob(normalized);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

/** Register OSC 52 clipboard writes (`OSC 52 ; c ; <base64> ST`).
 *
 *  Tools like zellij/tmux running inside SSH cannot call the desktop clipboard
 *  directly, so they ask the terminal emulator via OSC 52. We support write-only
 *  clipboard requests and deliberately ignore read queries (`?`). Unlike the
 *  app-specific OSC 7337 integration, this is a standard terminal clipboard
 *  protocol; remote sessions are the core use case.
 */
export function registerClipboardOscHandler(parser: OscParser, clipboard: ClipboardWriter): void {
  parser.registerOscHandler(OSC_CLIPBOARD_ID, (data: string) => {
    const sep = data.indexOf(";");
    if (sep < 0) return false;

    const selector = data.slice(0, sep);
    if (!decodesToClipboard(selector)) return false;

    const encoded = data.slice(sep + 1);
    if (encoded === "?") return true;

    const text = decodeBase64Utf8(encoded);
    if (text === null) return true;

    try {
      void Promise.resolve(clipboard.writeText(text)).catch(() => {});
    } catch {
      // Clipboard failures are non-fatal terminal output events.
    }
    return true;
  });
}
