export const DESKTOP_CLOSE_PREPARE_EVENT = "openmat:prepare-close";

export interface DesktopLifecycle {
  onCloseRequested(handler: () => void): Promise<() => void>;
  close(): Promise<void>;
}

interface DesktopWindow {
  onCloseRequested(handler: (event: { preventDefault(): void }) => void): Promise<() => void>;
  destroy(): Promise<void>;
}

async function currentWindow(): Promise<DesktopWindow> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export function createDesktopLifecycle(
  getWindow: () => Promise<DesktopWindow> = currentWindow,
): DesktopLifecycle {
  return {
    async onCloseRequested(handler) {
      return (await getWindow()).onCloseRequested((event) => {
        // Keep the WebView alive until saves and the recovery transaction finish.
        event.preventDefault();
        handler();
      });
    },
    async close() {
      await (await getWindow()).destroy();
    },
  };
}
