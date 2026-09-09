import { describe, expect, it } from "vitest";
import {
  KERNEL_PROTOCOL,
  createKernelRequest,
} from "../protocol/kernel-v0";
import {
  KERNEL_PROTOCOL_V1,
  MAX_PREVIEW_CODE_UNITS,
  MAX_STRING_ELEMENT_CODE_UNITS,
  SUPPORTED_KERNEL_PROTOCOLS,
  createBootstrapInitializeRequest,
  createKernelRequest as createVersionedKernelRequest,
  type KernelProtocol,
  type V1Capabilities,
} from "../protocol/kernel-v1";
import {
  KERNEL_PROTOCOL_V2,
  KERNEL_PROTOCOL_V3,
  createBootstrapInitializeRequest as createV2BootstrapInitializeRequest,
  createKernelRequest as createV2KernelRequest,
  type KernelRequest as V2KernelRequest,
  type V2Capabilities,
} from "../protocol/kernel-v2";
import {
  DEFAULT_MAX_MESSAGE_BYTES,
  KernelTransportError,
  WebSocketKernelTransport,
} from "./websocket-kernel-transport";

class FakeSocket extends EventTarget {
  readyState: number = WebSocket.CONNECTING;
  binaryType: BinaryType = "blob";
  readonly sent: string[] = [];
  closeCode: number | null = null;
  closeReason: string | null = null;

  open(): void {
    this.readyState = WebSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(code = 1000, reason = ""): void {
    this.closeCode = code;
    this.closeReason = reason;
    this.readyState = WebSocket.CLOSED;
    this.dispatchEvent(new CloseEvent("close", { code, reason }));
  }

  receive(data: unknown): void {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
}

class FakeKernelServer {
  readonly socket = new FakeSocket();
  responseSequence = 0;

  factory = (_url: string): WebSocket => this.socket as unknown as WebSocket;

  open(): void {
    this.socket.open();
  }

  requests(): V2KernelRequest[] {
    return this.socket.sent.map((frame) => JSON.parse(frame) as V2KernelRequest);
  }

  send(value: unknown): void {
    this.socket.receive(JSON.stringify(value));
  }

  respond(
    request: V2KernelRequest,
    result: { readonly type: string; readonly data: unknown },
  ): void {
    this.responseSequence += 1;
    this.send({
      protocol: request.protocol,
      sessionId: request.sessionId,
      messageId: `server-response-${this.responseSequence.toString()}`,
      kind: "response",
      replyTo: request.messageId,
      ok: true,
      result,
    });
  }
}

const v1Capabilities: V1Capabilities = {
  executionModes: ["cell"],
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 128,
  maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
  maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
  interrupt: true,
  workspaceDelta: true,
};

const v0Capabilities = {
  executionModes: ["cell" as const],
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 128,
  interrupt: true,
  workspaceDelta: true,
};

const v2Capabilities: V2Capabilities = {
  ...v1Capabilities,
  maxPreviewElements: 4,
  maxStringElementCodeUnits: 16,
  maxPreviewCodeUnits: 64,
  maxAggregateNodes: 16,
  maxAggregateElements: 32,
  maxAggregateDepth: 4,
};

async function negotiate(
  transport: WebSocketKernelTransport,
  server: FakeKernelServer,
  protocol: KernelProtocol,
  responseCapabilities: unknown,
) {
  const initialization = transport.request(
    createBootstrapInitializeRequest(
      "session-1",
      "initialize-1",
      {
        client: { name: "web-test", version: "1" },
        supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
        capabilities: v1Capabilities,
      },
    ),
  );
  const request = server.requests().at(-1)!;
  server.respond(request, {
    type: "initialize",
    data: {
      negotiatedProtocol: protocol,
      implementation: { name: "openmat-kernel", version: "1" },
      capabilities: responseCapabilities,
    },
  });
  return initialization;
}

async function negotiateV2(
  transport: WebSocketKernelTransport,
  server: FakeKernelServer,
) {
  const initialization = transport.request(
    createV2BootstrapInitializeRequest(
      "session-1",
      "initialize-v2",
      {
        client: { name: "web-test", version: "2" },
        supportedProtocols: [
          KERNEL_PROTOCOL_V2,
          KERNEL_PROTOCOL_V1,
          KERNEL_PROTOCOL,
        ],
        capabilities: v2Capabilities,
      },
    ),
  );
  const request = server.requests().at(-1)!;
  server.respond(request, {
    type: "initialize",
    data: {
      negotiatedProtocol: KERNEL_PROTOCOL_V2,
      implementation: { name: "openmat-kernel", version: "2" },
      capabilities: v2Capabilities,
    },
  });
  return initialization;
}

async function negotiateV3(
  transport: WebSocketKernelTransport,
  server: FakeKernelServer,
) {
  const initialization = transport.request(
    createV2BootstrapInitializeRequest(
      "session-1",
      "initialize-v3",
      {
        client: { name: "web-test", version: "3" },
        supportedProtocols: [KERNEL_PROTOCOL_V3],
        capabilities: v2Capabilities,
      },
    ),
  );
  const request = server.requests().at(-1)!;
  server.respond(request, {
    type: "initialize",
    data: {
      negotiatedProtocol: KERNEL_PROTOCOL_V3,
      implementation: { name: "openmat-kernel", version: "3" },
      capabilities: v2Capabilities,
    },
  });
  return initialization;
}

async function connect(server: FakeKernelServer, maxMessageBytes?: number) {
  const options =
    maxMessageBytes === undefined
      ? { webSocketFactory: server.factory }
      : { webSocketFactory: server.factory, maxMessageBytes };
  const transport = new WebSocketKernelTransport(
    "ws://127.0.0.1:8765/kernel",
    options,
  );
  const connection = transport.connect("session-1");
  server.open();
  await connection;
  return transport;
}

describe("WebSocketKernelTransport", () => {
  it("correlates v3 revision-checked element writes and conflict failures", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    await expect(negotiateV3(transport, server)).resolves.toMatchObject({
      result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V3 } },
    });

    const write = transport.request(
      createV2KernelRequest(
        KERNEL_PROTOCOL_V3,
        "session-1",
        "set-v3",
        "setVariableElement",
        {
          name: "A",
          indices: [2, 1],
          value: { real: "42", imaginary: "0" },
          expectedRevision: 5,
        },
      ),
    );
    expect(server.requests().at(-1)).toMatchObject({
      protocol: KERNEL_PROTOCOL_V3,
      request: {
        type: "setVariableElement",
        params: { expectedRevision: 5, indices: [2, 1] },
      },
    });
    server.respond(server.requests().at(-1)!, {
      type: "setVariableElement",
      data: {
        revision: 6,
        variable: {
          name: "A",
          class: "double",
          dimensions: [2, 2],
          complex: false,
          bytes: 32,
        },
      },
    });
    await expect(write).resolves.toMatchObject({
      result: { data: { revision: 6 } },
    });

    const stale = transport.request(
      createV2KernelRequest(
        KERNEL_PROTOCOL_V3,
        "session-1",
        "stale-v3",
        "setVariableElement",
        {
          name: "A",
          indices: [2, 1],
          value: { real: "9", imaginary: "0" },
          expectedRevision: 5,
        },
      ),
    );
    const staleRequest = server.requests().at(-1)!;
    server.send({
      protocol: KERNEL_PROTOCOL_V3,
      sessionId: "session-1",
      messageId: "server-conflict-1",
      kind: "response",
      replyTo: staleRequest.messageId,
      ok: false,
      error: {
        category: "workspace.revisionConflict",
        message: "workspace revision is 6; edit expected 5",
      },
    });
    await expect(stale).resolves.toMatchObject({
      ok: false,
      error: { category: "workspace.revisionConflict" },
    });

    await expect(
      transport.request(
        createV2KernelRequest(
          KERNEL_PROTOCOL_V3,
          "session-1",
          "invalid-v3",
          "setVariableElement",
          {
            name: "A",
            indices: [0, 1],
            value: { real: "01", imaginary: "0" },
            expectedRevision: 6,
          },
        ),
      ),
    ).rejects.toMatchObject({ code: "request_mismatch" });
  });

  it("offers v2 first, locks it for the session, and decodes inspect replies against their request", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const receivedEvents: string[] = [];
    transport.subscribe((event) => receivedEvents.push(event.event.type));

    await expect(negotiateV2(transport, server)).resolves.toMatchObject({
      protocol: KERNEL_PROTOCOL,
      result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V2 } },
    });
    expect(server.requests()[0]).toMatchObject({
      protocol: KERNEL_PROTOCOL,
      request: {
        type: "initialize",
        params: {
          supportedProtocols: [
            KERNEL_PROTOCOL_V2,
            KERNEL_PROTOCOL_V1,
            KERNEL_PROTOCOL,
          ],
        },
      },
    });
    await expect(
      transport.request(
        createV2KernelRequest(
          KERNEL_PROTOCOL_V1,
          "session-1",
          "illegal-v1-switch",
          "listWorkspace",
          {},
        ),
      ),
    ).rejects.toMatchObject({ code: "request_mismatch" });

    const inspection = transport.request(
      createV2KernelRequest(
        KERNEL_PROTOCOL_V2,
        "session-1",
        "inspect-v2",
        "inspect",
        {
          name: "items",
          range: { start: [1, 1], size: [1, 2] },
          maxElements: 1,
        },
      ),
    );
    server.send({
      protocol: KERNEL_PROTOCOL_V2,
      sessionId: "session-1",
      messageId: "future-event-v2",
      kind: "event",
      event: { type: "futureEvent", data: { retained: true } },
    });
    server.respond(server.requests().at(-1)!, {
      type: "inspect",
      data: {
        class: "cell",
        dimensions: [1, 2],
        complex: false,
        selectedRange: { start: [1, 1], size: [1, 2] },
        kind: "cell",
        items: [
          {
            class: "logical",
            size: [1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            kind: "logical",
            logical: [true],
          },
        ],
        truncation: { truncated: true, omittedElements: 1 },
        usage: { nodes: 2, elements: 2, codeUnits: 0, depth: 1 },
      },
    });
    await expect(inspection).resolves.toMatchObject({
      result: {
        data: {
          kind: "cell",
          items: [{ kind: "logical", logical: [true] }],
        },
      },
    });
    expect(receivedEvents).toEqual([]);

    const malicious = transport.request(
      createV2KernelRequest(
        KERNEL_PROTOCOL_V2,
        "session-1",
        "inspect-v2-over-request",
        "inspect",
        {
          name: "items",
          range: { start: [1, 1], size: [1, 2] },
          maxElements: 1,
        },
      ),
    );
    const logical = {
      class: "logical",
      size: [1, 1],
      ndims: 2,
      numel: 1,
      complex: false,
      kind: "logical",
      logical: [true],
    };
    server.respond(server.requests().at(-1)!, {
      type: "inspect",
      data: {
        class: "cell",
        dimensions: [1, 2],
        complex: false,
        selectedRange: { start: [1, 1], size: [1, 2] },
        kind: "cell",
        items: [logical, logical],
        truncation: { truncated: false, omittedElements: 0 },
        usage: { nodes: 3, elements: 4, codeUnits: 0, depth: 1 },
      },
    });
    await expect(malicious).rejects.toMatchObject({ code: "invalid_message" });
    expect(server.socket.closeCode).toBe(1002);
  });

  it("bootstraps in v0, locks v1, and preserves concurrent reply correlation", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const initialized = negotiate(
      transport,
      server,
      KERNEL_PROTOCOL_V1,
      v1Capabilities,
    );

    expect(server.requests()[0]).toMatchObject({
      protocol: KERNEL_PROTOCOL,
      request: {
        type: "initialize",
        params: {
          supportedProtocols: [KERNEL_PROTOCOL_V1, KERNEL_PROTOCOL],
        },
      },
    });
    await expect(initialized).resolves.toMatchObject({
      protocol: KERNEL_PROTOCOL,
      result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V1 } },
    });
    await expect(
      transport.request(
        createVersionedKernelRequest(
          KERNEL_PROTOCOL,
          "session-1",
          "illegal-v0-switch",
          "listWorkspace",
          {},
        ),
      ),
    ).rejects.toMatchObject({ code: "request_mismatch" });

    const execute = transport.request(
      createVersionedKernelRequest(
        KERNEL_PROTOCOL_V1,
        "session-1",
        "v1-execute",
        "execute",
        { code: "answer = 42;", sourceName: "test.m", mode: "cell" },
      ),
    );
    const workspace = transport.request(
      createVersionedKernelRequest(
        KERNEL_PROTOCOL_V1,
        "session-1",
        "v1-workspace",
        "listWorkspace",
        {},
      ),
    );
    const requests = server.requests();
    const executeRequest = requests.at(-2)!;
    const workspaceRequest = requests.at(-1)!;
    expect(executeRequest.protocol).toBe(KERNEL_PROTOCOL_V1);
    expect(workspaceRequest.protocol).toBe(KERNEL_PROTOCOL_V1);

    server.respond(workspaceRequest, {
      type: "listWorkspace",
      data: { variables: [] },
    });
    server.respond(executeRequest, {
      type: "execute",
      data: { interrupted: false },
    });

    await expect(workspace).resolves.toMatchObject({ replyTo: "v1-workspace" });
    await expect(execute).resolves.toMatchObject({ replyTo: "v1-execute" });
  });

  it("downgrades to v0 without adding v1 capability fields", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    await expect(
      negotiate(transport, server, KERNEL_PROTOCOL, v0Capabilities),
    ).resolves.toMatchObject({
      protocol: KERNEL_PROTOCOL,
      result: { data: { negotiatedProtocol: KERNEL_PROTOCOL } },
    });

    const workspace = transport.request(
      createVersionedKernelRequest(
        KERNEL_PROTOCOL,
        "session-1",
        "v0-workspace",
        "listWorkspace",
        {},
      ),
    );
    const request = server.requests().at(-1)!;
    expect(request.protocol).toBe(KERNEL_PROTOCOL);
    server.respond(request, {
      type: "listWorkspace",
      data: { variables: [] },
    });
    await expect(workspace).resolves.toMatchObject({
      protocol: KERNEL_PROTOCOL,
      replyTo: "v0-workspace",
    });
  });

  it("decodes exact v1 strings and enforces request-scoped preview limits", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const negotiatedCapabilities = {
      ...v1Capabilities,
      maxPreviewElements: 2,
      maxStringElementCodeUnits: 4,
      maxPreviewCodeUnits: 4,
    };
    await negotiate(
      transport,
      server,
      KERNEL_PROTOCOL_V1,
      negotiatedCapabilities,
    );

    const exact = transport.request(
      createVersionedKernelRequest(
        KERNEL_PROTOCOL_V1,
        "session-1",
        "inspect-exact",
        "inspect",
        {
          name: "labels",
          range: { start: [1, 1], size: [1, 2] },
          maxElements: 2,
        },
      ),
    );
    server.respond(server.requests().at(-1)!, {
      type: "inspect",
      data: {
        class: "string",
        dimensions: [1, 2],
        complex: false,
        selectedRange: { start: [1, 1], size: [1, 2] },
        values: [
          { kind: "string", codeUnits: [55_357], missing: false },
          { kind: "string", codeUnits: [], missing: true },
        ],
        truncation: { truncated: false, omittedElements: 0 },
      },
    });
    await expect(exact).resolves.toMatchObject({
      result: {
        data: {
          values: [
            { kind: "string", codeUnits: [55_357], missing: false },
            { kind: "string", codeUnits: [], missing: true },
          ],
        },
      },
    });

    const overRequestLimit = transport.request(
      createVersionedKernelRequest(
        KERNEL_PROTOCOL_V1,
        "session-1",
        "inspect-over-limit",
        "inspect",
        {
          name: "labels",
          range: { start: [1, 1], size: [1, 2] },
          maxElements: 1,
        },
      ),
    );
    server.respond(server.requests().at(-1)!, {
      type: "inspect",
      data: {
        class: "string",
        dimensions: [1, 2],
        complex: false,
        selectedRange: { start: [1, 1], size: [1, 2] },
        values: [
          { kind: "string", codeUnits: [65], missing: false },
          { kind: "string", codeUnits: [66], missing: false },
        ],
        truncation: { truncated: false, omittedElements: 0 },
      },
    });
    await expect(overRequestLimit).rejects.toMatchObject({
      code: "invalid_message",
    });
    expect(server.socket.closeCode).toBe(1002);
  });

  it("closes on malformed negotiated capabilities", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const initialization = negotiate(
      transport,
      server,
      KERNEL_PROTOCOL_V1,
      { ...v1Capabilities, maxStringElementCodeUnits: 0 },
    );

    await expect(initialization).rejects.toMatchObject({
      code: "invalid_message",
    });
    expect(server.socket.closeCode).toBe(1002);
  });

  it("closes on a mid-session protocol switch or unknown protocol", async () => {
    for (const switchedProtocol of [KERNEL_PROTOCOL, "openmat-kernel-future"]) {
      const server = new FakeKernelServer();
      const transport = await connect(server);
      await negotiate(
        transport,
        server,
        KERNEL_PROTOCOL_V1,
        v1Capabilities,
      );
      const pending = transport.request(
        createVersionedKernelRequest(
          KERNEL_PROTOCOL_V1,
          "session-1",
          `pending-${switchedProtocol}`,
          "listWorkspace",
          {},
        ),
      );

      server.send({
        protocol: switchedProtocol,
        sessionId: "session-1",
        messageId: `switched-${switchedProtocol}`,
        kind: "event",
        event: { type: "status", data: { status: "idle" } },
      });

      await expect(pending).rejects.toMatchObject({ code: "invalid_message" });
      expect(server.socket.closeCode).toBe(1002);
    }
  });

  it("matches the native server's 1 MiB default message limit", async () => {
    expect(DEFAULT_MAX_MESSAGE_BYTES).toBe(1024 * 1024);
    expect(
      () =>
        new WebSocketKernelTransport("ws://127.0.0.1:8765/kernel", {
          maxMessageBytes: DEFAULT_MAX_MESSAGE_BYTES + 1,
        }),
    ).toThrow(/no greater than 1048576/);
    const server = new FakeKernelServer();
    const transport = await connect(server);

    await expect(
      transport.request(
        createKernelRequest("session-1", "default-limit", "execute", {
          code: "x".repeat(DEFAULT_MAX_MESSAGE_BYTES),
          sourceName: "test.m",
          mode: "cell",
        }),
      ),
    ).rejects.toMatchObject({ code: "message_too_large" });
    expect(server.socket.sent).toHaveLength(0);
  });

  it("uses text JSON frames and correlates concurrent responses by replyTo", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const receivedEvents: string[] = [];
    transport.subscribe((message) => receivedEvents.push(message.event.type));

    const execute = transport.request(
      createKernelRequest("session-1", "execute-1", "execute", {
        code: "answer = 6 * 7;",
        sourceName: "test.m",
        mode: "cell",
      }),
    );
    const list = transport.request(
      createKernelRequest("session-1", "list-1", "listWorkspace", {}),
    );

    const [executeRequest, listRequest] = server.requests();
    expect(executeRequest).toMatchObject({
      kind: "request",
      request: {
        type: "execute",
        params: { sourceName: "test.m", mode: "cell" },
      },
    });
    expect(listRequest).toMatchObject({
      request: { type: "listWorkspace", params: {} },
    });

    server.send({
      protocol: KERNEL_PROTOCOL,
      sessionId: "session-1",
      messageId: "unknown-event-1",
      kind: "event",
      event: { type: "futureEvent", data: { answer: 42 } },
    });
    server.send({
      protocol: KERNEL_PROTOCOL,
      sessionId: "session-1",
      messageId: "status-event-1",
      kind: "event",
      event: { type: "status", data: { status: "busy" } },
    });
    server.respond(listRequest!, {
      type: "listWorkspace",
      data: {
        variables: [
          {
            name: "answer",
            class: "double",
            dimensions: [1, 1],
            complex: false,
            bytes: 8,
          },
        ],
      },
    });
    server.respond(executeRequest!, {
      type: "execute",
      data: { interrupted: false },
    });

    await expect(list).resolves.toMatchObject({
      replyTo: "list-1",
      result: { type: "listWorkspace" },
    });
    await expect(execute).resolves.toMatchObject({
      replyTo: "execute-1",
      result: { type: "execute", data: { interrupted: false } },
    });
    expect(receivedEvents).toEqual(["status"]);
    await expect(
      transport.request(
        createKernelRequest("session-1", "execute-1", "interrupt", {}),
      ),
    ).rejects.toMatchObject({ code: "duplicate_message_id" });
  });

  it("rejects every pending request when the connection closes", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const connectionLosses: Error[] = [];
    transport.subscribeConnectionLoss((error) => connectionLosses.push(error));
    const pending = transport.request(
      createKernelRequest("session-1", "pending-1", "listWorkspace", {}),
    );

    server.socket.close(1011, "kernel exited");

    await expect(pending).rejects.toMatchObject({
      code: "connection_closed",
    });
    await expect(
      transport.request(
        createKernelRequest("session-1", "after-close", "listWorkspace", {}),
      ),
    ).rejects.toBeInstanceOf(KernelTransportError);
    expect(connectionLosses).toHaveLength(1);
    expect(connectionLosses[0]).toMatchObject({ code: "connection_closed" });
  });

  it("does not report an intentional client disconnect as connection loss", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const connectionLosses: Error[] = [];
    transport.subscribeConnectionLoss((error) => connectionLosses.push(error));

    await transport.disconnect();

    expect(connectionLosses).toEqual([]);
    expect(server.socket.closeCode).toBe(1000);
  });

  it("rejects non-text frames and enforces inbound/outbound size limits", async () => {
    const binaryServer = new FakeKernelServer();
    const binaryTransport = await connect(binaryServer);
    const binaryPending = binaryTransport.request(
      createKernelRequest("session-1", "binary-1", "listWorkspace", {}),
    );
    binaryServer.socket.receive(new ArrayBuffer(4));
    await expect(binaryPending).rejects.toMatchObject({
      code: "unexpected_frame_type",
    });
    expect(binaryServer.socket.closeCode).toBe(1003);

    const largeServer = new FakeKernelServer();
    const largeTransport = await connect(largeServer, 256);
    const largePending = largeTransport.request(
      createKernelRequest("session-1", "large-1", "listWorkspace", {}),
    );
    largeServer.socket.receive("x".repeat(257));
    await expect(largePending).rejects.toMatchObject({
      code: "message_too_large",
    });
    expect(largeServer.socket.closeCode).toBe(1009);

    const outboundServer = new FakeKernelServer();
    const outboundTransport = await connect(outboundServer, 64);
    await expect(
      outboundTransport.request(
        createKernelRequest("session-1", "oversized-request", "execute", {
          code: "answer = 6 * 7;",
          sourceName: "test.m",
          mode: "cell",
        }),
      ),
    ).rejects.toMatchObject({ code: "message_too_large" });
    expect(outboundServer.socket.sent).toHaveLength(0);

    const utf8Server = new FakeKernelServer();
    const utf8Transport = await connect(utf8Server, 256);
    const utf8Pending = utf8Transport.request(
      createKernelRequest("session-1", "utf8-1", "listWorkspace", {}),
    );
    const utf8Frame = JSON.stringify({
      protocol: KERNEL_PROTOCOL,
      sessionId: "session-1",
      messageId: "future-utf8",
      kind: "event",
      event: { type: "futureEvent", data: "界".repeat(60) },
    });
    expect(utf8Frame.length).toBeLessThanOrEqual(256);
    expect(new TextEncoder().encode(utf8Frame).byteLength).toBeGreaterThan(256);
    utf8Server.socket.receive(utf8Frame);
    await expect(utf8Pending).rejects.toMatchObject({
      code: "message_too_large",
    });
    expect(utf8Server.socket.closeCode).toBe(1009);
  });

  it("rejects response types that do not match the pending request", async () => {
    const server = new FakeKernelServer();
    const transport = await connect(server);
    const pending = transport.request(
      createKernelRequest("session-1", "execute-1", "execute", {
        code: "answer = 42;",
        sourceName: "test.m",
        mode: "cell",
      }),
    );
    const [request] = server.requests();
    server.respond(request!, {
      type: "interrupt",
      data: { accepted: true },
    });

    await expect(pending).rejects.toMatchObject({ code: "request_mismatch" });
    expect(server.socket.closeCode).toBe(1002);
  });
});
