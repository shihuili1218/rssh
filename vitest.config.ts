// 单独的 vitest 配置，不污染 vite.config.ts 的 dev server 配置。
// svelte plugin 用于 transform `.svelte.ts` 文件里的 runes（$state 等）——
// i18n 模块就是这种文件，不带 plugin 测试 import 时会因 $state 未定义炸。
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
