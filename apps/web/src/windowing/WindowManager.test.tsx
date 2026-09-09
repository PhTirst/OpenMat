import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect } from "react";
import { describe, expect, it, vi } from "vitest";

import { OpenMatAppRegistry, type OpenMatDynamicAppModule } from "./app-registry";
import {
  OpenMatDynamicAppLoader,
  OpenMatWindowLayer,
  OpenMatWindowManagerProvider,
  useOpenMatWindowManagerActions,
  type OpenMatWindowManagerActions,
} from "./WindowManager";

const TEST_APP_ID = "openmat.test.window";

class TestPointerEvent extends MouseEvent {
  readonly pointerId: number;

  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 0;
  }
}

Object.defineProperty(window, "PointerEvent", {
  configurable: true,
  value: TestPointerEvent,
});

function createRegistry(): OpenMatAppRegistry {
  const registry = new OpenMatAppRegistry();
  registry.register<{ readonly label: string }>({
    id: TEST_APP_ID,
    displayName: "Test Window",
    defaultSize: { width: 400, height: 300 },
    minimumSize: { width: 240, height: 160 },
    component: ({ payload, window }) => (
      <div>
        <span>{payload.label}</span>
        <button type="button" onClick={() => window.setTitle(`${payload.label} renamed`)}>
          Rename
        </button>
      </div>
    ),
  });
  return registry;
}

function OpenTwoWindows({
  exposeActions,
}: {
  readonly exposeActions?: (actions: OpenMatWindowManagerActions) => void;
}) {
  const actions = useOpenMatWindowManagerActions();
  useEffect(() => {
    exposeActions?.(actions);
    actions.openWindow({
      id: "first",
      appId: TEST_APP_ID,
      title: "First",
      payload: { label: "First content" },
    });
    actions.openWindow({
      id: "second",
      appId: TEST_APP_ID,
      title: "Second",
      payload: { label: "Second content" },
    });
  }, [actions, exposeActions]);
  return null;
}

function renderManager(children: React.ReactNode, registry = createRegistry()) {
  return render(
    <OpenMatWindowManagerProvider registry={registry}>
      {children}
      <OpenMatWindowLayer />
    </OpenMatWindowManagerProvider>,
  );
}

describe("OpenMatWindowManager", () => {
  it("cascades, focuses, minimizes, maximizes, restores, and closes windows", async () => {
    renderManager(<OpenTwoWindows />);
    const first = await screen.findByRole("dialog", { name: "First" });
    const second = await screen.findByRole("dialog", { name: "Second" });

    expect(first).toHaveStyle({ left: "28px", top: "28px", zIndex: "100" });
    expect(second).toHaveStyle({ left: "56px", top: "56px", zIndex: "101" });
    expect(second).toHaveClass("active");

    fireEvent.pointerDown(first);
    await waitFor(() => {
      expect(first).toHaveClass("active");
      expect(first).toHaveStyle({ zIndex: "101" });
    });

    fireEvent.click(within(first).getByRole("button", { name: "Minimize First" }));
    expect(screen.queryByRole("dialog", { name: "First" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "First" }));
    expect(await screen.findByRole("dialog", { name: "First" })).toBeVisible();

    fireEvent.click(within(second).getByRole("button", { name: "Maximize Second" }));
    expect(second).toHaveAttribute("data-window-mode", "maximized");
    fireEvent.click(within(second).getByRole("button", { name: "Restore Second" }));
    expect(second).toHaveAttribute("data-window-mode", "normal");

    fireEvent.click(within(second).getByRole("button", { name: "Close Second" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Second" })).toBeNull(),
    );
  });

  it("commits pointer drag and resize interactions and supports keyboard transforms", async () => {
    renderManager(<OpenTwoWindows />);
    const first = await screen.findByRole("dialog", { name: "First" });
    const titlebar = within(first).getByLabelText(/Alt plus arrow keys move/u);

    fireEvent.pointerDown(titlebar, {
      button: 0,
      pointerId: 7,
      clientX: 100,
      clientY: 100,
    });
    expect(titlebar).toHaveFocus();
    fireEvent.pointerMove(titlebar, {
      pointerId: 7,
      clientX: 180,
      clientY: 150,
    });
    fireEvent.pointerUp(titlebar, {
      pointerId: 7,
      clientX: 180,
      clientY: 150,
    });
    await waitFor(() => expect(first).toHaveStyle({ left: "108px", top: "78px" }));

    const southEast = first.querySelector<HTMLElement>(
      '[data-resize-handle="south-east"]',
    );
    expect(southEast).not.toBeNull();
    fireEvent.pointerDown(southEast!, {
      button: 0,
      pointerId: 8,
      clientX: 0,
      clientY: 0,
    });
    fireEvent.pointerMove(southEast!, {
      pointerId: 8,
      clientX: 90,
      clientY: 40,
    });
    fireEvent.pointerUp(southEast!, {
      pointerId: 8,
      clientX: 90,
      clientY: 40,
    });
    await waitFor(() => expect(first).toHaveStyle({ width: "490px", height: "340px" }));

    fireEvent.keyDown(titlebar, { key: "ArrowLeft", altKey: true });
    await waitFor(() => expect(first).toHaveStyle({ left: "100px" }));
    fireEvent.keyDown(titlebar, {
      key: "ArrowDown",
      altKey: true,
      ctrlKey: true,
      shiftKey: true,
    });
    await waitFor(() => expect(first).toHaveStyle({ height: "364px" }));
  });

  it("honors cancel and defer close decisions without invoking a handler twice", async () => {
    let actions: OpenMatWindowManagerActions | undefined;
    const cancel = vi.fn(() => "cancel" as const);
    renderManager(
      <OpenTwoWindows exposeActions={(value) => (actions = value)} />,
    );
    const first = await screen.findByRole("dialog", { name: "First" });
    actions?.updateWindow("first", {
      closeButtonLabel: "Cancel-close First",
      onCloseRequested: cancel,
    });
    const closeButton = await within(first).findByRole("button", {
      name: "Cancel-close First",
    });
    fireEvent.click(closeButton);
    fireEvent.click(closeButton);
    await waitFor(() => expect(closeButton).toBeEnabled());
    expect(cancel).toHaveBeenCalledTimes(1);

    const defer = vi.fn(() => "defer" as const);
    actions?.updateWindow("first", {
      closeButtonLabel: "Deferred-close First",
      onCloseRequested: defer,
    });
    const deferredCloseButton = await within(first).findByRole("button", {
      name: "Deferred-close First",
    });
    fireEvent.click(deferredCloseButton);
    await waitFor(() => expect(deferredCloseButton).toBeDisabled());
    expect(defer).toHaveBeenCalledTimes(1);
    actions?.closeWindow("first");
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "First" })).toBeNull(),
    );
  });

  it("queues modal dialogs above windows, traps focus, restores it, and dismisses with Escape", async () => {
    let actions: OpenMatWindowManagerActions | undefined;
    renderManager(<OpenTwoWindows exposeActions={(value) => (actions = value)} />);
    const source = await within(
      await screen.findByRole("dialog", { name: "Second" }),
    ).findByRole("button", { name: "Rename" });
    source.focus();

    let first!: Promise<"accepted" | "cancelled">;
    let second!: Promise<"finished" | "cancelled">;
    act(() => {
      first = actions!.openDialog<"accepted" | "cancelled">({
        label: "First modal",
        dismissResult: "cancelled" as const,
        render: ({ close }) => (
          <section role="dialog" aria-label="First modal" tabIndex={-1}>
            <button data-dialog-initial-focus type="button">
              First action
            </button>
            <button type="button" onClick={() => close("accepted")}>
              Accept first
            </button>
          </section>
        ),
      });
      second = actions!.openDialog<"finished" | "cancelled">({
        label: "Second modal",
        dismissResult: "cancelled" as const,
        render: ({ close }) => (
          <section role="dialog" aria-label="Second modal" tabIndex={-1}>
            <button type="button" onClick={() => close("finished")}>
              Finish second
            </button>
          </section>
        ),
      });
    });

    const firstModal = await screen.findByRole("dialog", { name: "First modal" });
    expect(screen.queryByRole("dialog", { name: "Second modal" })).toBeNull();
    expect(firstModal.parentElement).toHaveAttribute("data-dialog-queue-length", "2");
    const firstAction = within(firstModal).getByRole("button", {
      name: "First action",
    });
    const accept = within(firstModal).getByRole("button", { name: "Accept first" });
    expect(firstAction).toHaveFocus();
    accept.focus();
    fireEvent.keyDown(accept, { key: "Tab" });
    expect(firstAction).toHaveFocus();
    fireEvent.click(accept);
    await expect(first).resolves.toBe("accepted");

    const secondModal = await screen.findByRole("dialog", { name: "Second modal" });
    fireEvent.keyDown(secondModal, { key: "Escape" });
    await expect(second).resolves.toBe("cancelled");
    await waitFor(() => expect(source).toHaveFocus());
  });

  it("loads a dynamic React app and opens it through the versioned host", async () => {
    const registry = createRegistry();
    const dynamicModule: OpenMatDynamicAppModule = {
      registerOpenMatApps(host) {
        expect(host.apiVersion).toBe("openmat-app-v1");
        host.apps.register<{ readonly answer: number }>({
          id: "openmat.tool.dynamic-ui",
          displayName: "Dynamic UI",
          defaultSize: { width: 420, height: 260 },
          minimumSize: { width: 240, height: 160 },
          component: ({ payload }) => {
            const [answer, setAnswer] = host.react.useState(payload.answer);
            return host.react.createElement(
              "button",
              { type: "button", onClick: () => setAnswer((value) => value + 1) },
              `Answer: ${answer.toString()}`,
            );
          },
        });
        host.windows.open({
          id: "dynamic-ui",
          appId: "openmat.tool.dynamic-ui",
          payload: { answer: 42 },
        });
      },
    };
    const sources = [dynamicModule] as const;
    const result = renderManager(
      <OpenMatDynamicAppLoader sources={sources} />,
      registry,
    );

    const dialog = await screen.findByRole("dialog", { name: "Dynamic UI" });
    const answer = within(dialog).getByRole("button", { name: "Answer: 42" });
    fireEvent.click(answer);
    expect(within(dialog).getByRole("button", { name: "Answer: 43" })).toBeVisible();
    expect(registry.has("openmat.tool.dynamic-ui")).toBe(true);

    result.unmount();
    expect(registry.has("openmat.tool.dynamic-ui")).toBe(false);
  });
});
