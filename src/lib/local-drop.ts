const FILE_DRAG_TYPES = new Set(["Files", "application/x-moz-file"]);

function dragTypes(dt: DataTransfer | null | undefined): string[] {
    return Array.from(dt?.types ?? []);
}

export function isLocalFileDrag(dt: DataTransfer | null | undefined): boolean {
    return dragTypes(dt).some((type) => FILE_DRAG_TYPES.has(type));
}

function fileUriToPath(uri: string): string | null {
    try {
        const url = new URL(uri);
        if (url.protocol !== "file:") return null;
        const path = decodeURIComponent(url.pathname);
        return url.hostname ? `//${url.hostname}${path}` : path.replace(/^\/([A-Za-z]:\/)/, "$1");
    } catch {
        return null;
    }
}

function pathsFromUriList(raw: string): string[] {
    return raw
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line && !line.startsWith("#"))
        .map(fileUriToPath)
        .filter((path): path is string => !!path);
}

export function localPathsFromDrop(dt: DataTransfer | null | undefined): string[] {
    const paths: string[] = [];
    for (const file of Array.from(dt?.files ?? [])) {
        const path = (file as File & { path?: string }).path;
        if (path) paths.push(path);
    }
    try {
        paths.push(...pathsFromUriList(dt?.getData("text/uri-list") ?? ""));
    } catch {
        /* Some webviews throw when reading unavailable drag data. */
    }
    return [...new Set(paths)];
}

export function installLocalFileDropNavigationGuard(target: Window = window): () => void {
    const preventDragOver = (event: DragEvent) => {
        if (!isLocalFileDrag(event.dataTransfer)) return;
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    };
    const preventDrop = (event: DragEvent) => {
        const paths = localPathsFromDrop(event.dataTransfer);
        if (!isLocalFileDrag(event.dataTransfer) && paths.length === 0) return;
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    };
    target.addEventListener("dragover", preventDragOver, { capture: true });
    target.addEventListener("drop", preventDrop, { capture: true });
    return () => {
        target.removeEventListener("dragover", preventDragOver, { capture: true });
        target.removeEventListener("drop", preventDrop, { capture: true });
    };
}
