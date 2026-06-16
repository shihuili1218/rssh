import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // xterm 6.0's InputHandler.requestMode uses `let r; … (r ||= {})`. esbuild,
  // downleveling that es2021 logical-assignment to Vite's default `modules`
  // (~es2020) target, drops the `let r` declaration → `void 0 || (r = {})` →
  // ReferenceError at runtime. It only fires when a TUI sends DECRQM mode queries
  // (vim/htop on the alternate buffer) and only in a minified prod build, so dev
  // and the normal-screen shell looked fine while every packaged vim froze.
  // Raising the target to es2021 stops the downlevel, so the bug never triggers —
  // no extra dependency, full minification kept. The WKWebView we ship in supports
  // es2021 natively. xterm confirms it's an esbuild bug, not theirs:
  // https://github.com/xtermjs/xterm.js/issues/5800 / https://github.com/evanw/esbuild/issues/3125
  build: {
    target: "es2021",
  },
  server: {
    host: host || false,
    port: 1420,
    strictPort: true,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
