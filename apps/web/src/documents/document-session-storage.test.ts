import { describe, expect, it } from "vitest";
import { documentSessionKey } from "./document-session-storage";

describe("document session identity", () => {
  it("isolates explicit development workspaces from the normal desktop session", () => {
    const normal = documentSessionKey("ws://127.0.0.1:42000/kernel", "desktop");
    const fixture = documentSessionKey("ws://127.0.0.1:42123/kernel", "desktop", "C:\\test workspace");
    expect(fixture).not.toBe(normal);
    expect(fixture).toBe(documentSessionKey("ws://127.0.0.1:42124/kernel", "desktop", "C:\\test workspace"));
  });
  it("keeps desktop drafts in the same session after a configured port change", () => {
    expect(documentSessionKey("ws://127.0.0.1:42000/kernel", "desktop"))
      .toBe(documentSessionKey("ws://127.0.0.1:42123/kernel", "desktop"));
  });

  it("keeps separate Web servers and the browser demo isolated", () => {
    const desktop = documentSessionKey("ws://127.0.0.1:42000/kernel", "desktop");
    const firstServer = documentSessionKey("ws://127.0.0.1:42000/kernel");
    const secondServer = documentSessionKey("ws://127.0.0.1:42123/kernel");
    expect(new Set([desktop, firstServer, secondServer, documentSessionKey(undefined)]).size).toBe(4);
  });
});
