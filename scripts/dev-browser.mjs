// Launch the self-contained rssh-server and print the URL to open in Chrome.
// The server serves BOTH the embedded frontend (HTTP) and the IPC (ws) on one
// port — no vite needed; this exercises exactly what the IDEA plugin ships.
//
//     node scripts/dev-browser.mjs
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const server = spawn(
    "cargo",
    ["run", "--quiet", "--manifest-path", "src-tauri/Cargo.toml", "--features", "server", "--bin", "rssh-server"],
    { stdio: ["ignore", "pipe", "inherit"] },
);

createInterface({ input: server.stdout }).on("line", (line) => {
    let info;
    try { info = JSON.parse(line); } catch { return; }
    if (info?.port && info?.token) {
        const url = `http://127.0.0.1:${info.port}/?rsshPort=${info.port}&rsshToken=${info.token}`;
        console.log(`\n  ▶ rssh ready (self-contained, port ${info.port}).`);
        console.log(`  ▶ Open in Chrome:\n\n      ${url}\n`);
    }
});

const shutdown = () => {
    try { server.kill(); } catch {}
    process.exit(0);
};
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
