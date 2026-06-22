<script lang="ts">
    import {onDestroy, onMount, untrack} from "svelte";
    import {SvelteSet} from "svelte/reactivity";
    import {Terminal, type IDisposable} from "@xterm/xterm";
    import {FitAddon} from "@xterm/addon-fit";
    import {SearchAddon} from "@xterm/addon-search";
    import {Unicode11Addon} from "@xterm/addon-unicode11";
    import {ImageAddon} from "@xterm/addon-image";
    import {invoke} from "@tauri-apps/api/core";
    import {listen, type UnlistenFn} from "@tauri-apps/api/event";
    import type {HighlightRule} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import * as ai from "../ai/store.svelte.ts";
    import * as theme from "../themes/store.svelte.ts";
    import MobileKeybar from "./MobileKeybar.svelte";
    import {registerRsshOscHandlers} from "../osc/handler.ts";
    import {createCommandBlockTracker, type CommandBlock, type CommandBlockTracker} from "../terminal/command-blocks.ts";
    import {createFoldStore, type FoldStore} from "../terminal/folds.ts";
    import {extractBlocksText} from "../terminal/block-content.ts";
    import {setupTouchScroll} from "../terminal/touch-scroll.ts";
    import {renderBlocksToBlob} from "../terminal/block-to-image.ts";
    import {inputNewline, normalizeIncoming, bytesToHex, parseHexInput, parseLoginScript, remapEditingKeys, normalizeOutgoing, type LoginStep} from "../terminal/serial-transforms.ts";
    import {compileHighlightRules, type CompiledHighlightRule} from "../terminal/highlight.ts";
    import {HighlightDecorator} from "../terminal/highlight-decorations.ts";
    import {t} from "../i18n/index.svelte.ts";
    import {ACTIONS, matchBinding, type ActionId} from "../keyboard/keymap.ts";
    import * as keymap from "../stores/keymap.svelte.ts";
    import BlockContextMenu, {type MenuItem} from "./BlockContextMenu.svelte";

    let hlRules = $state<HighlightRule[]>([]);
    let hlCompiled = $state<CompiledHighlightRule[]>([]);
    let hlEverLoaded = false;
    let highlightDecorator: HighlightDecorator | undefined;

    function buildCompiledRules(rules: HighlightRule[]) {
        hlCompiled = compileHighlightRules(rules);
    }

    // React to HighlightManager edits. The store's `highlightsRevision`
    // counter is bumped on add/remove/reset; we then re-read the rules
    // and recompile. `hlEverLoaded` skips the very first run — the initial
    // load happens in onMount so the effect would otherwise double-load.
    // This must sit at script top-level (not inside the async onMount)
    // because $effect added after an `await` is not tracked by Svelte 5.
    $effect(() => {
        app.highlightsRevision();
        if (!hlEverLoaded) return;
        void (async () => {
            try {
                hlRules = await app.loadHighlights();
                buildCompiledRules(hlRules);
            } catch { /* DB read failure is non-fatal — old rules stay */ }
        })();
    });

    // Push compiled rules to the decoration layer whenever they change. The
    // decorator repaints the visible buffer, so an edit re-highlights existing
    // scrollback too — something the old write-time byte rewrite could never do.
    // Read hlCompiled UNCONDITIONALLY first: `decorator?.setRules(hlCompiled)`
    // short-circuits the whole expression (incl. the argument) while the
    // decorator is still undefined on first run, so hlCompiled would never be
    // tracked and the effect would never re-fire when rules load.
    $effect(() => {
        const rules = hlCompiled;
        highlightDecorator?.setRules(rules);
    });

    let {tabId, tabType, meta = {}}: {
        tabId: string;
        tabType: "ssh" | "local" | "serial";
        meta: Record<string, string>;
    } = $props();

    let containerEl: HTMLDivElement;
    let searchInputEl: HTMLInputElement;

    type AuthPromptData = { name: string; instructions: string; prompts: { prompt: string; echo: boolean }[] };
    let authPrompt = $state<AuthPromptData | null>(null);
    let authValues = $state<string[]>([]);

    function submitAuth() {
        if (!authPrompt) return;
        invoke("ssh_auth_respond", { tabId, responses: authValues });
        authPrompt = null;
        authValues = [];
    }

    /** 通用终端 prompt：临时接管 xterm onData，回车提交、Ctrl-C 取消、退格删字。
     *  - echo=false：输入不回显（passphrase 等敏感字段，sudo 风格）
     *  - echo=true：输入回显并支持 backspace 视觉退格（host key yes/no 等）
     *  调用方负责自己的 disposable 槽位（避免并发 prompt 互相覆盖）。 */
    function beginTerminalPrompt(
        promptText: string,
        opts: { echo: boolean; onSubmit: (value: string) => void; onCancel: () => void },
    ): IDisposable {
        terminal.write(`\r\n${promptText}`);

        let buffer = "";
        let done = false;
        let disposable: IDisposable | undefined;
        const finish = (action: "submit" | "cancel") => {
            if (done) return;
            done = true;
            disposable?.dispose();
            disposable = undefined;
            if (action === "submit") {
                terminal.write("\r\n");
                opts.onSubmit(buffer);
            } else {
                terminal.write("^C\r\n");
                opts.onCancel();
            }
            buffer = "";
        };

        disposable = terminal.onData((data: string) => {
            for (const ch of data) {
                const code = ch.charCodeAt(0);
                if (ch === "\r" || ch === "\n") { finish("submit"); return; }
                if (ch === "\x03") { finish("cancel"); return; }   // Ctrl-C
                if (ch === "\x7f" || ch === "\b") {                // Backspace / DEL
                    if (buffer.length > 0) {
                        buffer = buffer.slice(0, -1);
                        if (opts.echo) terminal.write("\b \b");
                    }
                    continue;
                }
                if (code < 0x20) continue;                          // 其他控制字符忽略
                buffer += ch;
                if (opts.echo) terminal.write(ch);
            }
        });
        return disposable!;
    }

    /** 终端内 passphrase 输入。监听后端 ssh:passphrase_prompt 事件触发。 */
    let passphraseInputDisposable: IDisposable | undefined;
    function beginPassphrasePrompt(promptText: string) {
        passphraseInputDisposable?.dispose();
        passphraseInputDisposable = beginTerminalPrompt(promptText, {
            echo: false,
            onSubmit: (v) => { invoke("ssh_passphrase_respond", { tabId, passphrase: v }); },
            onCancel: () => { invoke("ssh_passphrase_cancel", { tabId }); },
        });
    }

    /** 终端内 host key TOFU 确认（OpenSSH 风格）。监听 ssh:host_key_prompt 触发。
     *  yes/no/指纹 均不是秘密，需要回显让用户确认自己的输入。 */
    let hostKeyInputDisposable: IDisposable | undefined;
    function beginHostKeyPrompt(banner: string) {
        hostKeyInputDisposable?.dispose();
        hostKeyInputDisposable = beginTerminalPrompt(banner, {
            echo: true,
            onSubmit: (v) => { invoke("ssh_host_key_respond", { tabId, answer: v }); },
            onCancel: () => { invoke("ssh_host_key_cancel", { tabId }); },
        });
    }

    let terminal: Terminal;
    let fitAddon: FitAddon;
    let searchAddon: SearchAddon;
    let sessionId = $state<string | null>(null);
    let disconnected = $state(false);
    let showSearch = $state(false);
    let searchQuery = $state("");

    // Command block overlay state. `paintTick` is a dumb counter we bump
    // whenever something that affects the overlay changes — scroll, render,
    // block list change. The $derived below recomputes svg rects from it.
    let blockTracker: CommandBlockTracker | undefined;
    let foldStore: FoldStore | undefined;
    let paintTick = $state(0);
    let isAltBuffer = $state(false);

    // 右键菜单状态。null = 不显示。
    type CtxMenu = { x: number; y: number; items: MenuItem[] };
    let ctxMenu = $state<CtxMenu | null>(null);

    type BlockRect = {
        id: number;
        y: number;
        h: number;
        color: string;
        startLine: number;
        endLine: number;
        folded: boolean;
        foldCount: number; // 仅 folded=true 有效
    };

    const blockRects = $derived.by((): BlockRect[] => {
        paintTick; // dependency
        // selectedBlockIds 是 SvelteSet，下方 .has() 调用自带 reactivity
        if (!app.commandBlockBar()) return [];
        if (!terminal || !blockTracker || !containerEl || isAltBuffer) return [];
        const firstRow = containerEl.querySelector(".xterm-rows")?.firstElementChild as HTMLElement | null;
        const rowHeight = firstRow?.offsetHeight ?? 0;
        if (!rowHeight) return [];
        const buf = terminal.buffer.active;
        const viewportY = buf.viewportY;
        const rows = terminal.rows;
        // For an unfinished block, its tail is wherever the cursor currently
        // sits (absolute row = baseY + cursorY). This grows naturally as the
        // shell writes output and stops at the real last line — not at the
        // bottom of the viewport.
        const cursorAbs = buf.baseY + buf.cursorY;
        const out: BlockRect[] = [];
        for (const b of blockTracker.blocks) {
            if (b.start.isDisposed) continue;
            const folded = foldStore?.isFolded(b.id) ?? false;
            // Folded：竖线只标 prompt 行，body 已不在 buffer。
            // Unfolded：常规 [start..end] 跨度。
            const startLine = b.start.line;
            const endLine = folded
                ? b.start.line
                : b.end && !b.end.isDisposed ? b.end.line : cursorAbs;
            const top = Math.max(startLine, viewportY);
            const bot = Math.min(endLine, viewportY + rows - 1);
            if (top > bot) continue;
            const fold = folded ? foldStore?.getFold(b.id) : undefined;
            out.push({
                id: b.id,
                y: (top - viewportY) * rowHeight,
                h: (bot - top + 1) * rowHeight,
                color: b.color,
                startLine,
                endLine,
                folded,
                foldCount: fold?.count ?? 0,
            });
        }
        return out;
    });

    // Block 选中集合 — Finder 风格多选，halo 是它的视觉投影。
    //   单击          → 排他选中 + 顺带选中文本（让 Cmd+C 直接复制单块输出）
    //   Shift+click   → 锚点 .. 目标范围（清旧、填范围、anchor 不动、不动文本选区）
    //   Cmd/Ctrl+click → toggle 该块（anchor 移到此块、不动文本选区）
    //
    // 必须用 SvelteSet：原生 Set 在 $state 里 add/delete 不会让消费者
    // ($derived、模板表达式) 重算 —— Svelte 5 不给原生 Set 自动加代理。
    const selectedBlockIds = new SvelteSet<number>();

    // 染色：右键"染色"打开的块集合。会话级、非持久——块被 scrollback GC 掉，
    // 染色跟着消失（块本身就是临时的，给它造持久层是解决不存在的问题）。
    // 不变式：块 GC 时必须从这里剪枝，否则新块复用旧 id 会"鬼染色"。
    // 剪枝逻辑挂在 blockTracker.onChange，跟 selectedBlockIds 同一处。
    const coloredBlockIds = new SvelteSet<number>();
    let selectionAnchorId: number | null = null;

    // 自动染色：开关开着时，每个新块一出现就自动染（等价于自动右键"染色"）。
    // 块 id 单调递增、永不复用，用一个水位线整数记"已自动处理到哪"，保证每块只自动
    // 染一次——右键"取消染色"删掉的块 id 在水位线之下，不会被下一帧重新染回来。
    // 中途打开开关时，存量块（id > 水位线）也会被一并染上。
    let autoColoredHwm = 0;
    // blockListRevision 只在块集合真正增删时自增（见 blockTracker.onChange），不像
    // paintTick 每次滚动/渲染都跳 —— 否则开着自动染色时每帧都要空扫一遍所有块。
    let blockListRevision = $state(0);
    $effect(() => {
        blockListRevision; // 仅块集合变化时重算
        if (!app.autoColorBlocks() || !blockTracker) return;
        let maxId = autoColoredHwm;
        for (const b of blockTracker.blocks) {
            if (b.id > autoColoredHwm) coloredBlockIds.add(b.id);
            if (b.id > maxId) maxId = b.id;
        }
        autoColoredHwm = maxId;
    });

    function handleBlockClick(r: BlockRect, ev: MouseEvent) {
        if (ev.shiftKey) rangeSelectTo(r);
        else if (ev.metaKey || ev.ctrlKey) toggleSelect(r);
        else singleSelect(r);
    }

    function singleSelect(r: BlockRect) {
        selectedBlockIds.clear();
        selectedBlockIds.add(r.id);
        selectionAnchorId = r.id;
        // 顺带选中文本——单块复制是最高频场景，让 Cmd+C 直接走通
        terminal?.selectLines(r.startLine, r.endLine);
    }

    function toggleSelect(r: BlockRect) {
        if (selectedBlockIds.has(r.id)) selectedBlockIds.delete(r.id);
        else selectedBlockIds.add(r.id);
        selectionAnchorId = r.id;
    }

    function rangeSelectTo(r: BlockRect) {
        // 没 anchor 时退化为单击——Finder 同款行为
        if (selectionAnchorId === null || !blockTracker) {
            singleSelect(r);
            return;
        }
        const lo = Math.min(selectionAnchorId, r.id);
        const hi = Math.max(selectionAnchorId, r.id);
        selectedBlockIds.clear();
        for (const b of blockTracker.blocks) {
            if (b.id >= lo && b.id <= hi) selectedBlockIds.add(b.id);
        }
        // anchor 不动：shift 是"扩展"，不重置锚点
    }

    function clearBlockSelection() {
        if (selectedBlockIds.size > 0) selectedBlockIds.clear();
        selectionAnchorId = null;
    }

    function onWindowMouseDown(e: MouseEvent) {
        if (selectedBlockIds.size === 0) return;
        const t = e.target as Element | null;
        if (!t) return;
        // 点 bar 自身或菜单内部不清空——分别由 toggle / 菜单 action 处理。
        if (t.closest(".block-hit") || t.closest(".block-menu")) return;
        clearBlockSelection();
    }

    function onWindowKeyDown(e: KeyboardEvent) {
        if (e.key === "Escape" && selectedBlockIds.size > 0) {
            clearBlockSelection();
            // 不 preventDefault：Esc 仍要送到 shell（vim/less 等需要）。
        }
    }

    /** Finder 规则：右键对象 ∈ 已选集 → 作用于全部已选；否则 → 仅作用于该块。
     *  用于"复制"类操作。fold 仍是 per-block 动作，不走这条。 */
    function copyTargetBlocks(rightClickedId: number): CommandBlock[] {
        if (!blockTracker) return [];
        const ids = selectedBlockIds.has(rightClickedId)
            ? new Set(selectedBlockIds)
            : new Set([rightClickedId]);
        return blockTracker.blocks.filter((b) => ids.has(b.id));
    }

    async function copyBlocksAsText(blocks: CommandBlock[]) {
        if (!terminal || blocks.length === 0) return;
        // 传 foldStore：折叠块走 saved body，否则会被拉到 cursorAbs 把后续
        // 命令输出全卷进来（PR #24 reviewer 发现的 bug）
        const text = extractBlocksText(terminal, blocks, foldStore);
        if (!text) return;
        // arboard (not navigator.clipboard) so this process — not WebKitGTK —
        // owns the X11 CLIPBOARD selection. Otherwise a later arboard-based
        // paste (clipboard_read) deadlocks on its own WebView and times out.
        await app.writeClipboard(text);
        clearBlockSelection();
    }

    function copyBlocksAsImage(blocks: CommandBlock[]) {
        if (!terminal || blocks.length === 0) return;
        // 特性检测：某些 WebView/旧浏览器没有 ClipboardItem 或 clipboard.write，
        // `new ClipboardItem(...)` 会同步 throw ReferenceError，那是发生在
        // .catch() 前的，会冒泡到 click handler 把 UI 搞炸。先 bail out。
        if (typeof ClipboardItem === "undefined" || !navigator.clipboard?.write) {
            console.warn("copy image: ClipboardItem / clipboard.write unavailable");
            return;
        }
        // 关键：clipboard.write 必须**同步**地在 click handler 里调用，
        // ClipboardItem 接受 Promise<Blob> 由浏览器自己等。中间任何 await
        // 都会让 user gesture 失效 → NotAllowedError。
        const term = terminal;
        const fs = foldStore;
        try {
            const pngPromise = renderBlocksToBlob(term, blocks, {}, fs).then((b) => {
                if (!b) throw new Error("render produced no blob");
                return b;
            });
            navigator.clipboard
                .write([new ClipboardItem({ "image/png": pngPromise })])
                .then(() => clearBlockSelection())
                .catch((e) => console.warn("copy image failed:", e));
        } catch (e) {
            // ClipboardItem 构造或 clipboard.write 调用本身的 sync throw 兜底
            console.warn("copy image failed (sync):", e);
        }
    }

    /** 把目标块的内容塞进 AI 输入框（不发送）。打开面板 + prefill，用户过目再发。
     *  面板绑定 activeTabId，而右键的终端就是 activeTab，所以 tabId 必然对得上。 */
    function sendBlocksToAi(blocks: CommandBlock[]) {
        if (!terminal || blocks.length === 0) return;
        const text = extractBlocksText(terminal, blocks, foldStore);
        if (!text) return;
        ai.openPanel();
        ai.prefillInput(tabId, text);
        clearBlockSelection();
    }

    function openBlockMenu(r: BlockRect, e: MouseEvent) {
        e.preventDefault();
        e.stopPropagation();
        if (!foldStore || !blockTracker) return;
        const folded = r.folded;
        // 折叠仅对"已关闭、有 body"的 block 可用。展开总是可用（能进菜单
        // 说明 fold 记录还在，对应 block.start 仍存活）。
        const block = blockTracker.blocks.find((b) => b.id === r.id);
        const canFold = !!block && !!block.end && !block.end.isDisposed
                          && block.start.line + 1 <= block.end.line;
        // Finder 规则确定"复制"目标。snapshot：菜单弹出那一刻的选区
        // 决定操作对象，后续 selectedBlockIds 变化不影响 onClick 闭包。
        const targets = copyTargetBlocks(r.id);
        const n = targets.length;
        const textLabel = n > 1
            ? t("terminal.block.menu.copy_text_n", { n })
            : t("terminal.block.menu.copy_text");
        const imageLabel = n > 1
            ? t("terminal.block.menu.copy_image_n", { n })
            : t("terminal.block.menu.copy_image");
        // 染色是 per-block 状态 toggle（像 fold），不走 Finder 多选——标签随右键块的染色态翻转。
        const colored = coloredBlockIds.has(r.id);
        // 发送到 AI 走 Finder 多选（像复制）。未配 API key → 置灰。
        const aiConfigured = ai.settings()?.has_api_key === true;
        const sendAiLabel = n > 1
            ? t("terminal.block.menu.send_ai_n", { n })
            : t("terminal.block.menu.send_ai");
        ctxMenu = {
            x: e.clientX,
            y: e.clientY,
            items: [
                {
                    label: t(folded ? "terminal.block.menu.unfold" : "terminal.block.menu.fold"),
                    disabled: !folded && !canFold,
                    action: () => {
                        if (folded) foldStore!.unfold(r.id);
                        else foldStore!.fold(r.id);
                    },
                },
                {
                    label: t(colored ? "terminal.block.menu.uncolor" : "terminal.block.menu.color"),
                    action: () => {
                        // SvelteSet 的 add/delete 会让模板里的 .has(r.id) 重算 → 染色层即时增删。
                        if (coloredBlockIds.has(r.id)) coloredBlockIds.delete(r.id);
                        else coloredBlockIds.add(r.id);
                    },
                },
                {
                    label: textLabel,
                    disabled: n === 0,
                    action: () => copyBlocksAsText(targets),
                },
                {
                    label: imageLabel,
                    disabled: n === 0,
                    action: () => copyBlocksAsImage(targets),
                },
                {
                    label: sendAiLabel,
                    disabled: n === 0 || !aiConfigured,
                    action: () => sendBlocksToAi(targets),
                },
            ],
        };
    }

    // Listener tracking — disposed on cleanup/reconnect
    let unlisteners: UnlistenFn[] = [];
    let dataDisposable: IDisposable | undefined;
    let resizeDisposable: IDisposable | undefined;
    let reconnectDisposable: IDisposable | undefined;
    let resizeObs: ResizeObserver;
    let mobileKeyboardCleanup: (() => void) | undefined;
    let mobileTouchScrollCleanup: (() => void) | undefined;

    const isLocal = $derived(tabType === "local");
    const isSsh = $derived(tabType === "ssh");
    // Transport table — the per-tab byte-stream IPC contract lives in DATA, not in
    // branches. Adding a transport (telnet, …) is one more row, zero code change.
    // resize:null means the transport has no rows/cols (serial) → callers skip it.
    const TRANSPORT: Record<"ssh" | "local" | "serial", {
        write: string; resize: string | null; data: string; close: string; closeCmd: string;
    }> = {
        ssh:    { write: "ssh_write",    resize: "ssh_resize", data: "ssh:data",    close: "ssh:close",    closeCmd: "ssh_disconnect" },
        local:  { write: "pty_write",    resize: "pty_resize", data: "pty:data",    close: "pty:close",    closeCmd: "pty_close" },
        serial: { write: "serial_write", resize: null,         data: "serial:data", close: "serial:close", closeCmd: "serial_close" },
    };
    const writeCmd = $derived(TRANSPORT[tabType].write);
    const resizeCmd = $derived(TRANSPORT[tabType].resize);
    const dataEvent = $derived(TRANSPORT[tabType].data);
    const closeEvent = $derived(TRANSPORT[tabType].close);
    const closeCmd = $derived(TRANSPORT[tabType].closeCmd);

    // ── Serial (Tabby-style) input/output transforms. Driven by the saved
    //    profile's meta; null for ssh/local so those paths are untouched. ──
    const serialOpts = $derived(tabType === "serial" ? {
        inputNewline: meta.input_newline || "cr",
        outputNewline: meta.output_newline || "raw",
        localEcho: meta.local_echo === "true",
        backspace: meta.backspace || "del",
        slowSend: meta.slow_send === "true",
        inputMode: meta.input_mode || "normal",
        outputMode: meta.output_mode || "text",
        loginScript: meta.login_script || "",
    } : null);

    function serialInputNewline(): string {
        return inputNewline(serialOpts?.inputNewline ?? "cr");
    }
    function serialNormalizeOut(text: string): string {
        return normalizeIncoming(text, serialOpts?.outputNewline ?? "raw");
    }
    function serialSendBytes(bytes: number[]) {
        if (!sessionId || disconnected || !bytes.length) return;
        if (!serialOpts?.slowSend) {
            invoke(writeCmd, { sessionId, data: bytes }).catch(() => {});
            return;
        }
        // Slow devices / bootloaders: one byte at a time, ~5ms apart.
        const sid = sessionId;
        let i = 0;
        const tick = () => {
            // Stop when finished, disconnected, OR the session was swapped out by a
            // fast reconnect — otherwise queued ticks keep writing to the stale sid.
            // .catch swallows the reject that a closed session raises mid-loop.
            if (i >= bytes.length || disconnected || sessionId !== sid) return;
            invoke(writeCmd, { sessionId: sid, data: [bytes[i]] }).catch(() => {});
            i += 1;
            setTimeout(tick, 5);
        };
        tick();
    }
    function serialSendText(text: string) {
        serialSendBytes(Array.from(new TextEncoder().encode(text)));
    }

    // Cap local input buffers (hex / line editor) so a pathological no-newline
    // paste can't grow them without bound; reset on (re)connect (connectAndWire).
    const SERIAL_INPUT_CAP = 8192;

    // Hex input mode: accumulate typed hex (echoed), flush bytes on Enter.
    let serialHexBuf = "";
    function serialHexInput(data: string) {
        for (const ch of data) {
            if (ch === "\r" || ch === "\n") {
                const bytes = parseHexInput(serialHexBuf);
                serialHexBuf = "";
                terminal.write("\r\n");
                serialSendBytes(bytes);
            } else if (ch === "\x7f" || ch === "\b") {
                if (serialHexBuf) { serialHexBuf = serialHexBuf.slice(0, -1); terminal.write("\b \b"); }
            } else if (/[0-9a-fA-F ]/.test(ch)) {
                if (serialHexBuf.length < SERIAL_INPUT_CAP) { serialHexBuf += ch; terminal.write(ch); }
            }
        }
    }

    // Line-editor mode: buffer + locally echo input, send the whole line on
    // Enter. For half-duplex / non-echoing devices. Mirrors serialHexInput.
    // Escape sequences (arrows / Fn keys) are dropped — no in-line cursor nav.
    let serialLineBuf = "";
    function serialLineInput(data: string) {
        if (data.charCodeAt(0) === 0x1b) return;
        for (const ch of data) {
            if (ch === "\r" || ch === "\n") {
                terminal.write("\r\n");
                serialSendText(serialLineBuf + serialInputNewline());
                serialLineBuf = "";
            } else if (ch === "\x7f" || ch === "\b") {
                if (serialLineBuf) { serialLineBuf = serialLineBuf.slice(0, -1); terminal.write("\b \b"); }
            } else if (ch >= " ") {
                if (serialLineBuf.length < SERIAL_INPUT_CAP) { serialLineBuf += ch; terminal.write(ch); }
            }
        }
    }

    function serialOnData(data: string) {
        // Local editors (hex / line) own their backspace and never emit the
        // device-backspace byte, so dispatch to them BEFORE the remap below.
        if (serialOpts?.inputMode === "hex") { serialHexInput(data); return; }
        if (serialOpts?.inputMode === "line") { serialLineInput(data); return; }
        // Remap the editing keys (Backspace 0x7f / Delete CSI 3~) to the
        // device's delete byte. del mode leaves both as-is (VT/readline peer);
        // bs/csi3 make BOTH keys emit the one byte the device knows, so Delete
        // deletes instead of echoing a stray `~`.
        data = remapEditingKeys(data, serialOpts?.backspace ?? "del");
        // Convert EVERY line break to the device EOL — handles the single Enter
        // key AND multi-char chunks (native paste / IME), which the old
        // `data === "\r"` check passed through raw. Echo with CRLF so multi-line
        // input renders correctly in xterm.
        data = normalizeOutgoing(data, serialOpts?.inputNewline ?? "cr");
        if (serialOpts?.localEcho) terminal.write(data.replace(/\r\n|\r|\n/g, "\r\n"));
        serialSendText(data);
    }

    // ── Login script (expect / send), run on connect. ──
    let loginSteps: LoginStep[] = [];
    let loginStepIdx = 0;
    let loginBuf = "";
    const loginDecoder = new TextDecoder("utf-8");
    function initLoginScript() {
        loginSteps = parseLoginScript(serialOpts?.loginScript ?? "");
        loginStepIdx = 0;
        loginBuf = "";
        runLoginSends();
    }
    /** Fire consecutive `send` steps until the next `expect` (or the end). */
    function runLoginSends() {
        while (loginStepIdx < loginSteps.length && loginSteps[loginStepIdx].kind === "send") {
            serialSendText(loginSteps[loginStepIdx].text + serialInputNewline());
            loginStepIdx += 1;
        }
    }
    function feedLoginScript(raw: Uint8Array) {
        if (loginStepIdx >= loginSteps.length) return;
        const step = loginSteps[loginStepIdx];
        if (step.kind !== "expect") return;
        // Dedicated decoder (not the display one) so we don't corrupt its stream
        // state by decoding the same bytes twice. Cap the buffer.
        loginBuf = (loginBuf + loginDecoder.decode(raw, { stream: true })).slice(-4096);
        if (loginBuf.includes(step.text)) {
            loginBuf = "";
            loginStepIdx += 1;
            runLoginSends();
        }
    }

    /** Inject user text as input (snippet / broadcast, and the serial paste
     *  path). Serial: convert every line break to the device's configured EOL
     *  and honor slow-send. ssh/local: write raw — the PTY owns its own line
     *  discipline. Control sequences (arrows / Esc / Tab) must NOT come through
     *  here — they go raw via the registered terminal writer (writePty). */
    function sendText(text: string) {
        if (!text || disconnected || !sessionId) return;
        if (tabType === "serial") {
            serialSendText(normalizeOutgoing(text, serialOpts?.inputNewline ?? "cr"));
            return;
        }
        invoke(writeCmd, { sessionId, data: Array.from(new TextEncoder().encode(text)) });
    }

    function pasteText(text: string) {
        if (!text || disconnected || !sessionId) return;
        // Serial has no bracketed paste and speaks the device's EOL — that's
        // exactly sendText's job, so reuse it (newline transform + slow-send).
        if (tabType === "serial") { sendText(text); return; }
        // Collapse every line break to a single CR before sending: the PTY's
        // ICRNL turns each CR into one \n, so a raw CRLF would double (#98).
        // (xterm's prepareTextForTerminal does this; we bypass terminal.paste().)
        const normalized = text.replace(/\r?\n/g, "\r");
        const wrapped = terminal.modes.bracketedPasteMode
            ? `\x1b[200~${normalized}\x1b[201~` : normalized;
        invoke(writeCmd, { sessionId, data: Array.from(new TextEncoder().encode(wrapped)) });
    }

    function copySelection() {
        const sel = terminal.getSelection();
        if (sel) app.writeClipboard(sel);
    }

    /** Copy-on-select: fires on a real left-button mouse release on the terminal
     *  host — covers drag, double- and triple-click. Bound to mouseup (NOT
     *  terminal.onSelectionChange) so programmatic selections never hit the
     *  clipboard: xterm fires onSelectionChange for selectLines()/selectAll()
     *  too, and the block bar is a sibling subtree whose clicks don't bubble
     *  here. The toggle is read live. */
    function onSelectMouseUp(e: MouseEvent) {
        if (e.button !== 0 || !app.copyOnSelect()) return;
        if (terminal.hasSelection()) copySelection();
    }

    /** Terminal-area right-click, per app.rightClickAction():
     *  - "menu": do nothing — let xterm's handler + the native system menu run
     *    (this is the default "current menu").
     *  - "paste": paste the clipboard.
     *  - "copyPaste": copy the selection (if any) and clear it, else paste.
     *
     *  Registered in CAPTURE phase on the host so it runs BEFORE xterm's own
     *  contextmenu handler (which is on a descendant, bubble phase). For the
     *  non-menu modes we stopPropagation so xterm's rightClickHandler never runs:
     *  on macOS it focuses the hidden <textarea> and selects a word, and that
     *  editable-field focus is what makes WKWebView pop the native menu even
     *  after preventDefault. Stopping it first lets preventDefault actually
     *  suppress the menu. */
    function onTerminalContextMenu(e: MouseEvent) {
        if (app.isMobile) return; // mobile keeps the native long-press menu
        const action = app.rightClickAction();
        if (action === "menu") return; // let xterm + the native system menu through
        e.preventDefault();
        e.stopPropagation();
        if (action === "paste") {
            app.readClipboard().then(pasteText);
        } else if (terminal.hasSelection()) {
            copySelection();
            terminal.clearSelection();
        } else {
            app.readClipboard().then(pasteText);
        }
    }

    function openSearch() {
        showSearch = true;
        requestAnimationFrame(() => searchInputEl?.focus());
    }

    let _lastSearchN = 0;
    $effect(() => {
        const req = app.searchRequest();
        if (req && req.tabId === tabId && req.n !== _lastSearchN) {
            _lastSearchN = req.n;
            openSearch();
        }
    });

    function closeSearch() { showSearch = false; searchAddon?.clearDecorations(); terminal?.focus(); }
    function doSearch() { if (searchQuery) searchAddon?.findNext(searchQuery); }
    function searchNext() { searchAddon?.findNext(searchQuery); }
    function searchPrev() { searchAddon?.findPrevious(searchQuery); }

    // ─── Shared connect/wire helpers ───

    const decoder = new TextDecoder("utf-8");

    /** Wire Tauri event listeners for session data + close. */
    async function wireSessionEvents(sid: string) {
        unlisteners.push(await listen<number[]>(`${dataEvent}:${sid}`, (ev) => {
            const raw = new Uint8Array(ev.payload);
            if (serialOpts) {
                feedLoginScript(raw);
                if (serialOpts.outputMode === "hex") { terminal.write(bytesToHex(raw)); return; }
                terminal.write(serialNormalizeOut(decoder.decode(raw, { stream: true })));
                return;
            }
            // Write the raw bytes untouched — keyword highlighting is a decoration
            // layer over the parsed grid (HighlightDecorator), not a byte rewrite.
            terminal.write(raw);
        }));
        unlisteners.push(await listen(`${closeEvent}:${sid}`, () => {
            disconnected = true;
            // Serial: the port just died; free its backend handle NOW so the
            // exclusive OS port is released. Otherwise the reconnect below
            // re-opens the same path while the stale handle still owns it and
            // fails. ssh/local hold no exclusive OS resource, so they skip this.
            if (tabType === "serial") invoke("serial_close", { sessionId: sid }).catch(() => {});
            terminal.write("\r\n\x1b[31m--- Disconnected ---\x1b[0m\r\n");
            terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
            setupReconnect();
        }));
    }

    /** Register terminal input + resize handlers (disposes old ones first). */
    function wireSessionInput(sid: string) {
        dataDisposable?.dispose();
        resizeDisposable?.dispose();

        dataDisposable = terminal.onData((data: string) => {
            if (disconnected) return;
            if (serialOpts) { serialOnData(data); return; }
            invoke(writeCmd, { sessionId: sid, data: Array.from(new TextEncoder().encode(processInput(data))) });
        });
        resizeDisposable = terminal.onResize(({ cols, rows }) => {
            if (!disconnected && resizeCmd) invoke(resizeCmd, { sessionId: sid, cols, rows });
        });
    }

    /** Full connect cycle: spawn session, wire events + input. */
    async function connectAndWire(): Promise<boolean> {
        // Cleanup previous
        unlisteners.forEach(u => u());
        unlisteners = [];
        disconnected = false;
        sessionId = null;
        serialHexBuf = "";
        serialLineBuf = "";

        if (tabType === "serial") {
            try {
                sessionId = await invoke<string>("serial_open", {
                    port: meta.port,
                    config: {
                        baud_rate: Number(meta.baud_rate) || 115200,
                        data_bits: Number(meta.data_bits) || 8,
                        parity: meta.parity || "none",
                        stop_bits: Number(meta.stop_bits) || 1,
                        flow_control: meta.flow_control || "none",
                        xany: meta.xany === "true",
                    },
                });
            } catch (e: any) {
                terminal.write(`\x1b[31mSerial open failed: ${e}\x1b[0m\r\n`);
                terminal.write("\x1b[90mPress any key to retry.\x1b[0m\r\n");
                disconnected = true;
                return false;
            }
            await wireSessionEvents(sessionId);
            initLoginScript();
        } else if (isLocal) {
            try {
                sessionId = await invoke<string>("pty_spawn", { cols: terminal.cols, rows: terminal.rows });
            } catch (e: any) {
                terminal.write(`\x1b[31mLaunch failed: ${e}\x1b[0m\r\n`);
                return false;
            }
            await wireSessionEvents(sessionId);
        } else {
            // SSH: listen on tabId FIRST for connection logs + auth prompts
            const logUn = await listen<number[]>(`ssh:data:${tabId}`, (ev) => {
                terminal.write(new Uint8Array(ev.payload));
            });
            const authUn = await listen<AuthPromptData>(`ssh:auth_prompt:${tabId}`, (ev) => {
                authPrompt = ev.payload;
                authValues = ev.payload.prompts.map(() => "");
            });
            const passUn = await listen<{ prompt: string }>(`ssh:passphrase_prompt:${tabId}`, (ev) => {
                beginPassphrasePrompt(ev.payload.prompt);
            });
            const hkUn = await listen<{ banner: string }>(`ssh:host_key_prompt:${tabId}`, (ev) => {
                beginHostKeyPrompt(ev.payload.banner);
            });

            try {
                // GUI 路径下 meta.profileId 永远非空（所有 ssh tab 入口——HomeScreen /
                // osc handler / AppShell.connectPinned 都从 profile 出发）。直连参数
                // （host/username/auth_type/secret）的后端 stub 早已是 dead code，
                // 跟前端 invoke 一起收紧。
                sessionId = await invoke<string>("ssh_connect", {
                    profileId: meta.profileId,
                    logSessionId: tabId,
                    cols: terminal.cols, rows: terminal.rows,
                });
            } catch (e: any) {
                logUn(); authUn(); passUn(); hkUn();
                passphraseInputDisposable?.dispose(); passphraseInputDisposable = undefined;
                hostKeyInputDisposable?.dispose(); hostKeyInputDisposable = undefined;
                terminal.write(`\x1b[31mConnection failed: ${e}\x1b[0m\r\n`);
                terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
                disconnected = true;
                return false;
            }
            logUn(); authUn(); passUn(); hkUn();
            passphraseInputDisposable?.dispose(); passphraseInputDisposable = undefined;
            hostKeyInputDisposable?.dispose(); hostKeyInputDisposable = undefined;
            await wireSessionEvents(sessionId);
        }

        wireSessionInput(sessionId!);

        // Sync initial size
        requestAnimationFrame(() => {
            fitAddon.fit();
            if (sessionId && !disconnected && resizeCmd) {
                invoke(resizeCmd, { sessionId, cols: terminal.cols, rows: terminal.rows });
            }
        });

        // 远端 shell 探测（仅 SSH）。连接成功时跑——此刻 init_command 已由后端在
        // ssh_connect 返回前写入 PTY，探针排在其后。门控：auto_detect 开 + 该 profile
        // 进程缓存未命中（remoteShellProbeNeeded）。命中即写 profile 缓存，供 AI 会话
        // 启动时读初始 shell。fire-and-forget，不阻塞终端就绪；重连时缓存已命中 →
        // needed=false，不重复刷探针。
        if (isSsh) {
            const sid = sessionId!;
            void ai.remoteShellProbeNeeded(sid)
                .then((needed) => { if (needed) return ai.probeRemoteShell(sid); })
                .catch((e) => console.warn("[ai] connect-time shell probe skipped:", e));
        }

        return true;
    }

    function processInput(data: string): string {
        const ctrl = app.ctrlActive();
        const alt = app.altActive();
        if (!ctrl && !alt) return data;
        if (ctrl && data.length === 1) {
            const code = data.toUpperCase().charCodeAt(0);
            if (code >= 65 && code <= 90) data = String.fromCharCode(code - 64);
        }
        if (alt) data = '\x1b' + data;
        app.clearModifiers();
        return data;
    }

    function setupReconnect() {
        reconnectDisposable?.dispose();
        reconnectDisposable = terminal.onData(() => {
            if (!disconnected) return;
            reconnectDisposable?.dispose();
            reconnectDisposable = undefined;
            reconnect();
        });
    }

    async function reconnect() {
        terminal.write("\r\n\x1b[36mReconnecting ...\x1b[0m\r\n");
        const ok = await connectAndWire();
        setupReconnect();
        if (!ok) {
            disconnected = true;
        }
    }

    function setupMobileSoftKeyboard(helper: HTMLTextAreaElement) {
        const longPressMs = 360;
        const moveSlopPx = 12;
        const originalHelperStyle = helper.getAttribute("style");
        let scrollResetRaf = 0;
        let helperPinRaf = 0;
        let gesture: {
            pointerId: number;
            x: number;
            y: number;
            longPress: boolean;
            moved: boolean;
            timer: number | undefined;
        } | null = null;

        function resetDocumentScroll() {
            if (scrollResetRaf) return;
            scrollResetRaf = requestAnimationFrame(() => {
                scrollResetRaf = 0;
                if (window.scrollX !== 0 || window.scrollY !== 0) window.scrollTo(0, 0);
                document.documentElement.scrollTop = 0;
                document.documentElement.scrollLeft = 0;
                document.body.scrollTop = 0;
                document.body.scrollLeft = 0;
            });
        }

        function pinKeyboardHelper() {
            const viewport = window.visualViewport;
            const minTop = (viewport?.offsetTop ?? 0) + 1;
            const maxTop = minTop + (viewport?.height ?? window.innerHeight) - 2;
            const containerTop = containerEl.getBoundingClientRect().top + 8;
            const top = Math.max(minTop, Math.min(maxTop, containerTop));

            // xterm keeps this textarea far off-screen by default. On mobile
            // WebView, focusing it makes the page pan to reveal it, and the IME
            // composition/candidate UI anchors to its position — so off-screen
            // means a yanked page and a misplaced input popup. Keep it invisible
            // but in-view. (Layout-independent: not about any fixed chrome.)
            helper.style.position = "fixed";
            helper.style.left = "1px";
            helper.style.top = `${Math.round(top)}px`;
            helper.style.width = "1px";
            helper.style.height = "1px";
            helper.style.opacity = "0";
            helper.style.zIndex = "-1";
            helper.style.pointerEvents = "none";
            helper.style.caretColor = "transparent";
            helper.style.background = "transparent";
            helper.style.color = "transparent";
            helper.style.border = "0";
            helper.style.padding = "0";
            helper.style.margin = "0";
            helper.style.outline = "0";
            helper.style.resize = "none";
            helper.style.overflow = "hidden";
        }

        function onViewportChange() {
            if (document.activeElement !== helper) return;
            pinKeyboardHelper();
            resetDocumentScroll();
        }

        function onWindowScroll() {
            if (document.activeElement === helper) resetDocumentScroll();
        }

        function keepKeyboardHelperInView() {
            pinKeyboardHelper();
            resetDocumentScroll();
            if (helperPinRaf) cancelAnimationFrame(helperPinRaf);
            helperPinRaf = requestAnimationFrame(() => {
                helperPinRaf = 0;
                pinKeyboardHelper();
                resetDocumentScroll();
            });
        }

        function lockKeyboard() {
            helper.readOnly = true;
            helper.setAttribute("readonly", "true");
            helper.setAttribute("inputmode", "none");
            helper.tabIndex = -1;
            helper.inert = true;
        }

        function unlockKeyboard() {
            helper.inert = false;
            helper.readOnly = false;
            helper.removeAttribute("readonly");
            helper.setAttribute("inputmode", "text");
            helper.tabIndex = 0;
        }

        function showKeyboard() {
            pinKeyboardHelper();
            unlockKeyboard();
            helper.focus({ preventScroll: true });
            resetDocumentScroll();
        }

        function hideKeyboard() {
            helper.blur();
            lockKeyboard();
            resetDocumentScroll();
        }

        function clearGestureTimer() {
            if (gesture?.timer) {
                window.clearTimeout(gesture.timer);
                gesture.timer = undefined;
            }
        }

        function shouldHandleTouch(ev: PointerEvent) {
            return ev.pointerType === "touch" || ev.pointerType === "pen";
        }

        function onPointerDown(ev: PointerEvent) {
            if (!shouldHandleTouch(ev)) return;
            clearGestureTimer();
            gesture = {
                pointerId: ev.pointerId,
                x: ev.clientX,
                y: ev.clientY,
                longPress: false,
                moved: false,
                timer: undefined,
            };
            gesture.timer = window.setTimeout(() => {
                if (!gesture || gesture.pointerId !== ev.pointerId) return;
                gesture.longPress = true;
                hideKeyboard();
            }, longPressMs);
        }

        function onPointerMove(ev: PointerEvent) {
            if (!gesture || gesture.pointerId !== ev.pointerId) return;
            const dx = ev.clientX - gesture.x;
            const dy = ev.clientY - gesture.y;
            if (Math.hypot(dx, dy) <= moveSlopPx) return;
            gesture.moved = true;
            clearGestureTimer();
            hideKeyboard();
        }

        function onPointerUp(ev: PointerEvent) {
            if (!gesture || gesture.pointerId !== ev.pointerId) return;
            const shouldOpenKeyboard = !gesture.longPress && !gesture.moved;
            clearGestureTimer();
            gesture = null;
            if (shouldOpenKeyboard) showKeyboard();
            else lockKeyboard();
        }

        function onPointerCancel(ev: PointerEvent) {
            if (!gesture || gesture.pointerId !== ev.pointerId) return;
            clearGestureTimer();
            gesture = null;
            lockKeyboard();
        }

        function onContextMenu(_ev: Event) {
            // 长按定时器 (360ms) 通常已经先锁过键盘了，这里只是兜底。
            // 不要 preventDefault：那会连带掐掉系统的复制/粘贴菜单。
            hideKeyboard();
            // ev.preventDefault();
            // ev.stopImmediatePropagation();
        }

        function onBlur() {
            lockKeyboard();
        }

        pinKeyboardHelper();
        lockKeyboard();
        helper.blur();
        helper.addEventListener("blur", onBlur);
        helper.addEventListener("input", keepKeyboardHelperInView);
        helper.addEventListener("keydown", keepKeyboardHelperInView);
        helper.addEventListener("compositionstart", keepKeyboardHelperInView);
        helper.addEventListener("compositionupdate", keepKeyboardHelperInView);
        window.addEventListener("scroll", onWindowScroll, { passive: true });
        window.visualViewport?.addEventListener("scroll", onViewportChange, { passive: true });
        window.visualViewport?.addEventListener("resize", onViewportChange, { passive: true });
        containerEl.addEventListener("pointerdown", onPointerDown, { capture: true, passive: true });
        containerEl.addEventListener("pointermove", onPointerMove, { capture: true, passive: true });
        containerEl.addEventListener("pointerup", onPointerUp, { capture: true, passive: true });
        containerEl.addEventListener("pointercancel", onPointerCancel, { capture: true, passive: true });
        containerEl.addEventListener("contextmenu", onContextMenu, { capture: true });

        return () => {
            clearGestureTimer();
            if (scrollResetRaf) cancelAnimationFrame(scrollResetRaf);
            if (helperPinRaf) cancelAnimationFrame(helperPinRaf);
            if (originalHelperStyle === null) helper.removeAttribute("style");
            else helper.setAttribute("style", originalHelperStyle);
            helper.removeEventListener("blur", onBlur);
            helper.removeEventListener("input", keepKeyboardHelperInView);
            helper.removeEventListener("keydown", keepKeyboardHelperInView);
            helper.removeEventListener("compositionstart", keepKeyboardHelperInView);
            helper.removeEventListener("compositionupdate", keepKeyboardHelperInView);
            window.removeEventListener("scroll", onWindowScroll);
            window.visualViewport?.removeEventListener("scroll", onViewportChange);
            window.visualViewport?.removeEventListener("resize", onViewportChange);
            containerEl.removeEventListener("pointerdown", onPointerDown, { capture: true });
            containerEl.removeEventListener("pointermove", onPointerMove, { capture: true });
            containerEl.removeEventListener("pointerup", onPointerUp, { capture: true });
            containerEl.removeEventListener("pointercancel", onPointerCancel, { capture: true });
            containerEl.removeEventListener("contextmenu", onContextMenu, { capture: true });
        };
    }

    let unsubscribeTheme: (() => void) | null = null;
    let unsubscribeFont: (() => void) | null = null;

    onMount(async () => {
        terminal = new Terminal({
            cursorBlink: true,
            fontSize: theme.termFontSize(),
            fontFamily: theme.currentTermFontStack(),
            allowProposedApi: true,
            theme: theme.currentTermTheme(),
            // xterm 6 draws its own scrollbar at overviewRuler.width||14 (=14px),
            // out of reach of the global ::-webkit-scrollbar; pin it to the app's 6px.
            overviewRuler: { width: 6 },
        });
        // Listener fires immediately with current theme (already applied above)
        // and on every palette change. Keep the unsubscribe for onDestroy.
        unsubscribeTheme = theme.registerXtermThemeListener((t) => {
            if (terminal) terminal.options.theme = t;
        });
        fitAddon = new FitAddon();
        searchAddon = new SearchAddon();
        terminal.loadAddon(fitAddon);
        terminal.loadAddon(searchAddon);
        terminal.loadAddon(new Unicode11Addon());
        // SIXEL / iTerm IIP 图片协议。移动端把内存上限压一半。
        terminal.loadAddon(new ImageAddon({
            sixelSupport: true,
            sixelScrolling: true,
            iipSupport: true,
            storageLimit: app.isMobile ? 32 : 128,
            pixelLimit: app.isMobile ? 4_000_000 : 16_000_000,
        }));
        terminal.open(containerEl);
        terminal.unicode.activeVersion = "11";
        // Keyword highlighting lives here: a decoration layer over the parsed
        // cell grid. The reactive $effect above feeds it the compiled rules.
        highlightDecorator = new HighlightDecorator(terminal);
        fitAddon.fit();

        // Terminal font: the chosen family (prepended to the base stack) and
        // pixel size. Registered after open()+fit() because the immediate
        // callback refits, which needs fitAddon to exist. Both fields alter
        // cell metrics, so (unlike theme) we must refit after applying.
        unsubscribeFont = theme.registerXtermFontListener((font) => {
            if (!terminal) return;
            terminal.options.fontFamily = font.family;
            terminal.options.fontSize = font.size;
            fitAddon?.fit();
        });

        // 移动端：xterm 的 helper-textarea 一旦 focus 就会召系统键盘，
        // 而长按选择需要避开这个 focus。短按终端时解锁并 focus；长按、
        // 拖选或 contextmenu 时立刻锁回去。
        if (app.isMobile) {
            const helper = terminal.textarea ?? containerEl.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea");
            if (helper) {
                mobileKeyboardCleanup = setupMobileSoftKeyboard(helper);
            }
            // 与上面的 pointer 键盘状态机正交：靠"位移超阈值才接管"避开点按(弹键盘)和
            // 长按(选字)。代价：长按选中后再拖动会变成滚动而非扩展选区——移动端有
            // 块复制 / 发 AI 取文，这个取舍可接受。
            mobileTouchScrollCleanup = setupTouchScroll(containerEl, terminal);
        }

        app.registerTerminalControls(tabId, {
            getSelection: () => terminal.getSelection(),
            paste: pasteText,
            sendText,
            focus: () => terminal.focus(),
        });

        // Copy-on-select (left-button mouseup) + right-click action (capture
        // phase — required so preventDefault can suppress the native menu before
        // xterm/WebView handle the event). See onSelectMouseUp / onTerminalContextMenu.
        containerEl.addEventListener("mouseup", onSelectMouseUp);
        containerEl.addEventListener("contextmenu", onTerminalContextMenu, { capture: true });

        // App-level terminal shortcuts (search / SFTP / snippet / copy / paste).
        // Bindings are user-customizable (lib/keyboard/keymap.ts); read live so a
        // rebind takes effect immediately. We intercept a matched combo before xterm
        // forwards it to the PTY; everything else passes through to the shell.
        function runTerminalShortcut(id: ActionId) {
            switch (id) {
                case "term.search": openSearch(); break;
                case "term.sftp": app.navigate("sftp"); break;
                case "term.snippet": app.openSnippetPicker(); break;
                case "term.paste": app.readClipboard().then(pasteText); break;
                case "term.copy": copySelection(); break;
            }
        }
        terminal.attachCustomKeyEventHandler((e: KeyboardEvent) => {
            if (e.type !== "keydown") return true;
            for (const a of ACTIONS) {
                if (a.surface !== "terminal") continue;
                if (!matchBinding(e, keymap.binding(a.id))) continue;
                // SFTP only applies to remote sessions on desktop; elsewhere let the shell have the key.
                if (a.id === "term.sftp" && (!isSsh || app.isMobile)) return true;
                e.preventDefault();
                runTerminalShortcut(a.id);
                return false;
            }
            return true;
        });

        // OSC 7337: rssh CLI → app integration（处理逻辑见 lib/osc/handler.ts）。
        // 仅本地 PTY 注册：open:/fwd: 引用的是本地保存的 profile，远程 SSH/serial
        // 主机发这个序列在语义上只可能是攻击 —— 让客户端无确认地连到任意已保存
        // profile 或开端口转发。xterm 解析器分不清本地字节与经数据流到达的远程
        // 字节，所以唯一干净的防线是根本不在远程终端上挂这个 handler。
        if (isLocal) {
            registerRsshOscHandlers(terminal.parser, {
                error: (msg) => terminal?.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
            });
        }

        // Command block tracker — marks Enter keypresses in normal buffer.
        blockTracker = createCommandBlockTracker(terminal);
        blockTracker.onChange(() => {
            paintTick++;
            blockListRevision++;
            // 剪枝：被 GC 的块从所有引用它 id 的集合里清掉，anchor 失效也复位。
            if (!blockTracker) return;
            if (selectedBlockIds.size === 0 && coloredBlockIds.size === 0 && selectionAnchorId === null) return;
            const live = new Set(blockTracker.blocks.map(b => b.id));
            for (const id of selectedBlockIds) {
                if (!live.has(id)) selectedBlockIds.delete(id);
            }
            for (const id of coloredBlockIds) {
                if (!live.has(id)) coloredBlockIds.delete(id);
            }
            if (selectionAnchorId !== null && !live.has(selectionAnchorId)) {
                selectionAnchorId = null;
            }
        });

        // Fold store — splice-based fold/unfold with auto-cleanup on resize and
        // scrollback trim. See folds.ts for the invariant analysis.
        foldStore = createFoldStore(terminal, blockTracker);
        foldStore.onChange(() => paintTick++);
        terminal.onScroll(() => paintTick++);
        terminal.onRender(() => paintTick++);
        terminal.buffer.onBufferChange((buf) => {
            isAltBuffer = buf.type === "alternate";
            paintTick++;
        });

        // Load highlight rules + the command-block-bar toggle. Awaiting
        // the toggle before `connectAndWire` runs avoids a first-frame
        // flash of the bar when the user has it disabled.
        try { hlRules = await app.loadHighlights(); buildCompiledRules(hlRules); } catch {}
        hlEverLoaded = true;
        await app.loadCommandBlockBar();
        await app.loadAutoColorBlocks();

        // Connect
        await connectAndWire();
        setupReconnect();

        terminal.onTitleChange((title) => {
            if (!title) return;
            if (isLocal) app.updateTabLabel(tabId, title);
            else app.setTerminalTitle(tabId, title);
        });

        // 块选中清空：点击非 .block-hit / 非 .block-menu 处 → 清空荧光。
        // Esc → 清空荧光（不 preventDefault，让 Esc 仍传到 shell）。
        window.addEventListener("mousedown", onWindowMouseDown);
        window.addEventListener("keydown", onWindowKeyDown);

        resizeObs = new ResizeObserver((entries) => {
            // Skip fitting when the container is hidden (display:none
            // collapses dimensions to zero) — fitting at 0×0 corrupts
            // xterm's column count and causes the narrow-tab bug.
            const { width, height } = entries[0].contentRect;
            if (width > 0 && height > 0) fitAddon?.fit();
        });
        resizeObs.observe(containerEl);
    });

    // Register session in global registry for broadcast
    $effect(() => {
        if (sessionId && !disconnected) {
            const sid = sessionId;
            untrack(() => {
                app.registerSession({ tabId, sessionId: sid, type: tabType });
                // 若该 tab 有活跃 AI session 且绑的不是新 sid（== 重连后 target_id 换了），
                // rebind 到新 sid，让 AI actor 后续的 file_ops / SFTP 走新连接。
                // 首次连接 (sessionForTab 为 undefined) 直接 skip。
                const aiInfo = ai.sessionForTab(tabId);
                if (aiInfo && aiInfo.target_id !== sid && tabType !== "serial") {
                    ai.rebindTarget(tabId, tabType, sid).catch((e) =>
                        console.warn("[ai] rebind on reconnect:", e),
                    );
                }
            });
        } else {
            untrack(() => app.unregisterSession(tabId));
        }
    });

    // When the block-bar toggle flips, xterm's left padding changes — it
    // needs to refit so columns recompute. The refit triggers xterm's own
    // render, which fires onRender → paintTick++, so the overlay resyncs
    // without us writing paintTick here (writing it here would make this
    // effect self-dependent via `++`, causing an update loop).
    $effect(() => {
        app.commandBlockBar(); // subscribe
        fitAddon?.fit();
    });

    // Focus terminal + register writer when this tab becomes active.
    // Double-rAF: the first frame lets the browser apply layout after
    // the pane switches from display:none → flex; the second frame
    // ensures the computed dimensions are stable before we fit.
    $effect(() => {
        if (app.activeTabId() === tabId && !app.settingsActive()) {
            requestAnimationFrame(() => requestAnimationFrame(() => fitAddon?.fit()));
            terminal?.focus();
            const writePty = (text: string) => {
                if (sessionId && !disconnected) {
                    invoke(writeCmd, {sessionId, data: Array.from(new TextEncoder().encode(text))});
                }
            };
            app.registerTerminalWriter(writePty);
            // DECCKM: bare arrows use SS3 (ESC O x) in app-cursor mode, CSI (ESC [ x)
            // in normal mode. Modified arrows always use CSI with params — SS3 has
            // no param form. That's the protocol, not a design choice.
            app.registerTerminalArrowSender((dir, mod) => {
                const appMode = terminal.modes.applicationCursorKeysMode;
                const seq = mod === 0
                    ? (appMode ? `\x1bO${dir}` : `\x1b[${dir}`)
                    : `\x1b[1;${mod}${dir}`;
                writePty(seq);
            });
        }
    });

    onDestroy(() => {
        unsubscribeTheme?.();
        unsubscribeFont?.();
        window.removeEventListener("mousedown", onWindowMouseDown);
        window.removeEventListener("keydown", onWindowKeyDown);
        containerEl?.removeEventListener("mouseup", onSelectMouseUp);
        containerEl?.removeEventListener("contextmenu", onTerminalContextMenu, { capture: true });
        unlisteners.forEach(u => u());
        dataDisposable?.dispose();
        resizeDisposable?.dispose();
        reconnectDisposable?.dispose();
        // 关 tab 时若停在 prompt 阶段，主动取消让后端 connect 流程跳出，
        // 否则 ssh_connect 会在 worker 线程上挂着等用户输入。
        // 三类 prompt（auth / passphrase / host_key）都无脑发 cancel；后端 remove
        // 不存在的 key 是 no-op，幂等。
        if (passphraseInputDisposable) {
            invoke("ssh_passphrase_cancel", { tabId }).catch(() => {});
        }
        if (hostKeyInputDisposable) {
            invoke("ssh_host_key_cancel", { tabId }).catch(() => {});
        }
        invoke("ssh_auth_cancel", { tabId }).catch(() => {});
        passphraseInputDisposable?.dispose();
        hostKeyInputDisposable?.dispose();
        resizeObs?.disconnect();
        mobileKeyboardCleanup?.();
        mobileTouchScrollCleanup?.();
        foldStore?.dispose();
        blockTracker?.dispose();
        app.unregisterTerminalWriter();
        app.unregisterTerminalArrowSender();
        app.unregisterTerminalControls(tabId);
        app.unregisterSession(tabId);
        // Serial is exempt from the !disconnected guard: serial_close just drops
        // the backend map entry (idempotent), so always free it on a known
        // sessionId — otherwise an unplugged-then-closed serial tab leaks its
        // handle until window-close/reconcile sweeps it. ssh/local keep the guard
        // (disconnecting an already-dead session is pointless / may error).
        if (sessionId && (!disconnected || tabType === "serial")) {
            if (isSsh) {
                // 把 tabId 一并传给后端做防御性 waiters 清理；漏传不致命。
                invoke("ssh_disconnect", { sessionId, tabId }).catch(() => {});
            } else {
                // local PTY / serial：按 transport 选 close 命令（pty_close / serial_close）。
                invoke(closeCmd, { sessionId }).catch(() => {});
            }
        }
        highlightDecorator?.dispose();
        terminal?.dispose();
    });
</script>

<div class="term-outer">
    {#if showSearch}
        <div class="search-bar">
            <input
                    bind:this={searchInputEl}
                    type="text"
                    bind:value={searchQuery}
                    placeholder="Search..."
                    oninput={doSearch}
                    onkeydown={(e) => {
          if (e.key === "Enter") { e.shiftKey ? searchPrev() : searchNext(); }
          if (e.key === "Escape") closeSearch();
        }}
            />
            <button class="search-btn" onclick={searchPrev} title="Previous">&#x25B2;</button>
            <button class="search-btn" onclick={searchNext} title="Next">&#x25BC;</button>
            <button class="search-btn" onclick={closeSearch} title="Close">&times;</button>
        </div>
    {/if}
    {#if authPrompt}
        <div class="auth-overlay">
            <div class="auth-dialog">
                {#if authPrompt.name}<div class="auth-title">{authPrompt.name}</div>{/if}
                {#if authPrompt.instructions}<div class="auth-instructions">{authPrompt.instructions}</div>{/if}
                {#each authPrompt.prompts as p, i}
                    <label class="auth-label">
                        <span>{p.prompt}</span>
                        <input
                            type={p.echo ? "text" : "password"}
                            bind:value={authValues[i]}
                            onkeydown={(e) => { if (e.key === "Enter") submitAuth(); }}
                        />
                    </label>
                {/each}
                <button class="auth-submit" onclick={submitAuth}>Submit</button>
            </div>
        </div>
    {/if}
    <div class="term-wrap" class:is-mobile={app.isMobile} class:no-block-bar={!app.commandBlockBar()}>
        <div class="xterm-host" bind:this={containerEl}></div>
        {#if app.commandBlockBar()}
            <!-- 染色层：整行宽的半透明色块，用块自身的色条颜色。pointer-events:none
                 让点击/选中穿透到 xterm 与 .block-hit。坐标同 fold-label：block-bar
                 SVG 有 top:4px 偏移，这个 div 是 .term-wrap 子节点，要把那 4px 加回来。
                 排在 SVG 之前 → 色条画在染色之上，主条保持清晰。 -->
            {#each blockRects as r (r.id)}
                {#if coloredBlockIds.has(r.id)}
                    <div class="block-tint"
                         style="top: calc({r.y}px + 4px); height: {r.h}px; background: {r.color};"></div>
                {/if}
            {/each}
            <svg class="block-bar" aria-hidden="true">
                {#if isAltBuffer}
                    <rect x="5" y="0" width="3" height="100%" rx="1.5" style="fill: var(--text-dim)" opacity="0.5" />
                {:else}
                    {#each blockRects as r (r.id)}
                        {#if selectedBlockIds.has(r.id)}
                            <!-- Halo：选中时画在主条之下，加宽加高、半透明。
                                 实体矩形——视觉信号不依赖 filter 是否生效，
                                 在 WKWebView/Tauri 等对 SVG filter 支持差的
                                 环境也能稳定显示。blur 是锦上添花，没了也行。 -->
                            <rect class="block-halo"
                                  x="2" y={r.y - 2} width="9" height={r.h + 4} rx="3"
                                  fill={r.color} opacity="0.45" />
                        {/if}
                        {#if r.folded}
                            <!-- 虚线 stroke 表示折叠态；fill 透明让背景透出 -->
                            <rect x="5" y={r.y} width="3" height={r.h} rx="1.5"
                                  fill="none" stroke={r.color} stroke-width="1"
                                  stroke-dasharray="2,2" />
                        {:else}
                            <rect x="5" y={r.y} width="3" height={r.h} rx="1.5" fill={r.color} />
                        {/if}
                        <rect class="block-hit" x="0" y={r.y} width="12" height={r.h}
                              fill="transparent"
                              onclick={(e) => handleBlockClick(r, e)}
                              oncontextmenu={(e) => openBlockMenu(r, e)} />
                    {/each}
                {/if}
            </svg>
            <!-- 折叠后的行数角标。pointer-events 关掉，不挡选中。
                 r.y 是 SVG 坐标系；SVG 自身有 top: 4px 的偏移，这个 div 是
                 .term-wrap 子节点，要把那 4px 加回来才能跟主条对齐。 -->
            {#each blockRects as r (r.id)}
                {#if r.folded}
                    <div class="fold-label" style="top: calc({r.y}px + 4px);">
                        ⋯ {r.foldCount} {r.foldCount === 1 ? "line" : "lines"}
                    </div>
                {/if}
            {/each}
        {/if}
    </div>
    {#if ctxMenu}
        <BlockContextMenu
            x={ctxMenu.x}
            y={ctxMenu.y}
            items={ctxMenu.items}
            onClose={() => (ctxMenu = null)}
        />
    {/if}
    {#if app.isMobile}
        <MobileKeybar />
    {/if}
</div>

<style>
    .term-outer {
        display: flex;
        flex-direction: column;
        width: 100%;
        height: 100%;
    }

    .term-wrap {
        flex: 1;
        min-height: 0;
        position: relative;
    }

    /* xterm.js paints theme.background into the row canvas, but the
       .xterm-viewport scroll layer keeps its own background. When the
       user turns off "terminal bg follows theme" and picks a preset
       whose background differs from --bg, that viewport leaks a
       frame-coloured ring around the rendered rows. Pin it to --term-bg
       (kept in sync by themes/store.svelte.ts::writeTermVars). */
    .term-wrap :global(.xterm-viewport) {
        background-color: var(--term-bg) !important;
    }

    .xterm-host {
        width: 100%;
        height: 100%;
    }

    /* Widen left padding 4px → 12px to make room for the block bar.
       When the feature is off, restore the original symmetric 4px padding. */
    .term-wrap :global(.xterm) {
        height: 100%;
        padding: 4px 4px 4px 12px;
    }
    .term-wrap.no-block-bar :global(.xterm) {
        padding: 4px;
    }

    /* Mobile: hide xterm's inline .composition-view (the black box pinned at
       the cursor) — the soft keyboard's own candidate bar already shows the
       IME composing text, so it's redundant (Sogou flashes it on backspace).
       MUST be visibility:hidden, NOT display:none: xterm sizes the hidden input
       textarea from this element's getBoundingClientRect() (updateComposition-
       Elements), so display:none zeroes that rect, collapses the textarea, and
       breaks IME input entirely (Sogou can't type). visibility:hidden keeps the
       layout box (real rect, input works) while hiding the paint. */
    .term-wrap.is-mobile :global(.composition-view) {
        visibility: hidden !important;
    }

    /* Mobile: xterm 6's scrollbar slider has its own pointer-drag, which raced our
       touch-scroll handler (drag "sometimes" worked, sometimes not). Make it a pure
       position indicator on mobile so touches fall through to the content drag/fling;
       desktop keeps the draggable scrollbar. */
    .term-wrap.is-mobile :global(.xterm-scrollable-element > .scrollbar) {
        pointer-events: none;
    }

    /* Overlay painted inside the enlarged left padding. SVG itself ignores
       pointer events so text selection still works; only the per-block
       hit-box rects opt back in for click-to-select. */
    .block-bar {
        position: absolute;
        left: 0;
        top: 4px;
        width: 12px;
        height: calc(100% - 8px);
        pointer-events: none;
        overflow: visible;
    }
    /* 折叠角标：行末右侧贴一个灰字 "⋯ N lines"。
       pointer-events: none 让 xterm 选中、链接、滚动手势全部穿透。 */
    .fold-label {
        position: absolute;
        right: 8px;
        font-size: 11px;
        line-height: 1.4;
        padding: 0 6px;
        color: var(--text-dim);
        background: var(--surface);
        border-radius: 3px;
        pointer-events: none;
        white-space: nowrap;
        opacity: 0.85;
        z-index: 5;
    }

    .block-hit {
        pointer-events: auto;
        cursor: pointer;
    }

    /* Halo 光晕：实体半透明矩形，模糊柔化边缘。
       blur 在某些 WebView 上失效也无妨——半透明矩形本身就承载视觉信号。 */
    .block-halo {
        filter: blur(2px);
        pointer-events: none;
    }

    /* 染色层：整行宽的半透明色带。0.15 不透明度——文字依旧清晰可读，只是底色透出
       块的色条颜色。pointer-events:none 让选中/点击全部穿透到下面的 xterm。 */
    .block-tint {
        position: absolute;
        left: 0;
        right: 0;
        pointer-events: none;
        opacity: 0.15;
        border-radius: 2px;
    }

    .search-bar {
        display: flex;
        align-items: center;
        gap: 4px;
        padding: 4px 8px;
        background: var(--surface);
        border-bottom: 1px solid var(--divider);
        flex-shrink: 0;
    }

    .search-bar input {
        flex: 1;
        padding: 4px 8px;
        font-size: 12px;
        border-radius: 4px;
        min-width: 0;
    }

    .search-btn {
        background: none;
        border: none;
        color: var(--text-sub);
        font-size: 12px;
        cursor: pointer;
        padding: 2px 6px;
        border-radius: 4px;
    }

    .search-btn:hover {
        background: var(--divider);
        color: var(--text);
    }

    .auth-overlay {
        position: absolute;
        inset: 0;
        z-index: 10;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--overlay-strong);
    }

    .auth-dialog {
        background: var(--bg);
        border: 1px solid var(--divider);
        border-radius: 8px;
        padding: 20px;
        min-width: 300px;
        max-width: 400px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .auth-title {
        font-size: 14px;
        font-weight: 600;
        color: var(--text);
    }

    .auth-instructions {
        font-size: 12px;
        color: var(--text-sub);
    }

    .auth-label {
        display: flex;
        flex-direction: column;
        gap: 4px;
        font-size: 12px;
        color: var(--text-sub);
    }

    .auth-label input {
        padding: 6px 8px;
        border-radius: 4px;
        font-size: 13px;
    }

    .auth-submit {
        align-self: flex-end;
        padding: 6px 16px;
        border-radius: 4px;
        border: none;
        background: var(--accent);
        color: var(--bg);
        font-size: 13px;
        cursor: pointer;
    }
</style>
