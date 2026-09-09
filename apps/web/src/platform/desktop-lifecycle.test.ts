import { describe, expect, it, vi } from "vitest";
import { createDesktopLifecycle } from "./desktop-lifecycle";

describe("desktop close boundary", () => {
  it("prevents the native close before asking the workbench and only destroys explicitly", async () => {
    let request: ((event: { preventDefault(): void }) => void) | undefined;
    const remove = vi.fn();
    const destroy = vi.fn().mockResolvedValue(undefined);
    const lifecycle = createDesktopLifecycle(async () => ({
      onCloseRequested: async (handler) => { request = handler; return remove; }, destroy,
    }));
    const preventDefault = vi.fn();
    const workbench = vi.fn(() => expect(preventDefault).toHaveBeenCalledOnce());
    const unlisten = await lifecycle.onCloseRequested(workbench);
    request!({ preventDefault });
    expect(workbench).toHaveBeenCalledOnce();
    expect(destroy).not.toHaveBeenCalled();
    await lifecycle.close();
    expect(destroy).toHaveBeenCalledOnce();
    unlisten();
    expect(remove).toHaveBeenCalledOnce();
  });
});
