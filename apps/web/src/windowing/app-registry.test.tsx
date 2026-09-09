import { describe, expect, it, vi } from "vitest";
import * as ReactRuntime from "react";

import {
  OpenMatAppRegistry,
  activateOpenMatAppModule,
  type OpenMatDynamicAppHost,
} from "./app-registry";

function createHost() {
  const registry = new OpenMatAppRegistry();
  const windows = new Set<string>();
  const close = vi.fn((id: string) => windows.delete(id));
  const host: OpenMatDynamicAppHost = {
    apiVersion: "openmat-app-v1",
    react: ReactRuntime,
    apps: {
      register: (definition) => registry.register(definition),
      list: () =>
        registry.list().map(({ id, displayName }) => ({ id, displayName })),
    },
    windows: {
      open: (request) => {
        const id = request.id ?? `${request.appId}:generated`;
        windows.add(id);
        return id;
      },
      close,
      focus: () => undefined,
    },
  };
  return { close, host, registry, windows };
}

describe("OpenMatAppRegistry", () => {
  it("validates ids and sizes and rejects duplicate registrations", () => {
    const registry = new OpenMatAppRegistry();
    const definition = {
      id: "openmat.tool.example",
      displayName: "Example",
      defaultSize: { width: 500, height: 300 },
      minimumSize: { width: 200, height: 120 },
      component: () => <div>Example</div>,
    };
    const unregister = registry.register(definition);

    expect(registry.has(definition.id)).toBe(true);
    expect(() => registry.register(definition)).toThrow(/already registered/u);
    expect(() =>
      registry.register({ ...definition, id: "Not Valid" }),
    ).toThrow(/Invalid OpenMat app id/u);
    expect(() =>
      registry.register({
        ...definition,
        id: "openmat.tool.bad-size",
        minimumSize: { width: 0, height: 120 },
      }),
    ).toThrow(/invalid window size/u);
    expect(() =>
      registry.register({
        ...definition,
        id: "openmat.tool.nan-size",
        defaultSize: { width: Number.NaN, height: 120 },
      }),
    ).toThrow(/invalid window size/u);

    unregister();
    unregister();
    expect(registry.has(definition.id)).toBe(false);
  });
});

describe("activateOpenMatAppModule", () => {
  it("owns module registrations and windows for deterministic unload", async () => {
    const { close, host, registry, windows } = createHost();
    const moduleCleanup = vi.fn();
    const cleanup = await activateOpenMatAppModule(
      {
        registerOpenMatApps(moduleHost) {
          moduleHost.apps.register({
            id: "openmat.tool.dynamic",
            displayName: "Dynamic",
            defaultSize: { width: 480, height: 320 },
            minimumSize: { width: 240, height: 160 },
            component: () => <div>Dynamic</div>,
          });
          moduleHost.windows.open({
            id: "dynamic-window",
            appId: "openmat.tool.dynamic",
            payload: null,
          });
          return moduleCleanup;
        },
      },
      host,
    );

    expect(registry.has("openmat.tool.dynamic")).toBe(true);
    expect(windows).toEqual(new Set(["dynamic-window"]));

    cleanup();
    cleanup();
    expect(moduleCleanup).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenCalledWith("dynamic-window");
    expect(registry.has("openmat.tool.dynamic")).toBe(false);
  });

  it("rolls back partial activation when the module throws", async () => {
    const { host, registry, windows } = createHost();

    await expect(
      activateOpenMatAppModule(
        {
          registerOpenMatApps(moduleHost) {
            moduleHost.apps.register({
              id: "openmat.tool.partial",
              displayName: "Partial",
              defaultSize: { width: 480, height: 320 },
              minimumSize: { width: 240, height: 160 },
              component: () => <div>Partial</div>,
            });
            moduleHost.windows.open({
              id: "partial-window",
              appId: "openmat.tool.partial",
              payload: null,
            });
            throw new Error("activation failed");
          },
        },
        host,
      ),
    ).rejects.toThrow("activation failed");
    expect(registry.has("openmat.tool.partial")).toBe(false);
    expect(windows.size).toBe(0);
  });
});
