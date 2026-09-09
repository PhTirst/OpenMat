import { Blob as NodeBlob } from "node:buffer";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createDesktopPlatform, createPlatformServices, nativeFileKey, type NativeInvoke } from "./platform-services";

afterEach(() => {
  Object.defineProperty(window, "__OPENMAT_RUNTIME__", { configurable: true, value: undefined });
  vi.restoreAllMocks();
});

describe("platform services", () => {
  it("matches Windows file identities across separators and extended prefixes", () => {
    expect(nativeFileKey("\\\\?\\C:\\Work\\a.m")).toBe(nativeFileKey("c:/work/a.m"));
    expect(nativeFileKey("\\\\?\\UNC\\server\\share\\a.m")).toBe(nativeFileKey("//server/share/a.m"));
    expect(nativeFileKey("/work/A.m")).not.toBe(nativeFileKey("/work/a.m"));
  });
  it("uses browser services even when a browser connects to a localhost kernel", () => {
    Object.defineProperty(window, "__OPENMAT_RUNTIME__", { configurable: true, value: { kernelWebSocketUrl: "ws://127.0.0.1:42000/kernel" } });
    expect(createPlatformServices().kind).toBe("web");
    expect(createPlatformServices().files).toBeUndefined();
  });

  it("requires the explicit supported desktop bridge version", () => {
    Object.defineProperty(window, "__OPENMAT_RUNTIME__", { configurable: true, value: { platform: "desktop", nativeFileApiVersion: 1 } });
    expect(createPlatformServices().kind).toBe("desktop");
    Object.defineProperty(window, "__OPENMAT_RUNTIME__", { configurable: true, value: { platform: "desktop", nativeFileApiVersion: 2 } });
    expect(createPlatformServices).toThrow("unsupported native file API");
  });

  it("sends UTF-8 bytes and structured paths to the native save command", async () => {
    const invoke = vi.fn<NativeInvoke>().mockResolvedValue(null);
    const platform = createDesktopPlatform(invoke as NativeInvoke);
    expect(await platform.files!.saveTextFile("计算.m", "C:\\my work", "结果 = '你好';\n")).toBeNull();
    const [command, args] = invoke.mock.calls[0]!;
    expect(command).toBe("desktop_save_file");
    expect(args.suggestedName).toBe("计算.m");
    expect(args.initialDirectory).toBe("C:\\my work");
    expect(new TextDecoder().decode(new Uint8Array(args.contents as number[]))).toBe("结果 = '你好';\n");
  });

  it("returns export cancellation without starting a browser download", async () => {
    const invoke = vi.fn<NativeInvoke>().mockResolvedValue(null);
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click");
    const blob = new NodeBlob([new Uint8Array([0, 128, 255])]) as unknown as Blob;
    expect(await createDesktopPlatform(invoke as NativeInvoke).saveExport(blob, "Figure-1.png")).toBe(false);
    expect(invoke.mock.calls[0]?.[1].contents).toEqual([0, 128, 255]);
    expect(click).not.toHaveBeenCalled();
  });

  it("rejects oversized exports before copying the blob or invoking a dialog", async () => {
    const invoke = vi.fn<NativeInvoke>();
    const arrayBuffer = vi.fn();
    await expect(createDesktopPlatform(invoke as NativeInvoke).saveExport({ size: 64 * 1024 * 1024 + 1, arrayBuffer } as unknown as Blob, "large.png")).rejects.toThrow("64 MiB");
    expect(arrayBuffer).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });
});
