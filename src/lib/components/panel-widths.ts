export type PanelFitPriority = "ai" | "sftp";

export interface PanelWidthFitInput {
  containerWidth: number;
  mainMinWidth: number;
  panelMinWidth: number;
  defaultWidth: number;
  aiVisible: boolean;
  sftpVisible: boolean;
  aiWidth: number | null;
  sftpWidth: number | null;
  priority?: PanelFitPriority;
}

export function defaultPanelWidth(viewportWidth: number): number {
  return viewportWidth <= 800 ? 320 : 380;
}

export interface PanelResizeInput {
  startWidth: number;
  deltaX: number;
  sign: number;
  minWidth: number;
  containerWidth: number;
  mainMinWidth: number;
  otherPanelVisible: boolean;
}

/** Clamp a drag while reserving the main pane and the opposite panel's minimum. */
export function resizePanelWidth(input: PanelResizeInput): number {
  const oppositeMinimum = input.otherPanelVisible ? input.minWidth : 0;
  const maxWidth = Math.max(
    input.minWidth,
    input.containerWidth - input.mainMinWidth - oppositeMinimum,
  );
  return Math.max(
    input.minWidth,
    Math.min(maxWidth, input.startWidth + input.sign * input.deltaX),
  );
}

/**
 * Fit the two side panels into the current content width without mutating the
 * per-tab preferred widths. Restoring a large saved width, opening the opposite
 * panel, or shrinking the window therefore cannot collapse/overflow the main
 * pane; enlarging again restores the preference naturally.
 */
export function fitPanelWidths(input: PanelWidthFitInput): { ai: number; sftp: number } {
  const aiPreferred = input.aiWidth ?? input.defaultWidth;
  const sftpPreferred = input.sftpWidth ?? input.defaultWidth;
  const available = Math.max(0, input.containerWidth - input.mainMinWidth);

  if (!input.aiVisible && !input.sftpVisible) {
    return { ai: aiPreferred, sftp: sftpPreferred };
  }
  if (input.aiVisible && !input.sftpVisible) {
    const ai = input.aiWidth === null
      ? Math.min(aiPreferred, input.containerWidth)
      : fitSinglePanel(aiPreferred, available, input);
    return { ai, sftp: sftpPreferred };
  }
  if (!input.aiVisible && input.sftpVisible) {
    const sftp = input.sftpWidth === null
      ? Math.min(sftpPreferred, input.containerWidth)
      : fitSinglePanel(sftpPreferred, available, input);
    return { ai: aiPreferred, sftp };
  }

  if (aiPreferred + sftpPreferred <= available) {
    return { ai: aiPreferred, sftp: sftpPreferred };
  }

  // Below 2× panel minimum there is no way to satisfy all three minima. Split
  // the side budget evenly and preserve the main pane instead of overflowing.
  if (available <= input.panelMinWidth * 2) {
    const ai = Math.round(available / 2);
    return { ai, sftp: available - ai };
  }

  // A drag owns the remaining budget for that gesture. The opposite panel's
  // preference is deliberately left untouched, but its rendered width yields
  // down to the minimum. Switching drag owner simply switches this priority.
  if (input.priority === "ai") {
    const ai = clamp(
      aiPreferred,
      input.panelMinWidth,
      available - input.panelMinWidth,
    );
    return { ai, sftp: available - ai };
  }
  if (input.priority === "sftp") {
    const sftp = clamp(
      sftpPreferred,
      input.panelMinWidth,
      available - input.panelMinWidth,
    );
    return { ai: available - sftp, sftp };
  }

  // Water-fill: shrink the wider request down toward the narrower one first;
  // only when both exceed half the budget do they split evenly. Besides being
  // predictable, this is stable under dragging: fit(800,800)=440/440 and then
  // persisting the dragged 440 keeps fit(440,800)=440/440 instead of jumping.
  const lower = Math.min(aiPreferred, sftpPreferred);
  if (lower * 2 <= available) {
    return aiPreferred <= sftpPreferred
      ? { ai: lower, sftp: available - lower }
      : { ai: available - lower, sftp: lower };
  }
  const ai = Math.round(available / 2);
  return { ai, sftp: available - ai };
}

function fitSinglePanel(
  preferred: number,
  available: number,
  input: PanelWidthFitInput,
): number {
  const minimum = Math.min(input.panelMinWidth, input.containerWidth);
  const maximum = Math.max(minimum, available);
  return clamp(preferred, minimum, maximum);
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(value, maximum));
}
