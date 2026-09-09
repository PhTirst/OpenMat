import { createContext, useContext } from "react";
import { createDesktopLifecycle, type DesktopLifecycle } from "./desktop-lifecycle";
import { PendingOperations } from "./pending-operations";

export interface NativeFileLocation {
  readonly path: string;
  readonly directory: string;
  readonly name: string;
}

/** Compare host paths even when a tab was opened from a different Current Folder. */
export function nativeFileKey(path: string): string {
  const normalized = path.replaceAll("\\", "/").replace(/^\/\/\?\/UNC\//i, "//").replace(/^\/\/\?\//, "");
  return /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : normalized;
}

export interface DesktopFiles {
  pickDirectory(initialDirectory: string): Promise<string | null>;
  pickFile(initialDirectory: string): Promise<NativeFileLocation | null>;
  revealPath(rootPath: string, relativePath: string): Promise<void>;
  saveTextFile(name: string, directory: string, content: string): Promise<NativeFileLocation | null>;
}

export interface PlatformServices {
  readonly kind: "web" | "desktop";
  readonly files?: DesktopFiles;
  readonly lifecycle?: DesktopLifecycle;
  readonly restoreWorkspace?: boolean;
  readonly workspaceIdentity?: string;
  waitForWrites?(): Promise<void>;
  /** False means that the user canceled the native save dialog. */
  saveExport(blob: Blob, fileName: string): Promise<boolean>;
}

export function downloadBlob(blob: Blob, fileName: string): void {
  const url = URL.createObjectURL(blob);
  const revoke = URL.revokeObjectURL.bind(URL);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  anchor.hidden = true;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => revoke(url), 0);
}

const webPlatform: PlatformServices = {
  kind: "web",
  async saveExport(blob, fileName) {
    downloadBlob(blob, fileName);
    return true;
  },
};

export type NativeInvoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

async function invokeNative<T>(command: string, args: Record<string, unknown>): Promise<T> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<T>(command, args);
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}

export function createDesktopPlatform(invoke: NativeInvoke = invokeNative): PlatformServices {
  const operations = new PendingOperations();
  const save = (name: string, directory: string | null, contents: Uint8Array) =>
    operations.run(() => invoke<NativeFileLocation | null>("desktop_save_file", {
      suggestedName: name, initialDirectory: directory, contents: Array.from(contents),
    }));
  return {
    kind: "desktop",
    lifecycle: createDesktopLifecycle(),
    waitForWrites: () => operations.wait(),
    restoreWorkspace: window.__OPENMAT_RUNTIME__?.restoreWorkspace !== false,
    ...(window.__OPENMAT_RUNTIME__?.workspaceIdentity === undefined ? {} : { workspaceIdentity: window.__OPENMAT_RUNTIME__.workspaceIdentity }),
    files: {
      pickDirectory: (initialDirectory) => invoke("desktop_pick_directory", { initialDirectory }),
      pickFile: (initialDirectory) => invoke("desktop_pick_file", { initialDirectory }),
      revealPath: (rootPath, relativePath) => invoke("desktop_reveal_path", { rootPath, relativePath }),
      saveTextFile: (name, directory, content) => save(name, directory, new TextEncoder().encode(content)),
    },
    async saveExport(blob, fileName) {
      // Reject before copying a potentially large image into the IPC request.
      if (blob.size > 64 * 1024 * 1024) throw new Error("Native exports are limited to 64 MiB");
      return await save(fileName, null, new Uint8Array(await blob.arrayBuffer())) !== null;
    },
  };
}

export function createPlatformServices(): PlatformServices {
  const config = window.__OPENMAT_RUNTIME__;
  if (config?.platform !== "desktop") return webPlatform;
  if (config.nativeFileApiVersion !== 1) {
    throw new Error("This desktop host uses an unsupported native file API. Update OpenMat Desktop.");
  }
  return createDesktopPlatform();
}

export const PlatformContext = createContext<PlatformServices>(webPlatform);
export const usePlatformServices = (): PlatformServices => useContext(PlatformContext);
