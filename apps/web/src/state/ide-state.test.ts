import { describe, expect, it } from "vitest";
import { KERNEL_PROTOCOL, type KernelEvent } from "../protocol/kernel-v0";
import {
  KERNEL_PROTOCOL_V1,
  type V1MatrixPreview,
} from "../protocol/kernel-v1";
import { ideReducer, initialIdeState } from "./ide-state";

function event<TEvent extends KernelEvent>(value: TEvent): TEvent {
  return value;
}

describe("ideReducer", () => {
  it("clears session-derived views before connecting a new kernel session", () => {
    const next = ideReducer(
      {
        ...initialIdeState,
        activeRequestId: "request-1",
        commandWindowEntries: [
          { id: "history-1", channel: "stdout", text: "previous output\n" },
        ],
        figures: [
          {
            representations: {
              "application/vnd.openmat.plot+json": "{}",
            },
          },
        ],
        workspace: [
          {
            name: "answer",
            class: "double",
            dimensions: [1, 1],
            complex: false,
            bytes: 8,
          },
        ],
        inspection: {
          name: "answer",
          preview: {
            class: "double",
            dimensions: [1, 1],
            selectedRange: { start: [1, 1], size: [1, 1] },
            values: [{ kind: "number", value: 42 }],
            truncation: { truncated: false, omittedElements: 0 },
          },
        },
        inspectingName: "answer",
      },
      { type: "connectionStarted" },
    );

    expect(next).toMatchObject({
      connection: "connecting",
      kernelStatus: "starting",
      activeRequestId: null,
      figures: [],
      workspace: [],
      inspection: null,
      inspectingName: null,
    });
    expect(next.commandWindowEntries).toEqual([
      { id: "history-1", channel: "stdout", text: "previous output\n" },
    ]);
  });

  it("applies wire workspace summaries by name without inventing values", () => {
    const next = ideReducer(initialIdeState, {
      type: "eventReceived",
      event: event({
        protocol: KERNEL_PROTOCOL,
        sessionId: "session",
        messageId: "workspace-1",
        kind: "event",
        event: {
          type: "workspaceDelta",
          data: {
            added: [
              {
                name: "largeMatrix",
                class: "double",
                dimensions: [10_000, 10_000],
                complex: false,
                bytes: 800_000_000,
              },
            ],
            changed: [],
            removed: [],
          },
        },
      }),
    });

    expect(next.workspace).toEqual([
      {
        name: "largeMatrix",
        class: "double",
        dimensions: [10_000, 10_000],
        complex: false,
        bytes: 800_000_000,
      },
    ]);
    expect(next.workspace[0]).not.toHaveProperty("preview");
    expect(next.workspace[0]).not.toHaveProperty("summary");
    expect(next.workspace[0]).not.toHaveProperty("value");
  });

  it("routes text displays to the command window and plots to figures", () => {
    const withStream = ideReducer(initialIdeState, {
      type: "eventReceived",
      event: event({
        protocol: KERNEL_PROTOCOL,
        sessionId: "session",
        messageId: "stream-1",
        kind: "event",
        event: {
          type: "stream",
          data: { stream: "stdout", text: "hello\n" },
        },
      }),
    });
    const withDisplay = ideReducer(withStream, {
      type: "eventReceived",
      event: event({
        protocol: KERNEL_PROTOCOL,
        sessionId: "session",
        messageId: "display-1",
        kind: "event",
        event: {
          type: "display",
          data: { representations: { "text/plain": "42" } },
        },
      }),
    });

    expect(withDisplay.commandWindowEntries).toEqual([
      expect.objectContaining({ channel: "stdout", text: "hello\n" }),
      expect.objectContaining({ channel: "stdout", text: "42\n" }),
    ]);
    expect(withDisplay.figures).toEqual([]);

    const withFigure = ideReducer(withDisplay, {
      type: "eventReceived",
      event: event({
        protocol: KERNEL_PROTOCOL,
        sessionId: "session",
        messageId: "figure-1",
        kind: "event",
        event: {
          type: "display",
          data: {
            representations: {
              "text/plain": "line plot",
              "application/vnd.openmat.plot+json":
                '{"kind":"line","title":"Figure","x":[1],"y":[2]}',
            },
          },
        },
      }),
    });
    expect(withFigure.figures).toHaveLength(1);
    expect(withFigure.commandWindowEntries).toEqual(
      withDisplay.commandWindowEntries,
    );

    const afterClc = ideReducer(withFigure, {
      type: "eventReceived",
      event: event({
        protocol: KERNEL_PROTOCOL,
        sessionId: "session",
        messageId: "clear-1",
        kind: "event",
        event: {
          type: "display",
          data: {
            representations: {
              "application/vnd.openmat.command-window-clear+json": "{}",
            },
          },
        },
      }),
    });
    expect(afterClc.commandWindowEntries).toEqual([]);
    expect(afterClc.figures).toEqual(withFigure.figures);
  });

  it("echoes REPL commands but does not echo an editor buffer", () => {
    const editorRun = ideReducer(initialIdeState, {
      type: "executionStarted",
      requestId: "editor-run",
    });
    expect(editorRun.commandWindowEntries).toEqual([]);

    const commandRun = ideReducer(initialIdeState, {
      type: "executionStarted",
      requestId: "command-run",
      command: "answer = 42",
    });
    expect(commandRun.commandWindowEntries).toEqual([
      {
        id: "command-run-input",
        channel: "input",
        text: ">> answer = 42\n",
      },
    ]);
  });

  it("stores typed inspection results separately from summaries", () => {
    const next = ideReducer(initialIdeState, {
      type: "inspectionLoaded",
      name: "answer",
      preview: {
        class: "double",
        dimensions: [1, 1],
        selectedRange: { start: [1, 1], size: [1, 1] },
        values: [{ kind: "number", value: 42 }],
        truncation: { truncated: false, omittedElements: 0 },
      },
    });

    expect(next.inspection).toMatchObject({
      name: "answer",
      preview: { values: [{ kind: "number", value: 42 }] },
    });
  });

  it("keeps UTF-16 truth lossless in state", () => {
    const preview: V1MatrixPreview = {
      class: "string",
      dimensions: [1, 4],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 4] },
      values: [
        { kind: "string", codeUnits: [55_357], missing: false },
        { kind: "string", codeUnits: [], missing: false },
        { kind: "string", codeUnits: [], missing: true },
      ],
      truncation: { truncated: true, omittedElements: 1 },
    };
    const next = ideReducer(
      {
        ...initialIdeState,
        connection: "connected",
        negotiatedProtocol: KERNEL_PROTOCOL_V1,
      },
      { type: "inspectionLoaded", name: "labels", preview },
    );

    expect(next.inspection?.preview).toBe(preview);
    expect(next.inspection?.preview.values[0]).toEqual({
      kind: "string",
      codeUnits: [55_357],
      missing: false,
    });
    expect(JSON.stringify(next.inspection)).not.toContain("�");
  });
});
