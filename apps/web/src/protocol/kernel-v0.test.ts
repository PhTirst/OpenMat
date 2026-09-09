import { describe, expect, it } from "vitest";
import {
  KERNEL_PROTOCOL,
  createKernelRequest,
  parseKernelEvent,
  parseKernelResponse,
  parseKernelServerMessage,
  type Capabilities,
  type MatrixPreview,
} from "./kernel-v0";

const capabilities: Capabilities = {
  executionModes: ["file", "cell", "repl"],
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 128,
  interrupt: true,
  workspaceDelta: true,
};

const preview: MatrixPreview = {
  class: "double",
  dimensions: [2, 3],
  selectedRange: { start: [1, 1], size: [2, 3] },
  values: [
    { kind: "number", value: 1 },
    { kind: "complex", real: 2, imaginary: -3 },
    { kind: "logical", value: true },
    { kind: "text", value: "four" },
    { kind: "special", value: "nan" },
    { kind: "missing" },
  ],
  truncation: { truncated: false, omittedElements: 0 },
};

function response(result: unknown, messageId: string) {
  return {
    protocol: KERNEL_PROTOCOL,
    sessionId: "session-1",
    messageId,
    kind: "response",
    replyTo: `request-${messageId}`,
    ok: true,
    result,
    futureField: "ignored",
  };
}

function event(type: string, data: unknown, messageId: string) {
  return {
    protocol: KERNEL_PROTOCOL,
    sessionId: "session-1",
    messageId,
    kind: "event",
    event: { type, data },
    futureField: "ignored",
  };
}

describe("kernel-v0 requests", () => {
  it("serializes every request with the Rust tagged request shape", () => {
    const requests = [
      createKernelRequest("session-1", "initialize", "initialize", {
        client: { name: "web-test", version: "1" },
        supportedProtocols: [KERNEL_PROTOCOL],
        capabilities,
      }),
      createKernelRequest("session-1", "execute", "execute", {
        code: "answer = 6 * 7;",
        sourceName: "test.m",
        mode: "cell",
      }),
      createKernelRequest("session-1", "interrupt", "interrupt", {}),
      createKernelRequest("session-1", "inspect", "inspect", {
        name: "answer",
        range: { start: [1, 1], size: [1, 1] },
        maxElements: 1,
      }),
      createKernelRequest("session-1", "list", "listWorkspace", {}),
      createKernelRequest("session-1", "shutdown", "shutdown", {}),
    ];

    expect(requests.map((request) => request.request.type)).toEqual([
      "initialize",
      "execute",
      "interrupt",
      "inspect",
      "listWorkspace",
      "shutdown",
    ]);
    expect(requests[0]).toEqual(
      expect.objectContaining({
        kind: "request",
        request: {
          type: "initialize",
          params: {
            client: { name: "web-test", version: "1" },
            supportedProtocols: [KERNEL_PROTOCOL],
            capabilities,
          },
        },
      }),
    );
    for (const request of requests) {
      expect(request).not.toHaveProperty("method");
      expect(request).not.toHaveProperty("params");
    }
  });
});

describe("kernel-v0 responses", () => {
  it("parses every successful Rust result variant", () => {
    const results = [
      {
        type: "initialize",
        data: {
          negotiatedProtocol: KERNEL_PROTOCOL,
          implementation: { name: "openmat-runtime", version: "0.1.0" },
          capabilities,
        },
      },
      { type: "execute", data: { interrupted: false } },
      { type: "interrupt", data: { accepted: true } },
      { type: "inspect", data: preview },
      {
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
      },
      { type: "shutdown", data: {} },
    ];

    for (const [index, result] of results.entries()) {
      expect(parseKernelResponse(response(result, index.toString()))).toMatchObject({
        ok: true,
        result,
      });
    }
  });

  it("parses failures with Rust diagnostics and byte ranges", () => {
    const failure = {
      protocol: KERNEL_PROTOCOL,
      sessionId: "session-1",
      messageId: "failure-1",
      kind: "response",
      replyTo: "request-1",
      ok: false,
      error: {
        category: "compile.parse",
        message: "parse failed",
        diagnostics: [
          {
            code: "OM1001",
            severity: "hint",
            message: "insert an expression",
            range: { sourceName: "变量.m", start: 9, end: 10 },
            related: [
              {
                message: "assignment starts here",
                range: { sourceName: "变量.m", start: 0, end: 6 },
              },
            ],
          },
        ],
      },
    };

    expect(parseKernelResponse(failure)).toEqual(failure);
  });

  it("rejects invented and inconsistent response fields", () => {
    expect(
      parseKernelResponse(
        response({ type: "execute", data: { accepted: true } }, "bad-execute"),
      ),
    ).toBeNull();
    expect(
      parseKernelResponse(
        response(
          {
            type: "inspect",
            data: { ...preview, truncation: { truncated: true, omittedElements: 1 } },
          },
          "bad-preview",
        ),
      ),
    ).toBeNull();
  });
});

describe("kernel-v0 events", () => {
  it("parses every known Rust event variant", () => {
    const events = [
      event("status", { status: "busy" }, "status"),
      event("stream", { stream: "stdout", text: "hello\n" }, "stream"),
      event(
        "display",
        { representations: { "text/plain": "42" } },
        "display",
      ),
      event(
        "diagnostic",
        {
          severity: "warning",
          message: "warning text",
          range: { sourceName: "test.m", start: 0, end: 1 },
        },
        "diagnostic",
      ),
      event(
        "workspaceDelta",
        {
          added: [
            {
              name: "answer",
              class: "double",
              dimensions: [1, 1],
              complex: false,
            },
          ],
          changed: [],
          removed: [],
        },
        "workspace",
      ),
    ];

    expect(events.map((value) => parseKernelEvent(value)?.event.type)).toEqual([
      "status",
      "stream",
      "display",
      "diagnostic",
      "workspaceDelta",
    ]);
  });

  it("preserves unknown events for transport-level ignoring", () => {
    const futureEvent = event("futureEvent", { answer: 42 }, "future");
    expect(parseKernelEvent(futureEvent)).toBeNull();
    expect(parseKernelServerMessage(futureEvent)).toMatchObject({
      messageType: "unknownEvent",
      event: { event: { type: "futureEvent", data: { answer: 42 } } },
    });
  });

  it("rejects malformed known events instead of treating them as unknown", () => {
    expect(
      parseKernelServerMessage(
        event("stream", { channel: "stdout", text: "wrong field" }, "bad-stream"),
      ),
    ).toBeNull();
    expect(
      parseKernelServerMessage(
        event(
          "display",
          { representations: { "application/json": { invalid: "object" } } },
          "bad-display",
        ),
      ),
    ).toBeNull();
  });
});
