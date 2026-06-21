import { describe, expect, it, vi } from "vitest";
import { isLocalFileDrag, localPathsFromDrop } from "./local-drop.ts";

function dataTransfer(types: string[], data: Record<string, string> = {}, files: unknown[] = []) {
    return {
        types,
        files,
        getData: vi.fn((type: string) => data[type] ?? ""),
    } as unknown as DataTransfer;
}

describe("local file drop helpers", () => {
    it("recognizes file-like drag payloads without matching ordinary text drags", () => {
        expect(isLocalFileDrag(dataTransfer(["Files"]))).toBe(true);
        expect(isLocalFileDrag(dataTransfer(["application/x-moz-file"]))).toBe(true);
        expect(isLocalFileDrag(dataTransfer(["text/uri-list"]))).toBe(false);
        expect(isLocalFileDrag(dataTransfer(["text/plain"]))).toBe(false);
    });

    it("extracts local paths from file URI lists", () => {
        const dt = dataTransfer(["text/uri-list"], {
            "text/uri-list": "# comment\nfile:///home/me/a.txt\r\nfile:///tmp/b%20c.txt\nhttps://example.com/x",
        });
        expect(localPathsFromDrop(dt)).toEqual(["/home/me/a.txt", "/tmp/b c.txt"]);
    });

    it("deduplicates paths exposed by multiple drag formats", () => {
        const dt = dataTransfer(
            ["Files", "text/uri-list"],
            { "text/uri-list": "file:///tmp/a.txt" },
            [{ path: "/tmp/a.txt" }],
        );
        expect(localPathsFromDrop(dt)).toEqual(["/tmp/a.txt"]);
    });
});
