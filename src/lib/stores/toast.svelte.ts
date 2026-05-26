type ToastKind = "error" | "success" | "info";
export type ToastItem = { id: number; kind: ToastKind; message: string };

const DEFAULT_TTL_MS = 4000;

let items = $state<ToastItem[]>([]);
let nextId = 0;

// id → pending TTL timer. Tracked so `dismiss` can `clearTimeout`; otherwise
// the auto-removal timer keeps running against an array that no longer
// holds the toast — harmless, but every manual dismiss leaks a zombie
// timer for the full TTL window. Important when a screen pushes a burst
// of toasts and the user dismisses them quickly.
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export function toasts(): ToastItem[] {
    return items;
}

function push(kind: ToastKind, message: string, ttl = DEFAULT_TTL_MS) {
    const id = nextId++;
    items.push({ id, kind, message });
    const timer = setTimeout(() => {
        timers.delete(id);
        const idx = items.findIndex(t => t.id === id);
        if (idx >= 0) items.splice(idx, 1);
    }, ttl);
    timers.set(id, timer);
}

export function dismiss(id: number) {
    const timer = timers.get(id);
    if (timer !== undefined) {
        clearTimeout(timer);
        timers.delete(id);
    }
    const idx = items.findIndex(t => t.id === id);
    if (idx >= 0) items.splice(idx, 1);
}

export const toast = {
    error: (msg: string) => push("error", msg),
    success: (msg: string) => push("success", msg),
    info: (msg: string) => push("info", msg),
};
