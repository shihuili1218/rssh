import { invoke } from "@tauri-apps/api/core";

const isMobile = typeof navigator !== "undefined"
  && /Android|iPhone|iPad/i.test(navigator.userAgent);

/** Read text from the system clipboard. Errors are intentionally preserved. */
export async function readText(): Promise<string> {
  return isMobile
    ? await navigator.clipboard.readText()
    : await invoke<string>("clipboard_read");
}

/** Write text to the system clipboard. Errors are intentionally preserved. */
export async function writeText(text: string): Promise<void> {
  if (isMobile) {
    await navigator.clipboard.writeText(text);
    return;
  }
  await invoke<void>("clipboard_write", { text });
}
