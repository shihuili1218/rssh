import { describe, expect, it } from "vitest";

import {
  ACTIONS,
  bindingKey,
  collidingAction,
  defaultBinding,
  effectiveBindings,
  eventToBinding,
  findConflicts,
  formatBinding,
  isDefaultBinding,
  isModifierKey,
  matchBinding,
  parseOverrides,
  reservedConflict,
  serializeOverrides,
  TAB_CYCLE,
  validateBinding,
  type ActionId,
  type KeyBinding,
  type KeyEventLike,
} from "./keymap.ts";

/** Build a keydown-shaped object; everything defaults to "not pressed". */
function ev(partial: Partial<KeyEventLike>): KeyEventLike {
  return { key: "", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...partial };
}

describe("matchBinding", () => {
  it("matches when key and every modifier line up exactly", () => {
    expect(matchBinding(ev({ key: "w", ctrlKey: true }), { key: "w", ctrl: true })).toBe(true);
  });

  it("does not treat meta and ctrl as interchangeable", () => {
    // The whole point of D1: ⌘W and Ctrl+W are different combos now.
    expect(matchBinding(ev({ key: "w", metaKey: true }), { key: "w", ctrl: true })).toBe(false);
    expect(matchBinding(ev({ key: "w", ctrlKey: true }), { key: "w", meta: true })).toBe(false);
  });

  it("rejects extra modifiers the binding did not ask for", () => {
    expect(matchBinding(ev({ key: "w", ctrlKey: true, shiftKey: true }), { key: "w", ctrl: true })).toBe(false);
  });

  it("requires modifiers the binding asked for", () => {
    expect(matchBinding(ev({ key: "d", ctrlKey: true }), { key: "d", ctrl: true, shift: true })).toBe(false);
  });

  it("compares the key case-insensitively", () => {
    expect(matchBinding(ev({ key: "W", ctrlKey: true, shiftKey: true }), { key: "w", ctrl: true, shift: true })).toBe(true);
  });

  it("handles named keys", () => {
    expect(matchBinding(ev({ key: "Tab", ctrlKey: true }), { key: "Tab", ctrl: true })).toBe(true);
  });
});

describe("defaultBinding — D1: exact per-platform modifiers", () => {
  it("defaults tab.close to ⌘W on mac (Ctrl+W must fall through to the shell)", () => {
    const mac = defaultBinding("tab.close", true);
    expect(mac).toEqual({ key: "w", meta: true });
    // Pressing Ctrl+W on mac must NOT close the tab → shell gets werase.
    expect(matchBinding(ev({ key: "w", ctrlKey: true }), mac)).toBe(false);
    expect(matchBinding(ev({ key: "w", metaKey: true }), mac)).toBe(true);
  });

  it("defaults tab.close to Ctrl+W off mac", () => {
    const other = defaultBinding("tab.close", false);
    expect(other).toEqual({ key: "w", ctrl: true });
    expect(matchBinding(ev({ key: "w", ctrlKey: true }), other)).toBe(true);
  });

  it("keeps copy/paste on literal Ctrl+Shift on every platform", () => {
    expect(defaultBinding("term.paste", true)).toEqual({ key: "v", ctrl: true, shift: true });
    expect(defaultBinding("term.copy", false)).toEqual({ key: "c", ctrl: true, shift: true });
  });
});

describe("eventToBinding", () => {
  it("normalizes the key to lower case and keeps only pressed modifiers", () => {
    expect(eventToBinding(ev({ key: "W", ctrlKey: true, shiftKey: true }))).toEqual({ key: "w", ctrl: true, shift: true });
  });

  it("preserves named keys verbatim", () => {
    expect(eventToBinding(ev({ key: "Tab", ctrlKey: true }))).toEqual({ key: "Tab", ctrl: true });
  });
});

describe("formatBinding", () => {
  it("renders mac glyphs without separators", () => {
    expect(formatBinding({ key: "w", meta: true }, true)).toBe("⌘W");
    expect(formatBinding({ key: "d", meta: true, shift: true }, true)).toBe("⌘⇧D");
    expect(formatBinding({ key: "v", ctrl: true, shift: true }, true)).toBe("⌃⇧V");
  });

  it("renders plus-separated names off mac", () => {
    expect(formatBinding({ key: "w", ctrl: true }, false)).toBe("Ctrl+W");
    expect(formatBinding({ key: "v", ctrl: true, shift: true }, false)).toBe("Ctrl+Shift+V");
    expect(formatBinding({ key: "Tab", ctrl: true }, false)).toBe("Ctrl+Tab");
  });
});

describe("bindingKey", () => {
  it("is equal for identical combos and distinct otherwise", () => {
    expect(bindingKey({ key: "w", ctrl: true })).toBe(bindingKey({ key: "W", ctrl: true }));
    expect(bindingKey({ key: "w", ctrl: true })).not.toBe(bindingKey({ key: "w", meta: true }));
    expect(bindingKey({ key: "w", ctrl: true })).not.toBe(bindingKey({ key: "w", ctrl: true, shift: true }));
  });
});

describe("validateBinding — the shell-input guard", () => {
  it("rejects a bare key", () => {
    expect(validateBinding({ key: "k" }).ok).toBe(false);
  });

  it("rejects shift-only (still a printable char to the shell)", () => {
    expect(validateBinding({ key: "k", shift: true }).ok).toBe(false);
  });

  it("accepts any of ctrl / meta / alt", () => {
    expect(validateBinding({ key: "k", ctrl: true }).ok).toBe(true);
    expect(validateBinding({ key: "k", meta: true }).ok).toBe(true);
    expect(validateBinding({ key: "k", alt: true }).ok).toBe(true);
  });

  it("rejects a modifier key as the binding key (else bare Control would fire it)", () => {
    expect(validateBinding({ key: "Control", ctrl: true }).ok).toBe(false);
    expect(validateBinding({ key: "control", ctrl: true }).ok).toBe(false);
    expect(validateBinding({ key: "Shift", ctrl: true, shift: true }).ok).toBe(false);
  });
});

describe("isModifierKey", () => {
  it("matches the four modifier names case-insensitively, nothing else", () => {
    for (const k of ["Control", "Shift", "Alt", "Meta", "control", "META"]) {
      expect(isModifierKey(k)).toBe(true);
    }
    expect(isModifierKey("w")).toBe(false);
    expect(isModifierKey("Tab")).toBe(false);
  });
});

describe("isDefaultBinding", () => {
  it("recognizes the current platform default, case-insensitively on the key", () => {
    expect(isDefaultBinding("tab.close", { key: "w", meta: true }, true)).toBe(true);
    expect(isDefaultBinding("tab.close", { key: "W", meta: true }, true)).toBe(true);
    expect(isDefaultBinding("tab.close", { key: "w", ctrl: true }, false)).toBe(true);
  });

  it("rejects deviations and the other platform's default", () => {
    expect(isDefaultBinding("tab.close", { key: "j", ctrl: true }, false)).toBe(false);
    // mac default is ⌘W, so Ctrl+W is NOT the mac default
    expect(isDefaultBinding("tab.close", { key: "w", ctrl: true }, true)).toBe(false);
  });
});

describe("effectiveBindings", () => {
  it("returns a binding for every action with no overrides", () => {
    const eff = effectiveBindings({}, true);
    for (const a of ACTIONS) {
      expect(eff[a.id]).toEqual(defaultBinding(a.id, true));
    }
  });

  it("lets an override win while leaving others at default", () => {
    const eff = effectiveBindings({ "tab.close": { key: "j", ctrl: true } }, true);
    expect(eff["tab.close"]).toEqual({ key: "j", ctrl: true });
    expect(eff["term.search"]).toEqual(defaultBinding("term.search", true));
  });
});

describe("collidingAction — the shared guard for record AND reset", () => {
  it("returns null when no other action holds the combo", () => {
    const eff = effectiveBindings({}, true);
    expect(collidingAction("tab.close", { key: "k", meta: true }, eff)).toBeNull();
    // a shipped default never collides with another action's default
    expect(collidingAction("tab.close", defaultBinding("tab.close", true), eff)).toBeNull();
  });

  it("names another action occupying the combo and skips self", () => {
    // term.search has been moved onto ⌘W (tab.close's mac default).
    const eff = effectiveBindings({ "term.search": { key: "w", meta: true } }, true);
    // Reset/record path for tab.close → its default ⌘W now clashes with term.search.
    expect(collidingAction("tab.close", { key: "w", meta: true }, eff)).toBe("term.search");
    // Symmetric: querying term.search's own combo finds tab.close's default, not itself.
    expect(collidingAction("term.search", { key: "w", meta: true }, eff)).toBe("tab.close");
  });
});

describe("findConflicts", () => {
  it("reports nothing for the shipped defaults (mac and other)", () => {
    expect(findConflicts(effectiveBindings({}, true))).toEqual([]);
    expect(findConflicts(effectiveBindings({}, false))).toEqual([]);
  });

  it("groups two actions that collide on the same combo", () => {
    const close = defaultBinding("tab.close", true);
    const groups = findConflicts(effectiveBindings({ "term.search": close }, true));
    expect(groups.length).toBe(1);
    expect([...groups[0]].sort()).toEqual(["tab.close", "term.search"]);
  });
});

describe("serialize / parse overrides", () => {
  it("round-trips a valid overrides map", () => {
    const o = { "tab.close": { key: "j", ctrl: true } } as const;
    expect(parseOverrides(serializeOverrides(o))).toEqual(o);
  });

  it("returns an empty map for garbage or nullish input", () => {
    expect(parseOverrides("not json")).toEqual({});
    expect(parseOverrides(null)).toEqual({});
    expect(parseOverrides(undefined)).toEqual({});
  });

  it("drops unknown action ids and malformed bindings", () => {
    expect(parseOverrides(JSON.stringify({ "bogus.id": { key: "x", ctrl: true } }))).toEqual({});
    expect(parseOverrides(JSON.stringify({ "tab.close": { ctrl: true } }))).toEqual({}); // no key
  });

  it("drops loaded overrides that fail the same validity rules as recording", () => {
    // No modifier — a corrupt/dev write would otherwise close the tab on bare 'w'.
    expect(parseOverrides(JSON.stringify({ "tab.close": { key: "w" } }))).toEqual({});
    // Reserved fixed combo — would be swallowed by the tab cycler.
    expect(parseOverrides(JSON.stringify({ "tab.close": { key: "Tab", ctrl: true } }))).toEqual({});
    // Modifier key as the bound key — would fire when the user merely presses Control.
    expect(parseOverrides(JSON.stringify({ "tab.close": { key: "Control", ctrl: true } }))).toEqual({});
  });
});

describe("reservedConflict — fixed combos can't be stolen", () => {
  it("flags the tab-cycling combos with their label key", () => {
    expect(reservedConflict({ key: "Tab", ctrl: true })).toBe("shortcut.tab.cycle");
    expect(reservedConflict({ key: "Tab", ctrl: true, shift: true })).toBe("shortcut.tab.cycle");
  });

  it("returns null for combos that aren't reserved", () => {
    expect(reservedConflict({ key: "w", meta: true })).toBeNull();
    expect(reservedConflict({ key: "f", ctrl: true })).toBeNull();
  });

  it("never collides with a shipped default (mac or other)", () => {
    for (const a of ACTIONS) {
      expect(reservedConflict(defaultBinding(a.id, true))).toBeNull();
      expect(reservedConflict(defaultBinding(a.id, false))).toBeNull();
    }
  });

  it("reserves exactly the combos the tab cycler matches", () => {
    // Same data drives the AppShell cycler predicate, so the two can't drift.
    expect(TAB_CYCLE.length).toBe(2);
    for (const b of TAB_CYCLE) expect(reservedConflict(b)).toBe("shortcut.tab.cycle");
  });
});

describe("ACTIONS metadata", () => {
  it("has 9 unique actions split 4 global / 5 terminal", () => {
    expect(ACTIONS.length).toBe(9);
    const ids = ACTIONS.map((a) => a.id);
    expect(new Set(ids).size).toBe(9);
    expect(ACTIONS.filter((a) => a.surface === "global").length).toBe(4);
    expect(ACTIONS.filter((a) => a.surface === "terminal").length).toBe(5);
  });

  it("gives every action a mac and other default plus a label key", () => {
    for (const a of ACTIONS) {
      expect(typeof a.labelKey).toBe("string");
      expect(a.mac.key).toBeTruthy();
      expect(a.other.key).toBeTruthy();
    }
  });
});
