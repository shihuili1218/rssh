import { describe, expect, it } from "vitest";
import { remoteUploadName } from "./sftp-name.ts";

describe("remoteUploadName", () => {
  it("recovers the display name from a SAF ExternalStorage content URI", () => {
    // ACTION_GET_CONTENT on a user-visible provider encodes the path.
    const uri =
      "content://com.android.externalstorage.documents/document/primary%3ADownload%2Freport.pdf";
    expect(remoteUploadName(uri)).toBe("report.pdf");
  });

  it("falls back to the document id for an opaque provider", () => {
    const uri = "content://com.android.providers.downloads.documents/document/msf%3A1234";
    // No display name available — the id is the best we can do (caller may
    // still override; this just must not be empty/garbage).
    expect(remoteUploadName(uri)).toBe("1234");
  });

  it("returns the basename for a plain filesystem path", () => {
    expect(remoteUploadName("/sdcard/Documents/notes.txt")).toBe("notes.txt");
    expect(remoteUploadName("C:\\Users\\me\\key.pem")).toBe("key.pem");
  });

  it("returns empty when nothing usable remains (caller adds a fallback)", () => {
    expect(remoteUploadName("")).toBe("");
    expect(remoteUploadName("/")).toBe("");
  });

  it("does not throw on a malformed percent-escape", () => {
    expect(() => remoteUploadName("content://x/%E0%A4%A")).not.toThrow();
  });
});
