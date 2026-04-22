type ToastKind = "error" | "success" | "info";
export type ToastItem = { id: number; kind: ToastKind; message: string };

const DEFAULT_TTL_MS = 4000;

let items = $state<ToastItem[]>([]);
let nextId = 0;

export function toasts(): ToastItem[] {
    return items;
}

function push(kind: ToastKind, message: string, ttl = DEFAULT_TTL_MS) {
    const id = nextId++;
    items.push({ id, kind, message });
    setTimeout(() => {
        const idx = items.findIndex(t => t.id === id);
        if (idx >= 0) items.splice(idx, 1);
    }, ttl);
}

export function dismiss(id: number) {
    const idx = items.findIndex(t => t.id === id);
    if (idx >= 0) items.splice(idx, 1);
}

export const toast = {
    error: (msg: string) => push("error", msg),
    success: (msg: string) => push("success", msg),
    info: (msg: string) => push("info", msg),
};
