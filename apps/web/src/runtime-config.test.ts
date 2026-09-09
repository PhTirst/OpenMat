import { afterEach, describe, expect, it, vi } from "vitest";
import { kernelWebSocketUrl } from "./runtime-config";

function setRuntimeConfig(kernelWebSocketUrlValue?: string): void {
  Object.defineProperty(window, "__OPENMAT_RUNTIME__", {
    configurable: true,
    value:
      kernelWebSocketUrlValue === undefined
        ? undefined
        : { kernelWebSocketUrl: kernelWebSocketUrlValue },
  });
}

afterEach(() => {
  setRuntimeConfig();
  vi.unstubAllEnvs();
});

describe("kernelWebSocketUrl", () => {
  it("prefers a native runtime endpoint injected before module execution", () => {
    vi.stubEnv("VITE_OPENMAT_WS_URL", "ws://127.0.0.1:41000/kernel");
    setRuntimeConfig(" ws://127.0.0.1:42000/kernel ");

    expect(kernelWebSocketUrl()).toBe("ws://127.0.0.1:42000/kernel");
  });

  it("falls back to the Vite endpoint used by browser development", () => {
    vi.stubEnv("VITE_OPENMAT_WS_URL", " ws://127.0.0.1:43000/kernel ");

    expect(kernelWebSocketUrl()).toBe("ws://127.0.0.1:43000/kernel");
  });
});
