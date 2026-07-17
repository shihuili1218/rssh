import { describe, expect, it } from "vitest";
import { defaultPanelWidth, fitPanelWidths, resizePanelWidth } from "./panel-widths.ts";

const base = {
  containerWidth: 1200,
  mainMinWidth: 320,
  panelMinWidth: 280,
  defaultWidth: 380,
  aiVisible: true,
  sftpVisible: true,
  aiWidth: null,
  sftpWidth: null,
};

describe("fitPanelWidths", () => {
  it("clamps two restored widths without changing their stored preferences", () => {
    const fitted = fitPanelWidths({ ...base, aiWidth: 800, sftpWidth: 800 });

    expect(fitted).toEqual({ ai: 440, sftp: 440 });
    expect(fitted.ai + fitted.sftp + base.mainMinWidth).toBe(base.containerWidth);
  });

  it("gives unused space from the smaller panel to the larger panel", () => {
    expect(fitPanelWidths({ ...base, aiWidth: 280, sftpWidth: 800 })).toEqual({
      ai: 280,
      sftp: 600,
    });
  });

  it("keeps the dragged panel stable when its fitted width becomes its preference", () => {
    const initial = fitPanelWidths({ ...base, aiWidth: 800, sftpWidth: 800 });
    const afterDrag = fitPanelWidths({
      ...base,
      aiWidth: initial.ai,
      sftpWidth: 800,
    });

    expect(afterDrag).toEqual(initial);
  });

  it("re-clamps on a narrow container instead of overflowing", () => {
    const fitted = fitPanelWidths({
      ...base,
      containerWidth: 700,
      aiWidth: 800,
      sftpWidth: 800,
    });

    expect(fitted).toEqual({ ai: 190, sftp: 190 });
    expect(fitted.ai + fitted.sftp + base.mainMinWidth).toBe(700);
  });

  it("does not reserve width for a hidden opposite panel", () => {
    expect(fitPanelWidths({
      ...base,
      sftpVisible: false,
      aiWidth: 800,
      sftpWidth: 800,
    })).toEqual({ ai: 800, sftp: 800 });
  });

  it("preserves the responsive default for one panel on a narrow window", () => {
    expect(fitPanelWidths({
      ...base,
      containerWidth: 500,
      defaultWidth: 320,
      sftpVisible: false,
    })).toEqual({ ai: 320, sftp: 320 });
  });

  it("keeps one restored panel usable when the main minimum cannot also fit", () => {
    expect(fitPanelWidths({
      ...base,
      containerWidth: 500,
      defaultWidth: 320,
      aiWidth: 800,
      sftpVisible: false,
    })).toEqual({ ai: 280, sftp: 320 });
  });

  it("uses the viewport breakpoint rather than the sidebar-reduced content width", () => {
    expect(defaultPanelWidth(850)).toBe(380);
    expect(defaultPanelWidth(800)).toBe(320);
  });

  it("lets one gesture shrink and then restore against its captured opposite width", () => {
    const gesture = {
      startWidth: 440,
      sign: 1,
      minWidth: 280,
      containerWidth: 1200,
      mainMinWidth: 320,
      otherWidthAtStart: 440,
    };

    expect(resizePanelWidth({ ...gesture, deltaX: -10 })).toBe(430);
    expect(resizePanelWidth({ ...gesture, deltaX: 0 })).toBe(440);
  });
});
