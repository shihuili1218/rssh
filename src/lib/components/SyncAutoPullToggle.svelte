<!-- Same contract as AI's DangerModeToggle: the trigger never mutates the
     checkbox directly. Enabling asks for the decryption password first;
     disabling saves immediately. The store changes only after backend success. -->
<script lang="ts">
    import type { Snippet } from "svelte";
    import * as syncStatus from "../stores/sync.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import Modal from "./Modal.svelte";

    let { source, enabled, trigger, onError }: {
        source: syncStatus.SyncSource;
        enabled: boolean;
        trigger: Snippet<[() => void, boolean]>;
        onError?: (message: string) => void;
    } = $props();

    let showDialog = $state(false);
    let saving = $state(false);
    let password = $state("");
    let passwordError = $state("");

    let titleId = $derived(`sync-auto-pull-${source}-title`);
    let errorId = $derived(`sync-auto-pull-${source}-error`);

    function closeDialog() {
        if (saving) return;
        showDialog = false;
        password = "";
        passwordError = "";
    }

    function requestToggle() {
        if (saving) return;
        if (enabled) {
            void apply(false);
        } else {
            password = "";
            passwordError = "";
            showDialog = true;
        }
    }

    async function apply(wantOn: boolean) {
        if (wantOn && !password) {
            passwordError = t("sync.password_empty");
            return;
        }

        const savedPassword = wantOn ? password : null;
        saving = true;
        showDialog = false;
        password = "";
        passwordError = "";
        try {
            await syncStatus.saveAutoPull(source, wantOn, savedPassword);
        } catch (error) {
            onError?.(errMsg(error));
        } finally {
            saving = false;
        }
    }
</script>

{@render trigger(requestToggle, saving)}

{#if showDialog}
    <Modal onClose={closeDialog} class="stack" aria-labelledby={titleId}>
        <h3 id={titleId} class="dialog-title">{t("sync.auto_pull_password")}</h3>
        <input type="password"
               bind:value={password}
               placeholder={t("sync.password")}
               autocomplete="current-password"
               aria-describedby={passwordError ? errorId : undefined}
               onkeydown={(event) => { if (event.key === "Enter") void apply(true); }}/>
        {#if passwordError}
            <div id={errorId} class="password-error" role="alert">{passwordError}</div>
        {/if}
        <div class="modal-actions">
            <button class="btn btn-sm" onclick={closeDialog}>{t("common.cancel")}</button>
            <button class="btn btn-accent btn-sm" onclick={() => apply(true)} disabled={saving}>
                {t("common.confirm")}
            </button>
        </div>
    </Modal>
{/if}

<style>
    .password-error {
        font-size: 12px;
        color: var(--error);
    }
</style>
