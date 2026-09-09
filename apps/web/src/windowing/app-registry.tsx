import type { ComponentType } from "react";

import type {
  OpenMatWindowBounds,
  OpenMatWindowCloseHandler,
  OpenMatWindowSize,
} from "./window-state";

export interface OpenMatWindowHandle {
  readonly id: string;
  readonly active: boolean;
  close(): void;
  focus(): void;
  minimize(): void;
  maximize(): void;
  restore(): void;
  setTitle(title: string): void;
}

export interface OpenMatAppRenderProps<Payload> {
  readonly payload: Payload;
  readonly window: OpenMatWindowHandle;
}

export interface OpenMatAppDefinition<Payload = unknown> {
  readonly id: string;
  readonly displayName: string;
  readonly defaultSize: OpenMatWindowSize;
  readonly minimumSize: OpenMatWindowSize;
  readonly component: ComponentType<OpenMatAppRenderProps<Payload>>;
}

interface ErasedOpenMatAppDefinition {
  readonly id: string;
  readonly displayName: string;
  readonly defaultSize: OpenMatWindowSize;
  readonly minimumSize: OpenMatWindowSize;
  readonly component: ComponentType<OpenMatAppRenderProps<unknown>>;
}

export class OpenMatAppRegistry {
  readonly #definitions = new Map<string, ErasedOpenMatAppDefinition>();
  readonly #listeners = new Set<() => void>();
  #revision = 0;

  register<Payload>(definition: OpenMatAppDefinition<Payload>): () => void {
    if (!/^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*$/u.test(definition.id)) {
      throw new Error(`Invalid OpenMat app id: ${definition.id}`);
    }
    if (this.#definitions.has(definition.id)) {
      throw new Error(`OpenMat app is already registered: ${definition.id}`);
    }
    if (
      !Number.isFinite(definition.defaultSize.width) ||
      !Number.isFinite(definition.defaultSize.height) ||
      !Number.isFinite(definition.minimumSize.width) ||
      !Number.isFinite(definition.minimumSize.height) ||
      definition.defaultSize.width <= 0 ||
      definition.defaultSize.height <= 0 ||
      definition.minimumSize.width <= 0 ||
      definition.minimumSize.height <= 0
    ) {
      throw new Error(`OpenMat app ${definition.id} has an invalid window size.`);
    }
    const erased: ErasedOpenMatAppDefinition = {
      ...definition,
      component:
        definition.component as ComponentType<OpenMatAppRenderProps<unknown>>,
    };
    this.#definitions.set(definition.id, erased);
    this.#publish();
    return () => {
      if (this.#definitions.get(definition.id) === erased) {
        this.#definitions.delete(definition.id);
        this.#publish();
      }
    };
  }

  has(id: string): boolean {
    return this.#definitions.has(id);
  }

  get(id: string): ErasedOpenMatAppDefinition | undefined {
    return this.#definitions.get(id);
  }

  list(): readonly ErasedOpenMatAppDefinition[] {
    return [...this.#definitions.values()];
  }

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): number => this.#revision;

  #publish(): void {
    this.#revision += 1;
    this.#listeners.forEach((listener) => listener());
  }
}

export interface OpenMatDynamicWindowRequest<Payload = unknown> {
  readonly id?: string;
  readonly appId: string;
  readonly title?: string;
  readonly payload: Payload;
  readonly bounds?: OpenMatWindowBounds;
  readonly ariaDescription?: string;
  readonly closeButtonLabel?: string;
  readonly closeOnEscape?: boolean;
  readonly initialFocus?: "window" | "close";
  readonly onCloseRequested?: OpenMatWindowCloseHandler;
  readonly focusExisting?: boolean;
}

export interface OpenMatDynamicAppHost {
  readonly apiVersion: "openmat-app-v1";
  readonly react: typeof import("react");
  readonly apps: {
    register<Payload>(definition: OpenMatAppDefinition<Payload>): () => void;
    list(): readonly Pick<OpenMatAppDefinition, "id" | "displayName">[];
  };
  readonly windows: {
    open<Payload>(request: OpenMatDynamicWindowRequest<Payload>): string;
    close(id: string): void;
    focus(id: string): void;
  };
}

export interface OpenMatDynamicAppModule {
  registerOpenMatApps(
    host: OpenMatDynamicAppHost,
  ): void | (() => void) | Promise<void | (() => void)>;
}

function dynamicModule(value: unknown): OpenMatDynamicAppModule {
  if (
    typeof value === "object" &&
    value !== null &&
    "registerOpenMatApps" in value &&
    typeof value.registerOpenMatApps === "function"
  ) {
    return value as OpenMatDynamicAppModule;
  }
  if (
    typeof value === "object" &&
    value !== null &&
    "default" in value &&
    typeof value.default === "object" &&
    value.default !== null &&
    "registerOpenMatApps" in value.default &&
    typeof value.default.registerOpenMatApps === "function"
  ) {
    return value.default as OpenMatDynamicAppModule;
  }
  throw new Error(
    "Dynamic OpenMat app module does not export registerOpenMatApps(host).",
  );
}

export async function activateOpenMatAppModule(
  module: OpenMatDynamicAppModule,
  host: OpenMatDynamicAppHost,
): Promise<() => void> {
  const registrations = new Set<() => void>();
  const openedWindows = new Set<string>();
  const scopedHost: OpenMatDynamicAppHost = {
    ...host,
    apps: {
      register: (definition) => {
        const unregisterFromHost = host.apps.register(definition);
        const unregister = (): void => {
          if (registrations.delete(unregister)) {
            unregisterFromHost();
          }
        };
        registrations.add(unregister);
        return unregister;
      },
      list: host.apps.list,
    },
    windows: {
      open: (request) => {
        const id = host.windows.open(request);
        openedWindows.add(id);
        return id;
      },
      close: (id) => {
        openedWindows.delete(id);
        host.windows.close(id);
      },
      focus: host.windows.focus,
    },
  };
  let moduleCleanup: void | (() => void);
  try {
    moduleCleanup = await module.registerOpenMatApps(scopedHost);
  } catch (error: unknown) {
    openedWindows.forEach((id) => host.windows.close(id));
    [...registrations].reverse().forEach((unregister) => unregister());
    throw error;
  }

  let disposed = false;
  return () => {
    if (disposed) {
      return;
    }
    disposed = true;
    let cleanupError: unknown;
    try {
      moduleCleanup?.();
    } catch (error: unknown) {
      cleanupError = error;
    } finally {
      openedWindows.forEach((id) => host.windows.close(id));
      [...registrations].reverse().forEach((unregister) => unregister());
    }
    if (cleanupError !== undefined) {
      throw cleanupError;
    }
  };
}

export async function loadOpenMatAppModule(
  moduleUrl: string,
  host: OpenMatDynamicAppHost,
): Promise<() => void> {
  const loaded: unknown = await import(/* @vite-ignore */ moduleUrl);
  return activateOpenMatAppModule(dynamicModule(loaded), host);
}
