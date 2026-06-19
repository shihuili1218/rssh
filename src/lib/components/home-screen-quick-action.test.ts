import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const homeScreenSource = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "HomeScreen.svelte"),
  "utf8",
);

describe("HomeScreen quick profile action", () => {
  it("offers a direct Home-screen action to create a new SSH profile", () => {
    expect(homeScreenSource).toContain('onclick={() => app.navigate("profile-edit")}');
    expect(homeScreenSource).toContain('t("home.new_profile")');
  });
});
