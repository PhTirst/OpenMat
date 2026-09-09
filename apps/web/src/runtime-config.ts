export interface OpenMatRuntimeConfig {
  readonly kernelWebSocketUrl?: string;
  readonly platform?: "desktop";
  readonly nativeFileApiVersion?: number;
  readonly restoreWorkspace?: boolean;
  readonly workspaceIdentity?: string;
}

declare global {
  interface Window {
    readonly __OPENMAT_RUNTIME__?: OpenMatRuntimeConfig;
  }
}

function clean(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed === undefined || trimmed.length === 0 ? undefined : trimmed;
}

/**
 * Resolves the native host's runtime endpoint before falling back to Vite's
 * development-time configuration. Tauri injects the former before any module
 * executes, while the browser development workflow keeps using the latter.
 */
export function kernelWebSocketUrl(): string | undefined {
  return clean(window.__OPENMAT_RUNTIME__?.kernelWebSocketUrl)
    ?? clean(import.meta.env.VITE_OPENMAT_WS_URL);
}
