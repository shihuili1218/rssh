import { readdirSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  APP_ICON_NAMES,
  connectionIconName,
  dynamicPlatformIconName,
  tabIconName,
} from "./app-icon";

const FORBIDDEN_UI_GLYPHS = [
  "\u26a1", "\ud83d\udcc1", "\ud83d\udd17", "\ud83d\udcc4", "\ud83d\udccc", "\u2699",
  "\u2605", "\u2606", "\u270e", "\u26f0", "\u2601", "\u26a0", "\u2393",
  "\u32e1", "\u1770", "\u2726", "\ud80c\udc7c", "\u0f7c", "\u0f04",
] as const;

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return extname(path) === ".svelte" ? [path] : [];
  });
}

describe("app icon registry", () => {
  it("maps every connection and terminal kind to a semantic SVG icon", () => {
    expect(connectionIconName("ssh")).toBe("ssh");
    expect(connectionIconName("forward")).toBe("forward");
    expect(connectionIconName("serial")).toBe("serial");
    expect(connectionIconName("telnet")).toBe("telnet");

    expect(tabIconName("home")).toBe("home");
    expect(tabIconName("local")).toBe("terminal");
    expect(tabIconName("docker_exec")).toBe("docker");
    expect(tabIconName("kubectl_exec")).toBe("kubernetes");
    expect(tabIconName("edit")).toBe("edit");

    expect(dynamicPlatformIconName("docker")).toBe("docker");
    expect(dynamicPlatformIconName("k8s")).toBe("kubernetes");
    expect(new Set(APP_ICON_NAMES).size).toBe(APP_ICON_NAMES.length);
  });

  it("keeps emoji and font glyphs out of UI icon slots", () => {
    const roots = [join(process.cwd(), "src/lib/components"), join(process.cwd(), "src/lib/ai")];
    const offenders = sourceFiles(roots[0])
      .concat(sourceFiles(roots[1]))
      .flatMap((path) => {
        const source = readFileSync(path, "utf8");
        return FORBIDDEN_UI_GLYPHS
          .filter((glyph) => source.includes(glyph))
          .map((glyph) => `${path.slice(process.cwd().length + 1)}: ${glyph}`);
      });

    expect(offenders).toEqual([]);
  });
});
