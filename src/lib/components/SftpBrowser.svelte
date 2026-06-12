<script lang="ts">
    import {onDestroy, onMount, tick} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import * as app from "../stores/app.svelte.ts";
    import * as transfers from "../stores/transfers.svelte.ts";
    import type {RemoteEntry} from "../stores/app.svelte.ts";
    import { errMsg, t } from "../i18n/index.svelte.ts";

    /** Mirrors the backend WalkEntry; rel_path is always '/'-separated. */
    interface WalkEntry { rel_path: string; size: number; }

    /** Mirrors the backend FileStat. */
    interface FileStat {
        name: string;
        is_dir: boolean;
        size: number;
        mtime: number;
        uid?: number;
        gid?: number;
        user?: string;
        group?: string;
        permissions?: number;
    }

    let {meta}: { meta: Record<string, string> } = $props();

    let sftpId = $state<string | null>(null);
    let cwd = $state("/");
    let home = $state("/");
    let pathInput = $state("/");
    let entries = $state<RemoteEntry[]>([]);
    let loading = $state(true);
    let error = $state("");
    let notice = $state("");

    /** Names of selected entries in the current directory. Cleared on
     *  directory change — selections do not persist across directories. */
    let selected = $state(new Set<string>());
    /** Open/close state of the Upload dropdown menu. */
    let uploadMenuOpen = $state(false);
    let uploadWrapEl: HTMLDivElement | undefined;
    /** "Select all" checkbox — bound so we can drive `indeterminate` from an
     *  effect; the attribute form does not reliably sync the DOM property. */
    let selectAllEl: HTMLInputElement | undefined;

    /** Context menu state. */
    let ctxMenu = $state<{ x: number; y: number; entry: RemoteEntry } | null>(null);
    let ctxMenuEl: HTMLDivElement | undefined;
    let ctxDx = $state(0);
    let ctxDy = $state(0);
    let ctxReady = $state(false);

    /** Properties dialog state. */
    let propsStat = $state<FileStat | null>(null);
    let propsLoading = $state(false);

    /** Rename dialog state. */
    let renameEntry = $state<RemoteEntry | null>(null);
    let renameValue = $state("");

    /** Delete confirm state. */
    let deleteEntry = $state<RemoteEntry | null>(null);

    /** Renames the input after mount so it can receive focus. */
    let renameInputEl: HTMLInputElement | undefined;

    const selectedCount = $derived(selected.size);
    const allSelected = $derived(entries.length > 0 && selected.size === entries.length);
    const someSelected = $derived(selected.size > 0 && selected.size < entries.length);

    onMount(async () => {
        try {
            let id: string;
            if (meta.sessionId) {
                // Reuse existing SSH connection — no re-authentication needed
                id = await invoke<string>("sftp_connect_session", { sessionId: meta.sessionId });
            } else {
                id = await invoke<string>("sftp_connect", {
                    host: meta.host, port: Number(meta.port),
                    username: meta.username, authType: meta.authType, secret: meta.secret || null,
                });
            }
            sftpId = id;
            const h = await invoke<string>("sftp_home", {sftpId: id});
            home = h;
            cwd = h;
            pathInput = h;
            await listDir(h);
        } catch (e: any) {
            error = errMsg(e);
            loading = false;
        }
    });

    onDestroy(() => {
        if (sftpId) invoke("sftp_close", {sftpId});
    });

    async function listDir(path: string) {
        loading = true;
        error = "";
        try {
            entries = await invoke<RemoteEntry[]>("sftp_list", {sftpId, path});
            // Clear selection on directory change — selection has no meaning across directories.
            selected = new Set();
            cwd = path;
            pathInput = path;
        } catch (e: any) {
            error = errMsg(e);
        }
        loading = false;
    }

    function goUp() {
        const parent = cwd.replace(/\/[^/]+\/?$/, "") || "/";
        listDir(parent);
    }

    function expandHome(p: string): string {
        if (p !== "~" && !p.startsWith("~/")) return p;
        return (home + p.slice(1)).replace(/\/{2,}/g, "/");
    }

    function revertInput() {
        pathInput = cwd;
        error = "";
    }

    function submitPath() {
        const target = expandHome(pathInput.trim());
        if (!target) {
            revertInput();
            return;
        }
        listDir(target);
    }

    function onPathKeyDown(e: KeyboardEvent) {
        if (e.key === "Enter") {
            e.preventDefault();
            submitPath();
        } else if (e.key === "Escape") {
            revertInput();
            (e.currentTarget as HTMLInputElement).blur();
        }
    }

    function openEntry(e: RemoteEntry) {
        if (e.is_dir) listDir(joinRemote(cwd, e.name));
    }

    function basename(p: string): string {
        return p.split(/[\\/]/).pop() || p;
    }

    /** Join a remote path: always '/'-separated, empty segments filtered,
     *  root directory special-cased. */
    function joinRemote(...parts: string[]): string {
        let acc = "";
        for (const p of parts) {
            if (!p) continue;
            const cleaned = p.replace(/^\/+|\/+$/g, "");
            if (cleaned) acc += "/" + cleaned;
        }
        return acc || "/";
    }

    /** Join a local path: separator follows root (Windows '\\', Unix '/').
     *  '/' within rel_path is translated to the platform separator. */
    function joinLocal(root: string, ...rels: string[]): string {
        const sep = root.includes("\\") ? "\\" : "/";
        let acc = root.replace(/[\\/]+$/, "");
        for (const r of rels) {
            if (!r) continue;
            const cleaned = r.replace(/^[\\/]+|[\\/]+$/g, "").replace(/\//g, sep);
            if (cleaned) acc += sep + cleaned;
        }
        return acc;
    }

    function formatSize(bytes: number): string {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} K`;
        if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} M`;
        return `${(bytes / 1073741824).toFixed(1)} G`;
    }

    /** Render mtime: current year as MM-DD HH:mm, otherwise YYYY-MM-DD.
     *  A value of 0 means the server did not provide an mtime. */
    function formatMtime(secs: number): string {
        if (!secs) return "—";
        const d = new Date(secs * 1000);
        const yy = d.getFullYear();
        const now = new Date();
        const pad = (n: number) => n.toString().padStart(2, "0");
        if (yy === now.getFullYear()) {
            return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
        }
        return `${yy}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
    }

    function formatPermissions(perm: number | undefined): string {
        if (perm === undefined || perm === null) return "—";
        const p = perm & 0o777;
        const rwx = (v: number) => (v & 4 ? "r" : "-") + (v & 2 ? "w" : "-") + (v & 1 ? "x" : "-");
        return rwx((p >> 6) & 7) + rwx((p >> 3) & 7) + rwx(p & 7);
    }

    function toggleSelected(name: string) {
        const next = new Set(selected);
        if (next.has(name)) next.delete(name);
        else next.add(name);
        selected = next;
    }

    function toggleAll() {
        if (selected.size === entries.length) selected = new Set();
        else selected = new Set(entries.map(e => e.name));
    }

    function closeUploadMenu() { uploadMenuOpen = false; }
    function toggleUploadMenu() { uploadMenuOpen = !uploadMenuOpen; }

    function onWindowMouseDown(ev: MouseEvent) {
        if (!uploadMenuOpen) return;
        const target = ev.target as Node | null;
        if (uploadWrapEl && target && !uploadWrapEl.contains(target)) closeUploadMenu();
    }

    $effect(() => {
        if (uploadMenuOpen) {
            window.addEventListener("mousedown", onWindowMouseDown);
            return () => window.removeEventListener("mousedown", onWindowMouseDown);
        }
    });

    // `indeterminate` is a DOM property only — Svelte's attribute spread does
    // not reliably set it. Sync it imperatively whenever the selection changes.
    $effect(() => {
        if (selectAllEl) selectAllEl.indeterminate = someSelected;
    });

    // ── Context menu ──

    function onContextMenu(e: MouseEvent, entry: RemoteEntry) {
        e.preventDefault();
        e.stopPropagation();
        if (selected.has(entry.name)) return;
        ctxDx = 0;
        ctxDy = 0;
        ctxReady = false;
        ctxMenu = { x: e.clientX, y: e.clientY, entry };
    }

    function closeCtxMenu() { ctxMenu = null; ctxReady = false; }

    function onSftpContextMenu(e: MouseEvent) {
        e.preventDefault();
    }

    function onCtxMenuMount() {
        if (!ctxMenuEl) return;
        const r = ctxMenuEl.getBoundingClientRect();
        if (r.right > window.innerWidth) ctxDx = window.innerWidth - r.right - 4;
        if (r.bottom > window.innerHeight) ctxDy = window.innerHeight - r.bottom - 4;
        ctxReady = true;
    }

    $effect(() => {
        if (ctxMenu && ctxMenuEl) onCtxMenuMount();
    });

    function onDocumentKeydown(e: KeyboardEvent) {
        if (e.key === "Escape" && ctxMenu) {
            e.preventDefault();
            e.stopPropagation();
            closeCtxMenu();
        }
    }

    onMount(() => {
        document.addEventListener("keydown", onDocumentKeydown);
    });

    onDestroy(() => {
        document.removeEventListener("keydown", onDocumentKeydown);
    });

    function entryPath(entry: RemoteEntry): string {
        return joinRemote(cwd, entry.name);
    }

    function copyPath(entry: RemoteEntry) {
        app.writeClipboard(entryPath(entry));
        closeCtxMenu();
    }

    function copyPathToTerminal(entry: RemoteEntry) {
        // Remote filenames may legally contain control characters, including
        // newlines — sent raw to the shell, a hostile name like "x\nrm -rf ~\n"
        // would execute immediately. Strip C0 controls + DEL before sending.
        const path = entryPath(entry).replace(/[\x00-\x1f\x7f]/g, "");
        app.sendTextToActiveTerminal(path);
        closeCtxMenu();
    }

    /** Queue downloads for a set of entries into a local directory.
     *  Walks directories, queues individual files, and collects walk errors
     *  so the caller can display them. */
    async function queueDownloads(items: RemoteEntry[], dir: string): Promise<{ queued: number; walkErrors: string[] }> {
        let queued = 0;
        // Accumulate per-tree walk failures so users see every failed dir,
        // not just the last one.
        const walkErrors: string[] = [];
        for (const e of items) {
            const remote = joinRemote(cwd, e.name);
            if (e.is_dir) {
                // Expand each subtree into N independent Transfers. A walk
                // failure only skips that subtree; other selected entries
                // continue to be queued.
                try {
                    const walked = await invoke<WalkEntry[]>("sftp_walk_remote_dir", {
                        sftpId, remoteRoot: remote,
                    });
                    for (const w of walked) {
                        await transfers.startDownload({
                            sessionId: meta.sessionId,
                            remotePath: joinRemote(remote, w.rel_path),
                            localPath:  joinLocal(dir, e.name, w.rel_path),
                            sizeHint:   w.size,
                        });
                        queued++;
                    }
                } catch (err) {
                    walkErrors.push(`${e.name}: ${errMsg(err)}`);
                }
            } else {
                await transfers.startDownload({
                    sessionId: meta.sessionId,
                    remotePath: remote,
                    localPath:  joinLocal(dir, e.name),
                    sizeHint:   e.size,
                });
                queued++;
            }
        }
        return { queued, walkErrors };
    }

    async function downloadEntry(entry: RemoteEntry) {
        closeCtxMenu();
        error = "";
        notice = "";
        if (!meta.sessionId) { error = "Missing SSH session"; return; }
        try {
            if (entry.is_dir) {
                const dir = await invoke<string | null>("sftp_pick_folder");
                if (!dir) return;
                const { queued, walkErrors } = await queueDownloads([entry], dir);
                if (walkErrors.length > 0) error = `${t("sftp.walk_failed")}\n${walkErrors.join("\n")}`;
                if (queued > 0) notice = t("sftp.queued_n", { n: queued });
            } else if (app.isMobile) {
                const dir = await invoke<string | null>("sftp_pick_folder");
                if (!dir) return;
                await transfers.startDownload({
                    sessionId: meta.sessionId,
                    remotePath: entryPath(entry),
                    localPath:  joinLocal(dir, entry.name),
                    sizeHint:   entry.size,
                });
                notice = t("sftp.queued_n", { n: 1 });
            } else {
                const saved = await invoke<string | null>("sftp_save_file", {
                    sftpId, remotePath: entryPath(entry), defaultName: entry.name,
                });
                if (saved) notice = t("sftp.queued_n", { n: 1 });
            }
        } catch (err: any) {
            error = errMsg(err);
        }
    }

    function confirmDelete(entry: RemoteEntry) {
        closeCtxMenu();
        deleteEntry = entry;
    }

    async function doDelete() {
        if (!deleteEntry) return;
        const entry = deleteEntry;
        deleteEntry = null;
        error = "";
        try {
            await invoke("sftp_remove", { sftpId, path: entryPath(entry) });
            await listDir(cwd);
        } catch (e: any) {
            error = errMsg(e);
        }
    }

    function startRename(entry: RemoteEntry) {
        closeCtxMenu();
        renameEntry = entry;
        renameValue = entry.name;
        tick().then(() => { renameInputEl?.focus(); renameInputEl?.select(); });
    }

    async function doRename() {
        if (!renameEntry || !renameValue.trim()) return;
        if (renameValue.trim().includes("/")) return;
        const oldPath = entryPath(renameEntry);
        const newPath = joinRemote(cwd, renameValue.trim());
        error = "";
        try {
            await invoke("sftp_rename", { sftpId, oldPath, newPath });
            renameEntry = null;
            renameValue = "";
            await listDir(cwd);
        } catch (e: any) {
            error = errMsg(e);
        }
    }

    function showProperties(entry: RemoteEntry) {
        closeCtxMenu();
        propsLoading = true;
        propsStat = null;
        const path = entryPath(entry);
        invoke<FileStat>("sftp_stat", { sftpId, path })
            .then(stat => { if (propsLoading) { propsStat = stat; propsLoading = false; } })
            .catch((e: any) => { if (propsLoading) { error = errMsg(e); propsLoading = false; } });
    }

    function closeProperties() { propsStat = null; propsLoading = false; }

    // ── Download selected ──

    async function downloadSelected() {
        error = "";
        notice = "";
        if (!meta.sessionId) { error = "Missing SSH session"; return; }
        if (selected.size === 0) return;
        const items = entries.filter(e => selected.has(e.name));
        try {
            const dir = await invoke<string | null>("sftp_pick_folder");
            if (!dir) return;
            const { queued, walkErrors } = await queueDownloads(items, dir);
            if (walkErrors.length > 0) error = `${t("sftp.walk_failed")}\n${walkErrors.join("\n")}`;
            if (queued > 0) notice = t("sftp.queued_n", { n: queued });
            selected = new Set();
        } catch (err: any) {
            error = errMsg(err);
        }
    }

    async function uploadFiles() {
        error = "";
        notice = "";
        if (!meta.sessionId) { error = "Missing SSH session"; return; }
        try {
            const paths = await invoke<string[] | null>("sftp_pick_open_files");
            if (!paths || paths.length === 0) return;
            for (const p of paths) {
                const name = basename(p);
                await transfers.startUpload({
                    sessionId: meta.sessionId,
                    localPath:  p,
                    remotePath: joinRemote(cwd, name),
                });
            }
            notice = t("sftp.queued_n", { n: paths.length });
        } catch (err: any) {
            error = errMsg(err);
        }
    }

    async function uploadFolder() {
        error = "";
        notice = "";
        if (!meta.sessionId) { error = "Missing SSH session"; return; }
        try {
            const dir = await invoke<string | null>("sftp_pick_folder");
            if (!dir) return;
            const walked = await invoke<WalkEntry[]>("walk_local_dir", { localRoot: dir });
            if (walked.length === 0) { notice = t("sftp.folder_empty"); return; }
            const folderName = basename(dir);
            for (const w of walked) {
                await transfers.startUpload({
                    sessionId: meta.sessionId,
                    localPath:  joinLocal(dir, w.rel_path),
                    remotePath: joinRemote(cwd, folderName, w.rel_path),
                });
            }
            notice = t("sftp.queued_n", { n: walked.length });
        } catch (err: any) {
            error = errMsg(err);
        }
    }

</script>

<div class="sftp">
    <div class="toolbar">
        <span class="title">SFTP</span>
        <span class="grow"></span>
        <button type="button" class="btn-icon" onclick={() => app.closeSftp()} aria-label={t("common.close")} title={t("common.close")}>×</button>
    </div>
    <div class="header">
        <button class="btn btn-sm" onclick={goUp}>{t("sftp.up")}</button>
        <button class="btn btn-sm" onclick={() => listDir(cwd)}>{t("sftp.refresh")}</button>
        <div class="upload-wrap" bind:this={uploadWrapEl}>
            <!-- The dialog commands `sftp_pick_*` are not registered on Android
                 (rfd has no folder/multi-file picker there); gate the entry. -->
            <button class="btn btn-sm" disabled={!sftpId || app.isMobile} onclick={toggleUploadMenu} aria-haspopup="menu" aria-expanded={uploadMenuOpen}>
                {t("sftp.upload")} <span class="caret">▾</span>
            </button>
            {#if uploadMenuOpen}
                <div class="upload-menu" role="menu">
                    <button role="menuitem" onclick={() => { closeUploadMenu(); uploadFiles(); }}>{t("sftp.upload_files")}</button>
                    <button role="menuitem" onclick={() => { closeUploadMenu(); uploadFolder(); }}>{t("sftp.upload_folder")}</button>
                </div>
            {/if}
        </div>
        <button class="btn btn-sm" disabled={selectedCount === 0 || !sftpId || app.isMobile} onclick={downloadSelected}>
            {selectedCount > 0 ? t("sftp.download_n", { n: selectedCount }) : t("sftp.download")}
        </button>
    </div>
    <input
        type="text"
        class="breadcrumb-input"
        bind:value={pathInput}
        onkeydown={onPathKeyDown}
        disabled={!sftpId}
        spellcheck="false"
        autocomplete="off"
        autocapitalize="off"
        aria-label="Path"
    />

    {#if error}
        <div class="error-banner">{error}</div>
    {/if}
    {#if notice}
        <div class="notice-banner">{notice}</div>
    {/if}

    {#if loading}
        <p class="loading">{t("sftp.loading")}</p>
    {:else}
        <div class="file-list" oncontextmenu={onSftpContextMenu}>
            <div class="file-row file-header">
                <span class="cell-check">
                    <input
                        type="checkbox"
                        bind:this={selectAllEl}
                        checked={allSelected}
                        disabled={entries.length === 0}
                        onchange={toggleAll}
                        aria-label={t("sftp.select_all")}
                    />
                </span>
                <span class="cell-name h-label">{t("sftp.column.name")}</span>
                <span class="cell-size h-label">{t("sftp.column.size")}</span>
                <span class="cell-mtime h-label">{t("sftp.column.modified")}</span>
            </div>
            {#each entries as e (e.name)}
                <div
                    class="file-row"
                    class:dir={e.is_dir}
                    class:selected={selected.has(e.name)}
                    oncontextmenu={(ev) => onContextMenu(ev, e)}
                >
                    <span class="cell-check">
                        <input
                            type="checkbox"
                            checked={selected.has(e.name)}
                            onchange={() => toggleSelected(e.name)}
                            aria-label={t("sftp.select_entry", { name: e.name })}
                        />
                    </span>
                    <button class="file-name cell-name" onclick={() => openEntry(e)} title={e.name}>
                        <span class="file-icon">{e.is_dir ? "📁" : (e.is_symlink ? "🔗" : "📄")}</span>
                        <span class="file-label">{e.name}</span>
                    </button>
                    <span class="cell-size">{e.is_dir ? "—" : formatSize(e.size)}</span>
                    <span class="cell-mtime">{formatMtime(e.mtime)}</span>
                </div>
            {:else}
                <p class="empty">{t("sftp.empty_dir")}</p>
            {/each}
        </div>
    {/if}
</div>

{#if ctxMenu}
    <div class="ctx-backdrop"
         onclick={closeCtxMenu}
         oncontextmenu={(e) => { e.preventDefault(); closeCtxMenu(); }}
         role="presentation"></div>
    <div class="ctx-menu surface-raised"
         class:ready={ctxReady}
         bind:this={ctxMenuEl}
         style="left: {ctxMenu.x + ctxDx}px; top: {ctxMenu.y + ctxDy}px;">
        <button class="ctx-item" onclick={() => downloadEntry(ctxMenu!.entry)}>{t("sftp.ctx.download")}</button>
        <button class="ctx-item" onclick={() => confirmDelete(ctxMenu!.entry)}>{t("sftp.ctx.delete")}</button>
        <button class="ctx-item" onclick={() => startRename(ctxMenu!.entry)}>{t("sftp.ctx.rename")}</button>
        <div class="ctx-sep"></div>
        <button class="ctx-item" onclick={() => copyPath(ctxMenu!.entry)}>{t("sftp.ctx.copy_path")}</button>
        <button class="ctx-item" onclick={() => copyPathToTerminal(ctxMenu!.entry)}>{t("sftp.ctx.copy_path_terminal")}</button>
        <div class="ctx-sep"></div>
        <button class="ctx-item" onclick={() => showProperties(ctxMenu!.entry)}>{t("sftp.ctx.properties")}</button>
    </div>
{/if}

{#if deleteEntry}
    <div class="modal-overlay" onclick={() => { deleteEntry = null; }}>
        <div class="modal-card" onclick={(e) => e.stopPropagation()}>
            <p class="modal-text">{t("sftp.delete.confirm", { name: deleteEntry.name })}</p>
            <div class="modal-actions">
                <button class="btn btn-sm" onclick={() => { deleteEntry = null; }}>{t("common.cancel")}</button>
                <button class="btn btn-sm btn-danger" onclick={doDelete}>{t("common.delete")}</button>
            </div>
        </div>
    </div>
{/if}

{#if renameEntry}
    <div class="modal-overlay" onclick={() => { renameEntry = null; }}>
        <div class="modal-card" onclick={(e) => e.stopPropagation()}>
            <p class="modal-title">{t("sftp.rename.title")}</p>
            <input
                type="text"
                class="modal-input"
                bind:this={renameInputEl}
                bind:value={renameValue}
                onkeydown={(e) => { if (e.key === "Enter") doRename(); if (e.key === "Escape") renameEntry = null; }}
            />
            <div class="modal-actions">
                <button class="btn btn-sm" onclick={() => { renameEntry = null; }}>{t("common.cancel")}</button>
                <button class="btn btn-sm" onclick={doRename}>{t("sftp.rename.title")}</button>
            </div>
        </div>
    </div>
{/if}

{#if propsStat || propsLoading}
    <div class="modal-overlay" onclick={closeProperties}>
        <div class="modal-card props-card" onclick={(e) => e.stopPropagation()}>
            {#if propsLoading}
                <p class="loading">{t("common.loading")}</p>
            {:else if propsStat}
                <p class="props-filename">{propsStat.name}</p>
                <div class="props-spacer"></div>
                {#if !propsStat.is_dir}
                    <div class="props-row">
                        <span class="props-label">{t("sftp.props.size")}</span>
                        <span class="props-value">{formatSize(propsStat.size)}</span>
                    </div>
                {/if}
                <div class="props-row">
                    <span class="props-label">{t("sftp.props.modified")}</span>
                    <span class="props-value">{formatMtime(propsStat.mtime)}</span>
                </div>
                <div class="props-row">
                    <span class="props-label">{t("sftp.props.owner")}</span>
                    <span class="props-value">{propsStat.user ?? (propsStat.uid !== undefined && propsStat.uid !== null ? String(propsStat.uid) : "—")}</span>
                </div>
                <div class="props-row">
                    <span class="props-label">{t("sftp.props.group")}</span>
                    <span class="props-value">{propsStat.group ?? (propsStat.gid !== undefined && propsStat.gid !== null ? String(propsStat.gid) : "—")}</span>
                </div>
                <div class="props-row">
                    <span class="props-label">{t("sftp.props.permissions")}</span>
                    <span class="props-value">{formatPermissions(propsStat.permissions)}</span>
                </div>
            {/if}
            <div class="props-ok">
                <button class="btn btn-sm" onclick={closeProperties}>{t("sftp.props.ok")}</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .sftp {
        display: flex;
        flex-direction: column;
        height: 100%;
        padding: 12px 14px;
        box-sizing: border-box;
        overflow-y: auto;
        container-type: inline-size;
        /* aside 把 SFTP 收成侧边栏；不再做 max-width 居中。窄宽度下让按钮换行而不是溢出。 */
    }

    .toolbar {
        display: flex;
        align-items: center;
        gap: 8px;
        padding-bottom: 8px;
        border-bottom: 1px solid var(--divider);
        margin-bottom: 10px;
    }

    .title {
        font-size: 13px;
        font-weight: 600;
        color: var(--text-sub);
        letter-spacing: 0.4px;
    }

    .grow { flex: 1; }

    .btn-icon {
        width: 24px;
        height: 24px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border: none;
        background: transparent;
        color: var(--text-sub);
        font-size: 18px;
        line-height: 1;
        border-radius: var(--radius-sm);
        cursor: pointer;
    }
    .btn-icon:hover { color: var(--text); background: var(--accent-soft); }

    .header {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 8px;
        flex-wrap: wrap;
    }

    .upload-wrap {
        position: relative;
        display: inline-flex;
    }
    .caret {
        font-size: 9px;
        margin-left: 2px;
        opacity: 0.75;
    }
    .upload-menu {
        position: absolute;
        top: calc(100% + 4px);
        left: 0;
        z-index: 10;
        background: var(--surface);
        border: 1px solid var(--divider);
        border-radius: var(--radius-sm);
        box-shadow: 0 4px 12px rgba(0, 0, 0, 0.18);
        min-width: 140px;
        padding: 4px;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }
    .upload-menu button {
        background: transparent;
        border: none;
        text-align: left;
        font: inherit;
        color: var(--text);
        padding: 6px 10px;
        border-radius: var(--radius-sm);
        cursor: pointer;
    }
    .upload-menu button:hover {
        background: var(--accent-soft);
    }

    .breadcrumb-input {
        font-family: monospace;
        font-size: 12px;
        color: var(--text);
        padding: calc(6px * var(--density)) calc(10px * var(--density));
        margin-bottom: calc(8px * var(--density));
        background: var(--bg);
        box-shadow: var(--pressed);
        border: none;
        border-radius: var(--radius-sm);
        outline: none;
        width: 100%;
        box-sizing: border-box;
    }
    .breadcrumb-input:focus {
        box-shadow: var(--pressed), 0 0 0 1px var(--accent);
    }

    .error-banner {
        background: color-mix(in srgb, var(--error) 10%, transparent);
        border-left: 3px solid var(--error);
        color: var(--error);
        padding: 8px 12px;
        border-radius: var(--radius-sm);
        margin-bottom: 8px;
        font-size: 12px;
        white-space: pre-line;
    }

    .notice-banner {
        background: color-mix(in srgb, var(--success) 10%, transparent);
        border-left: 3px solid var(--success);
        color: var(--success);
        padding: 8px 12px;
        border-radius: var(--radius-sm);
        margin-bottom: 8px;
        font-size: 12px;
    }

    .loading {
        text-align: center;
        color: var(--text-dim);
        padding: 24px;
    }

    .file-list {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .file-row {
        display: grid;
        grid-template-columns: 24px 1fr 60px 90px;
        align-items: center;
        gap: 8px;
        padding: 6px 8px;
        border-radius: var(--radius-sm);
        transition: background 0.1s;
    }

    .file-row:not(.file-header):hover {
        background: color-mix(in srgb, var(--text-sub) 15%, transparent);
    }
    .file-row.selected {
        background: color-mix(in srgb, var(--accent) 12%, transparent);
    }
    .file-row.selected:hover {
        background: color-mix(in srgb, var(--accent) 18%, transparent);
    }

    .file-header {
        font-size: 11px;
        color: var(--text-dim);
        letter-spacing: 0.4px;
        text-transform: uppercase;
        border-bottom: 1px solid var(--divider);
        padding-bottom: 6px;
        margin-bottom: 2px;
    }
    .h-label { user-select: none; }
    .cell-size.h-label, .cell-mtime.h-label { text-align: right; }

    .cell-check {
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .cell-check input {
        cursor: pointer;
        margin: 0;
    }

    .file-name {
        grid-column: 2;
        border: none;
        background: none;
        text-align: left;
        font-family: inherit;
        font-size: 13px;
        color: var(--text);
        cursor: pointer;
        display: flex;
        align-items: center;
        gap: 6px;
        min-width: 0; /* let .file-label ellipsis-truncate */
        padding: 0;
    }
    .file-label {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .file-row.dir .file-name {
        font-weight: 600;
        color: var(--accent);
    }

    .file-icon {
        font-size: 14px;
        flex-shrink: 0;
    }

    .cell-size {
        font-size: 11px;
        color: var(--text-dim);
        text-align: right;
    }
    .cell-mtime {
        font-size: 11px;
        color: var(--text-dim);
        text-align: right;
        white-space: nowrap;
    }

    /* Narrow widths: drop the mtime column first. Size always stays — for
       multi-file downloads users care most about file size. */
    @container (max-width: 360px) {
        .file-row {
            grid-template-columns: 24px 1fr 60px;
        }
        .cell-mtime { display: none; }
    }

    .empty {
        text-align: center;
        color: var(--text-dim);
        padding: 24px;
    }

    /* ── Context menu (matches TabContextMenu style) ── */

    .ctx-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
    }

    .ctx-menu {
        position: fixed;
        z-index: 501;
        min-width: 180px;
        padding: calc(4px * var(--density));
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        display: flex;
        flex-direction: column;
        gap: 1px;
        visibility: hidden;
    }
    .ctx-menu.ready {
        visibility: visible;
    }

    .ctx-item {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 20px;
        padding: 7px 12px;
        border: none;
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--text);
        font-family: inherit;
        font-size: 13px;
        text-align: left;
        cursor: pointer;
        white-space: nowrap;
    }
    .ctx-item:hover:not(:disabled) {
        background: var(--surface);
    }

    .ctx-sep {
        height: 1px;
        background: var(--divider);
        margin: 4px 6px;
    }

    /* ── Modal overlay ── */

    .modal-overlay {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.4);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 600;
    }

    .modal-card {
        background: var(--surface);
        border: 1px solid var(--divider);
        border-radius: 8px;
        padding: 20px 24px;
        min-width: 280px;
        max-width: 90vw;
        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    }

    .modal-text {
        font-size: 13px;
        margin: 0 0 16px;
        line-height: 1.5;
    }

    .modal-title {
        font-size: 13px;
        font-weight: 600;
        margin: 0 0 12px;
    }

    .modal-input {
        width: 100%;
        box-sizing: border-box;
        font-family: monospace;
        font-size: 12px;
        padding: 6px 8px;
        border: 1px solid var(--divider);
        border-radius: var(--radius-sm);
        background: var(--bg);
        color: var(--text);
        outline: none;
        margin-bottom: 12px;
    }
    .modal-input:focus {
        border-color: var(--accent);
    }

    .modal-actions {
        display: flex;
        justify-content: flex-end;
        gap: 8px;
    }

    .btn-danger {
        color: var(--error) !important;
    }

    /* ── Properties dialog ── */

    .props-card {
        min-width: 300px;
    }

    .props-filename {
        font-size: 14px;
        font-weight: 600;
        margin: 0 0 4px;
        word-break: break-all;
    }

    .props-spacer {
        height: 12px;
    }

    .props-row {
        display: flex;
        justify-content: space-between;
        align-items: baseline;
        padding: 4px 0;
        font-size: 12px;
    }

    .props-label {
        color: var(--text-sub);
        flex-shrink: 0;
        margin-right: 16px;
    }

    .props-value {
        color: var(--text);
        text-align: right;
        word-break: break-all;
    }

    .props-ok {
        display: flex;
        justify-content: flex-end;
        margin-top: 16px;
    }

</style>
