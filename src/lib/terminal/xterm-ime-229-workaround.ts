import type { Terminal } from "@xterm/xterm";

type WorkaroundTerminal = Pick<Terminal, "input" | "options" | "textarea">;
type ListenerHost = Pick<HTMLElement, "addEventListener" | "removeEventListener">;

type SetupXtermIme229WorkaroundOptions = {
  terminal: WorkaroundTerminal;
  host: ListenerHost;
  enabled: boolean;
};

/**
 * xterm 6 can miss text in macOS WebKit when an IME-like app emits a plain
 * insertText event behind keyCode=229 without a real composition session.
 *
 * Keep this as one removable RSSH workaround: listen on the terminal host in
 * capture phase, before xterm's helper-textarea input listener, and only patch
 * the narrow "229 keydown -> insertText" sequence.
 */
export function setupXtermIme229Workaround({
  terminal,
  host,
  enabled,
}: SetupXtermIme229WorkaroundOptions): () => void {
  if (!enabled) return () => {};

  let pending229 = false;

  function isTextareaEvent(event: Event): boolean {
    return event.target === terminal.textarea;
  }

  function clearPending() {
    pending229 = false;
  }

  function onKeyDown(event: Event) {
    if (!isTextareaEvent(event)) return;
    const keyboardEvent = event as KeyboardEvent;
    pending229 = keyboardEvent.keyCode === 229 && !keyboardEvent.isComposing;
  }

  function onKeyUp(event: Event) {
    if (isTextareaEvent(event)) clearPending();
  }

  function onComposition(event: Event) {
    if (isTextareaEvent(event)) clearPending();
  }

  function onInput(event: Event) {
    if (!pending229 || !isTextareaEvent(event)) return;
    if (terminal.options.screenReaderMode) return;

    const inputEvent = event as InputEvent;
    if (inputEvent.isComposing || inputEvent.inputType !== "insertText" || !inputEvent.data) return;

    terminal.input(inputEvent.data, true);
    terminal.textarea.value = "";
    pending229 = false;

    event.stopPropagation();
    (event as Event & { stopImmediatePropagation?: () => void }).stopImmediatePropagation?.();
    if (event.cancelable) event.preventDefault();
  }

  host.addEventListener("keydown", onKeyDown, { capture: true });
  host.addEventListener("keyup", onKeyUp, { capture: true });
  host.addEventListener("compositionstart", onComposition, { capture: true });
  host.addEventListener("compositionend", onComposition, { capture: true });
  host.addEventListener("input", onInput, { capture: true });

  return () => {
    host.removeEventListener("keydown", onKeyDown, { capture: true });
    host.removeEventListener("keyup", onKeyUp, { capture: true });
    host.removeEventListener("compositionstart", onComposition, { capture: true });
    host.removeEventListener("compositionend", onComposition, { capture: true });
    host.removeEventListener("input", onInput, { capture: true });
    clearPending();
  };
}
