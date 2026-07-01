<!-- The one modal shell: scrim + centered card + dismiss behaviour. Every
     confirm/form popup renders its content as children; the look (border +
     flat drop shadow, theme-independent) and the close plumbing (scrim click,
     Esc) live here once instead of being copy-pasted per component.

     Style is deliberately NOT tied to the shape theme's --raised/--elevation
     tokens — those turn puffy under the neumorphism shape. A popup's lift off
     the scrim shouldn't change with the skin. -->
<script lang="ts">
    import type { Snippet } from "svelte";
    import type { HTMLAttributes } from "svelte/elements";

    let {
        onClose,
        class: klass = "",
        children,
        ...rest
    }: {
        /** Dismiss handler — fired by scrim click and by Esc. */
        onClose: () => void;
        /** Extra classes merged onto the card (rare sizing overrides). */
        class?: string;
        children: Snippet;
    } & HTMLAttributes<HTMLDivElement> = $props();

    // Esc closes only the topmost modal. Capture phase + stopPropagation so it
    // beats app-level Esc handlers (AppShell closes the SFTP panel / drawer on
    // Esc); without this, Esc inside a modal would also collapse what's under it.
    $effect(() => {
        function onKey(e: KeyboardEvent) {
            if (e.key !== "Escape") return;
            e.stopPropagation();
            e.preventDefault();
            onClose();
        }
        window.addEventListener("keydown", onKey, true);
        return () => window.removeEventListener("keydown", onKey, true);
    });
</script>

<div class="modal-overlay" onclick={onClose} role="presentation">
    <div
        class="modal-card surface-popup {klass}"
        {...rest}
        role="dialog"
        aria-modal="true"
        tabindex="-1"
        onclick={(e) => e.stopPropagation()}
    >
        {@render children()}
    </div>
</div>

<style>
    .modal-overlay {
        position: fixed;
        inset: 0;
        z-index: 600;
        background: var(--overlay-strong);
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 20px;
    }
    /* Surface (bg / border / radius / theme shadow) comes from .surface-popup;
       .modal-card only owns layout. */
    .modal-card {
        padding: 20px 24px;
        min-width: 280px;
        max-width: min(90vw, 460px);
        max-height: calc(100vh - 40px);
        overflow-y: auto;
        box-sizing: border-box;
    }
    /* Opt-in vertical rhythm for title/body/actions dialogs. Content that lays
       itself out (e.g. tight property rows) just omits it and gets block flow. */
    .modal-card.stack {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
</style>
