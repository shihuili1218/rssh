import App from "./App.svelte";
import { mount } from "svelte";
import * as theme from "./lib/themes/store.svelte.ts";
import * as transfers from "./lib/stores/transfers.svelte.ts";

// Apply persisted theme before mount so first paint reflects the user's choice.
// We don't await — startup paint blocks on the persisted lookup otherwise. The
// :root literal defaults match the dark-neumorphism preset, so the worst case
// is a brief flicker if the user picked a different palette.
theme.init();

// SFTP 并发上限：从 DB 拉持久化值覆盖默认 10。fire-and-forget —— 用户在做出
// 第一笔 transfer 前这个 promise 已经 resolve；万一没（极快点击）也只是用一次默认值，
// 不影响功能。
void transfers.loadMaxConcurrent();

const app = mount(App, { target: document.getElementById("app")! });

export default app;
