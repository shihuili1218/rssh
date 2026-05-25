import App from "./App.svelte";
import { mount } from "svelte";
import * as theme from "./lib/themes/store.svelte.ts";

// Apply persisted theme before mount so first paint reflects the user's choice.
// We don't await — startup paint blocks on the persisted lookup otherwise.
//
// 配合 index.html 里的 <html class="preload"> + 内联禁 transition 规则：
// theme.init() 异步把 data-shape / data-density / palette 写入 <html>，桌面/iOS
// 上几乎和首屏同时，但 Android Tauri WebView 冷启动 I/O 慢，会晚一拍——这时如果
// 不禁 transition，box-shadow / transform 会从 :root 默认值动画到主题终值，表现
// 为所有按钮闪一下。preload class 让那一拍直接落地终态，下一帧再恢复正常动画。
theme.init().finally(() => {
    requestAnimationFrame(() => {
        document.documentElement.classList.remove("preload");
    });
});

const app = mount(App, { target: document.getElementById("app")! });

export default app;
