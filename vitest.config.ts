// 单独的 vitest 配置，不污染 vite.config.ts 的 dev server 配置。
// 只跑 src 下 *.test.ts；不需要 svelte plugin（我们只测纯 .ts 函数）。
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
