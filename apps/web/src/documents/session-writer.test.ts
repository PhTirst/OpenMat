import { describe, expect, it, vi } from "vitest";
import { snapshotDocumentSession } from "./document-session";
import { SessionWriter } from "./session-writer";

describe("ordered session writes", () => {
  it("waits for an older autosave before committing the final exit snapshot", async () => {
    let release!: () => void;
    const slow = new Promise<void>((resolve) => { release = resolve; });
    const save = vi.fn().mockReturnValueOnce(slow).mockResolvedValue(undefined);
    const writer = new SessionWriter({ save, load: vi.fn(), clear: vi.fn() }, "desktop");
    const older = snapshotDocumentSession([], null);
    const latest = snapshotDocumentSession([], null, { rootPath: "C:\\last folder", designerMounted: false, designerVisible: false });
    const first = writer.save(older);
    const final = writer.save(latest);
    expect(save).toHaveBeenCalledTimes(1);
    release();
    await Promise.all([first, final]);
    expect(save).toHaveBeenLastCalledWith("desktop", latest);
  });

  it("allows a final retry after an autosave transaction fails", async () => {
    const save = vi.fn().mockRejectedValueOnce(new Error("disk full")).mockResolvedValue(undefined);
    const writer = new SessionWriter({ save, load: vi.fn(), clear: vi.fn() }, "desktop");
    const session = snapshotDocumentSession([], null);
    const earlier = writer.save(session);
    const retry = writer.save(session);
    await expect(earlier).rejects.toThrow("disk full");
    await expect(retry).resolves.toBeUndefined();
  });
});
