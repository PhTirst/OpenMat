import { describe, expect, it } from "vitest";
import {
  KERNEL_PROTOCOL,
  createKernelRequest,
  parseKernelEvent,
} from "../protocol/kernel-v0";
import {
  KERNEL_PROTOCOL_V1,
  MAX_PREVIEW_CODE_UNITS,
  MAX_STRING_ELEMENT_CODE_UNITS,
  SUPPORTED_KERNEL_PROTOCOLS,
  createBootstrapInitializeRequest,
  createKernelRequest as createVersionedKernelRequest,
} from "../protocol/kernel-v1";
import {
  KERNEL_PROTOCOL_V2,
  KERNEL_PROTOCOL_V3,
  MAX_AGGREGATE_DEPTH,
  MAX_AGGREGATE_ELEMENTS,
  MAX_AGGREGATE_NODES,
  SUPPORTED_KERNEL_PROTOCOLS as V2_SUPPORTED_KERNEL_PROTOCOLS,
  createBootstrapInitializeRequest as createV2BootstrapInitializeRequest,
  createKernelRequest as createV2KernelRequest,
} from "../protocol/kernel-v2";
import { MockKernelTransport } from "./mock-kernel-transport";

const clientCapabilities = {
  executionModes: ["cell" as const],
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 128,
  interrupt: true,
  workspaceDelta: true,
};

describe("MockKernelTransport", () => {
  it("negotiates v3 as the product protocol and emits versioned workspace data", async () => {
    const transport = new MockKernelTransport({ eventDelayMs: 0 });
    const eventProtocols: string[] = [];
    transport.subscribe((event) => eventProtocols.push(event.protocol));
    await transport.connect("test-session");

    const initialized = await transport.request(
      createV2BootstrapInitializeRequest("test-session", "initialize-v2", {
        client: { name: "test", version: "2" },
        supportedProtocols: V2_SUPPORTED_KERNEL_PROTOCOLS,
        capabilities: {
          ...clientCapabilities,
          maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
          maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
          maxAggregateNodes: MAX_AGGREGATE_NODES,
          maxAggregateElements: MAX_AGGREGATE_ELEMENTS,
          maxAggregateDepth: MAX_AGGREGATE_DEPTH,
        },
      }),
    );

    expect(initialized).toMatchObject({
      protocol: KERNEL_PROTOCOL,
      result: {
        data: {
          negotiatedProtocol: KERNEL_PROTOCOL_V3,
          capabilities: {
            maxAggregateNodes: MAX_AGGREGATE_NODES,
            maxAggregateElements: MAX_AGGREGATE_ELEMENTS,
            maxAggregateDepth: MAX_AGGREGATE_DEPTH,
          },
        },
      },
    });
    expect(eventProtocols).toEqual([KERNEL_PROTOCOL, KERNEL_PROTOCOL_V3]);

    const workspace = await transport.request(
      createV2KernelRequest(
        KERNEL_PROTOCOL_V3,
        "test-session",
        "list-v2",
        "listWorkspace",
        {},
      ),
    );
    expect(workspace).toMatchObject({
      protocol: KERNEL_PROTOCOL_V3,
      result: { data: { revision: 0, variables: [] } },
    });
  });

  it("negotiates v1 from a v0 bootstrap and emits the next envelope as v1", async () => {
    const transport = new MockKernelTransport({
      eventDelayMs: 0,
      supportedProtocols: [KERNEL_PROTOCOL_V1, KERNEL_PROTOCOL],
    });
    const eventProtocols: string[] = [];
    transport.subscribe((event) => eventProtocols.push(event.protocol));
    await transport.connect("test-session");

    const initialized = await transport.request(
      createBootstrapInitializeRequest("test-session", "initialize-v1", {
        client: { name: "test", version: "1" },
        supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
        capabilities: {
          ...clientCapabilities,
          maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
          maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
        },
      }),
    );

    expect(initialized).toMatchObject({
      protocol: KERNEL_PROTOCOL,
      result: {
        data: {
          negotiatedProtocol: KERNEL_PROTOCOL_V1,
          capabilities: {
            maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
            maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
          },
        },
      },
    });
    expect(eventProtocols).toEqual([KERNEL_PROTOCOL, KERNEL_PROTOCOL_V1]);

    const workspace = await transport.request(
      createVersionedKernelRequest(
        KERNEL_PROTOCOL_V1,
        "test-session",
        "list-v1",
        "listWorkspace",
        {},
      ),
    );
    expect(workspace.protocol).toBe(KERNEL_PROTOCOL_V1);
  });

  it("supports v0-only downgrade and rejects malformed v1 offers", async () => {
    const v0Transport = new MockKernelTransport({
      eventDelayMs: 0,
      supportedProtocols: [KERNEL_PROTOCOL],
    });
    await v0Transport.connect("test-session");
    const downgraded = await v0Transport.request(
      createBootstrapInitializeRequest("test-session", "initialize-v0", {
        client: { name: "test", version: "1" },
        supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
        capabilities: {
          ...clientCapabilities,
          maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
          maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
        },
      }),
    );
    expect(downgraded).toMatchObject({
      protocol: KERNEL_PROTOCOL,
      result: {
        data: {
          negotiatedProtocol: KERNEL_PROTOCOL,
          capabilities: clientCapabilities,
        },
      },
    });

    const invalidTransport = new MockKernelTransport({ eventDelayMs: 0 });
    await invalidTransport.connect("invalid-session");
    const invalid = await invalidTransport.request(
      createKernelRequest("invalid-session", "invalid-v1", "initialize", {
        client: { name: "test", version: "1" },
        supportedProtocols: [KERNEL_PROTOCOL_V1, KERNEL_PROTOCOL],
        capabilities: clientCapabilities,
      }),
    );
    expect(invalid).toMatchObject({
      ok: false,
      error: { category: "protocol.invalidCapabilities" },
    });
  });

  it("emits a deterministic wire-compatible execute slice", async () => {
    const transport = new MockKernelTransport({
      eventDelayMs: 0,
      supportedProtocols: [KERNEL_PROTOCOL],
    });
    const events: string[] = [];
    const unsubscribe = transport.subscribe((message) => {
      expect(parseKernelEvent(message)).not.toBeNull();
      events.push(
        message.event.type === "status"
          ? `status:${message.event.data.status}`
          : message.event.type,
      );
    });

    await transport.connect("test-session");
    const initialized = await transport.request(
      createKernelRequest("test-session", "request-1", "initialize", {
        client: { name: "test", version: "0" },
        supportedProtocols: [KERNEL_PROTOCOL],
        capabilities: clientCapabilities,
      }),
    );
    expect(initialized).toMatchObject({
      ok: true,
      result: {
        type: "initialize",
        data: { negotiatedProtocol: KERNEL_PROTOCOL },
      },
    });
    events.length = 0;

    const response = await transport.request(
      createKernelRequest("test-session", "request-2", "execute", {
        code: "answer = 6 * 7;",
        sourceName: "test.m",
        mode: "cell",
      }),
    );

    expect(response).toMatchObject({
      ok: true,
      replyTo: "request-2",
      result: { type: "execute", data: { interrupted: false } },
    });
    expect(events).toEqual([
      "status:busy",
      "display",
      "workspaceDelta",
      "status:idle",
    ]);

    unsubscribe();
    await transport.disconnect();
  });

  it("returns real list and inspect result shapes", async () => {
    const transport = new MockKernelTransport({ eventDelayMs: 0 });
    await transport.connect("test-session");
    await transport.request(
      createKernelRequest("test-session", "initialize-1", "initialize", {
        client: { name: "test", version: "0" },
        supportedProtocols: [KERNEL_PROTOCOL],
        capabilities: clientCapabilities,
      }),
    );
    await transport.request(
      createKernelRequest("test-session", "execute-1", "execute", {
        code: "y = 30;",
        sourceName: "test.m",
        mode: "cell",
      }),
    );

    const workspace = await transport.request(
      createKernelRequest("test-session", "list-1", "listWorkspace", {}),
    );
    const inspection = await transport.request(
      createKernelRequest("test-session", "inspect-1", "inspect", {
        name: "y",
        range: { start: [1, 1], size: [1, 1] },
        maxElements: 1,
      }),
    );

    expect(workspace).toMatchObject({
      ok: true,
      result: {
        type: "listWorkspace",
        data: {
          variables: [
            { name: "x", class: "double", complex: false },
            { name: "y", class: "double", complex: false },
          ],
        },
      },
    });
    expect(inspection).toMatchObject({
      ok: true,
      result: {
        type: "inspect",
        data: {
          selectedRange: { start: [1, 1], size: [1, 1] },
          values: [{ kind: "number", value: 30 }],
          truncation: { truncated: false, omittedElements: 0 },
        },
      },
    });
  });

  it("supports cooperative interruption on the control path", async () => {
    const transport = new MockKernelTransport({ eventDelayMs: 20 });
    const statuses: string[] = [];
    transport.subscribe((message) => {
      if (message.event.type === "status") {
        statuses.push(message.event.data.status);
      }
    });
    await transport.connect("test-session");
    await transport.request(
      createKernelRequest("test-session", "initialize-1", "initialize", {
        client: { name: "test", version: "0" },
        supportedProtocols: [KERNEL_PROTOCOL],
        capabilities: clientCapabilities,
      }),
    );

    const execution = transport.request(
      createKernelRequest("test-session", "execute-1", "execute", {
        code: "while true\nend",
        sourceName: "test.m",
        mode: "cell",
      }),
    );
    const interruption = await transport.request(
      createKernelRequest("test-session", "interrupt-1", "interrupt", {}),
    );
    const result = await execution;

    expect(interruption).toMatchObject({
      ok: true,
      result: { type: "interrupt", data: { accepted: true } },
    });
    expect(result).toMatchObject({
      ok: true,
      result: { type: "execute", data: { interrupted: true } },
    });
    expect(statuses).toContain("interrupted");
    expect(statuses.at(-1)).toBe("idle");
  });
});
