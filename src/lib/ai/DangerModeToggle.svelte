<!-- Headless danger-mode toggle: the ONE home of the safety contract — enabling
     pops a confirm modal (auto-approve runs every proposed command with no
     confirmation, a foot-gun), disabling is immediate (off = the safe default).
     The trigger control (settings checkbox / toolbar icon) is delegated to the
     `trigger` snippet so each surface renders its own look while sharing this
     logic + the confirm modal. Never call saveSettings({dangerMode:true})
     anywhere else — route every toggle through requestToggle so the warning
     can't be bypassed. -->
<script lang="ts">
    import type { Snippet } from "svelte";
    import * as ai from "./store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";

    let { trigger, onError }: {
        // trigger(requestToggle, saving): the caller wires these onto its control.
        trigger: Snippet<[() => void, boolean]>;
        // Raw error message; each surface routes it to its own error UI.
        onError?: (msg: string) => void;
    } = $props();

    let showDialog = $state(false);
    let saving = $state(false);

    function requestToggle() {
        if (saving) return;
        if (ai.settings()?.danger_mode === true) {
            void apply(false); // disabling → immediate
        } else {
            showDialog = true; // enabling → confirm first
        }
    }

    async function apply(wantOn: boolean) {
        saving = true;
        showDialog = false;
        try {
            await ai.saveSettings({ dangerMode: wantOn });
        } catch (e) {
            console.error("[ai] toggle danger mode:", e);
            onError?.(errMsg(e));
        } finally {
            saving = false;
        }
    }
</script>

{@render trigger(requestToggle, saving)}

{#if showDialog}
    <div class="dialog-backdrop" onclick={() => (showDialog = false)} role="presentation">
        <div class="dialog surface-raised" onclick={(e) => e.stopPropagation()}
             role="dialog" aria-modal="true"
             aria-labelledby="danger-confirm-title" aria-describedby="danger-confirm-body">
            <h3 id="danger-confirm-title" class="title">{t("ai.settings.danger.confirm_title")}</h3>
            <div id="danger-confirm-body" class="body">{t("ai.settings.danger.confirm_body")}</div>
            <div class="btn-row">
                <button class="btn btn-sm" onclick={() => (showDialog = false)}>{t("common.cancel")}</button>
                <button class="btn btn-sm btn-danger-solid" onclick={() => apply(true)} disabled={saving}>
                    {t("ai.settings.danger.confirm_enable")}
                </button>
            </div>
        </div>
    </div>
{/if}

<style>
    .dialog-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
        background: var(--overlay-strong);
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .dialog {
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        padding: calc(24px * var(--density));
        max-width: 460px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .title {
        font-size: 16px;
        color: var(--error);
        font-weight: 700;
    }
    .body {
        font-size: 13px;
        color: var(--text);
        line-height: 1.55;
        white-space: pre-line;
    }
    .btn-row {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
        margin-top: 4px;
    }
    /* "Enable anyway" — error fill so clicking it reads as stepping on a mine. */
    .btn-danger-solid {
        background: var(--error);
        color: var(--white);
        border-color: var(--error);
    }
    .btn-danger-solid:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
