import { describe, expect, it } from "vitest";
import { WebSocketLspClient } from "./client";

class FakeWebSocket {
  readyState = 0;
  readonly sent: string[] = [];
  readonly closes: Array<{ code?: number; reason?: string }> = [];
  onopen: ((event: Event) => unknown) | null = null;
  onclose: ((event: CloseEvent) => unknown) | null = null;
  onerror: ((event: Event) => unknown) | null = null;
  onmessage: ((event: MessageEvent) => unknown) | null = null;

  send(text: string): void {
    this.sent.push(text);
  }

  close(code?: number, reason?: string): void {
    this.closes.push({ ...(code === undefined ? {} : { code }), ...(reason === undefined ? {} : { reason }) });
    this.readyState = 3;
    this.onclose?.({} as CloseEvent);
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.({} as Event);
  }

  message(value: unknown): void {
    this.onmessage?.({ data: JSON.stringify(value) } as MessageEvent);
  }

  peerClose(): void {
    this.readyState = 3;
    this.onclose?.({} as CloseEvent);
  }
}

const sentMessage = (socket: FakeWebSocket, index: number): Record<string, unknown> =>
  JSON.parse(socket.sent[index] ?? "null") as Record<string, unknown>;

describe("WebSocketLspClient", () => {
  it("initializes, correlates out-of-order requests, forwards notifications, and shuts down", async () => {
    const socket = new FakeWebSocket();
    const client = new WebSocketLspClient(
      "ws://127.0.0.1:49152/lsp",
      () => socket as unknown as WebSocket,
    );
    const states: string[] = [];
    const notifications: string[] = [];
    client.onStateChange((state) => states.push(state));
    client.onNotification((notification) => notifications.push(notification.method));

    const connecting = client.connect();
    socket.open();
    await Promise.resolve();
    const initialize = sentMessage(socket, 0);
    expect(initialize.method).toBe("initialize");
    expect(initialize.id).toBe(1);
    expect(initialize.params).toMatchObject({ capabilities: {
      workspace: { workspaceEdit: { documentChanges: true, resourceOperations: ["rename"] } },
    } });
    socket.message({
      jsonrpc: "2.0",
      id: 1,
      result: { capabilities: {}, serverInfo: { name: "openmat-lsp", version: "0.1.0" } },
    });
    await connecting;
    expect(sentMessage(socket, 1).method).toBe("initialized");
    expect(client.state).toBe("ready");

    const completion = client.request<string>("textDocument/completion", { prefix: "cal" });
    const hover = client.request<string>("textDocument/hover", { symbol: "calculate" });
    await Promise.resolve();
    const completionRequest = sentMessage(socket, 2);
    const hoverRequest = sentMessage(socket, 3);
    socket.message({ jsonrpc: "2.0", id: hoverRequest.id, result: "hover-result" });
    socket.message({ jsonrpc: "2.0", id: completionRequest.id, result: "completion-result" });
    await expect(completion).resolves.toBe("completion-result");
    await expect(hover).resolves.toBe("hover-result");

    socket.message({
      jsonrpc: "2.0",
      method: "textDocument/publishDiagnostics",
      params: { uri: "file:///demo.m", diagnostics: [] },
    });
    expect(notifications).toEqual(["textDocument/publishDiagnostics"]);

    const disconnecting = client.disconnect();
    await Promise.resolve();
    const shutdown = sentMessage(socket, 4);
    expect(shutdown.method).toBe("shutdown");
    socket.message({ jsonrpc: "2.0", id: shutdown.id, result: null });
    await disconnecting;
    expect(sentMessage(socket, 5).method).toBe("exit");
    expect(socket.closes[0]?.code).toBe(1000);
    expect(client.state).toBe("disconnected");
    expect(states).toContain("connecting");
    expect(states).toContain("ready");
  });

  it("rejects pending requests when the peer disappears", async () => {
    const socket = new FakeWebSocket();
    const client = new WebSocketLspClient(
      "ws://127.0.0.1:49152/lsp",
      () => socket as unknown as WebSocket,
    );
    const connecting = client.connect();
    socket.open();
    await Promise.resolve();
    socket.message({ jsonrpc: "2.0", id: 1, result: { capabilities: {} } });
    await connecting;

    const pending = client.request("textDocument/completion", {});
    await Promise.resolve();
    socket.peerClose();
    await expect(pending).rejects.toThrow("connection was lost");
    expect(client.state).toBe("disconnected");
  });

  it("enforces the 1 MiB outbound JSON-RPC limit", async () => {
    const socket = new FakeWebSocket();
    const client = new WebSocketLspClient(
      "ws://127.0.0.1:49152/lsp",
      () => socket as unknown as WebSocket,
    );
    const connecting = client.connect();
    socket.open();
    await Promise.resolve();
    socket.message({ jsonrpc: "2.0", id: 1, result: { capabilities: {} } });
    await connecting;

    await expect(
      client.request("textDocument/didSomething", { text: "x".repeat(1024 * 1024) }),
    ).rejects.toThrow("exceeds 1 MiB");
  });
});
