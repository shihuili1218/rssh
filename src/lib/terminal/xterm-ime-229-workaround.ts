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
 * Upstream: https://github.com/xtermjs/xterm.js/issues/5887
 *
 * Keep this as one removable RSSH workaround: listen on the terminal host in
 * capture phase, before xterm's helper-textarea input listener, and only patch
 * the next composed insertText after a non-composing 229 keydown. Remove this
 * module once the upstream input guard no longer drops that sequence.
 */
export function setupXtermIme229Workaround({
  terminal,
  host,
  enabled,
}: SetupXtermIme229WorkaroundOptions): () => void {
  if (!enabled) return () => {};

  const textarea = terminal.textarea;
  if (!textarea) return () => {};

  let pending229 = false;

  function isTextareaEvent<T extends Event>(event: T): event is T & { target: HTMLTextAreaElement } {
    return event.target === textarea;
  }

  function isInputEvent(event: Event): event is InputEvent {
    return "data" in event && "inputType" in event && "isComposing" in event;
  }

  function clearPending() {
    pending229 = false;
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!isTextareaEvent(event)) return;
    pending229 = event.keyCode === 229 && !event.isComposing;
  }

  function onKeyUp(event: KeyboardEvent) {
    if (isTextareaEvent(event)) clearPending();
  }

  function onComposition(event: CompositionEvent) {
    if (isTextareaEvent(event)) clearPending();
  }

  function onInput(event: Event) {
    if (!pending229 || !isTextareaEvent(event)) return;
    pending229 = false;
    if (!isInputEvent(event) || terminal.options.screenReaderMode) return;

    if (
      !event.composed ||
      event.isComposing ||
      event.inputType !== "insertText" ||
      !event.data
    ) {
      return;
    }

    terminal.input(event.data, true);
    event.target.value = "";

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
