import App from "./App.svelte";
import { mount } from "svelte";
import * as theme from "./lib/themes/store.svelte.ts";

// Apply persisted theme before mount so first paint reflects the user's choice.
// We don't await — startup paint blocks on the persisted lookup otherwise. The
// :root literal defaults match the dark-neumorphism preset, so the worst case
// is a brief flicker if the user picked a different palette.
theme.init();

const app = mount(App, { target: document.getElementById("app")! });

export default app;
