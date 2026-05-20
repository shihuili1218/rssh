<script lang="ts">
  import { onMount } from "svelte";
  import AppShell from "./lib/components/AppShell.svelte";
  import ToastStack from "./lib/components/ToastStack.svelte";
  import WelcomeScreen from "./lib/components/WelcomeScreen.svelte";
  import { loadProfiles, loadForwards } from "./lib/stores/app.svelte.ts";
  import * as updates from "./lib/stores/updates.svelte.ts";

  // First-launch auto-show: when there are no profiles and no forwards,
  // surface the cinematic welcome once. After the user dismisses it
  // (Get Started / Skip / Esc / Close) we set this localStorage flag
  // so a later "wiped everything" state doesn't loop the demo on every
  // boot. Settings → About → "Preview welcome screen" is the manual
  // replay path; it deliberately bypasses this flag.
  const DISMISSED_KEY = "rssh.welcome.dismissed";

  let showWelcome = $state(false);

  onMount(async () => {
    // Skip background update polling on clone / AI-handoff windows —
    // they're transient and the main window already owns the timer.
    if (!window.__rssh_clone && !window.__rssh_ai_handoff) {
      updates.startBackgroundChecks();
    }

    // localStorage / Tauri may not be available in non-app hosts
    // (e.g. vitest, browser preview) — defensively swallow errors so a
    // missing API never blocks the regular AppShell from rendering.
    let dismissed = false;
    try {
      dismissed = localStorage.getItem(DISMISSED_KEY) === "true";
    } catch {}
    if (dismissed) return;

    try {
      const [profiles, forwards] = await Promise.all([
        loadProfiles(),
        loadForwards(),
      ]);
      if (profiles.length === 0 && forwards.length === 0) {
        showWelcome = true;
      }
    } catch (e) {
      console.debug("welcome auto-check skipped:", e);
    }
  });

  function dismissWelcome() {
    showWelcome = false;
    try {
      localStorage.setItem(DISMISSED_KEY, "true");
    } catch {}
  }
</script>

<AppShell />
<ToastStack />

{#if showWelcome}
  <WelcomeScreen onDismiss={dismissWelcome} />
{/if}
