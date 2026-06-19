// MUST be first — installs the IPC shim before any store/component module
// evaluates and touches invoke/listen (no-op inside the real Tauri webview).
import "./lib/ipc-boot.ts";
import App from "./App.svelte";
import { mount } from "svelte";
import * as theme from "./lib/themes/store.svelte.ts";
import * as transfers from "./lib/stores/transfers.svelte.ts";
import * as app from "./lib/stores/app.svelte.ts";

// Apply persisted theme before mount so first paint reflects the user's choice.
// We don't await — startup paint blocks on the persisted lookup otherwise. The
// :root literal defaults match the dark-neumorphism preset, so the worst case
// is a brief flicker if the user picked a different palette.
theme.init();

// SFTP 并发上限：从 DB 拉持久化值覆盖默认 10。fire-and-forget —— 用户在做出
// 第一笔 transfer 前这个 promise 已经 resolve；万一没（极快点击）也只是用一次默认值，
// 不影响功能。
void transfers.loadMaxConcurrent();

// Terminal interaction prefs (copy-on-select + right-click action). Global,
// loaded once at startup so the first terminal honors persisted values without
// waiting for the Settings screen. fire-and-forget like loadMaxConcurrent.
void app.loadCopyOnSelect();
void app.loadRightClickAction();
void app.loadConfirmCloseTab();

const instance = mount(App, { target: document.getElementById("app")! });

export default instance;
