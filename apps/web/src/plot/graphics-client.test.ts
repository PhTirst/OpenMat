import { describe, expect, it, vi } from "vitest";
import {
  GraphicsV1Client,
  type GraphicsClientEvent,
  type GraphicsSocket,
} from "./graphics-client";
import type { FigureDiscovery } from "./graphics-v1";

const discovery: FigureDiscovery = {
  schemaVersion: 1,
  graphicsProtocol: "openmat-graphics-v1",
  endpoint: "/graphics/v1",
  attachToken: "top-secret-token",
  figureId: "figure-1",
  revision: 1,
};

const limits = {
  renderBackend: "webgpu",
  maxTextFrameBytes: 1_048_576,
  maxBinaryFrameBytes: 8_388_608,
  maxBufferBytes: 268_435_456,
  maxResidentBytes: 536_870_912,
  maxObjects: 100_000,
};

class FakeSocket implements GraphicsSocket {
  binaryType: BinaryType = "blob";
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  readonly sent: string[] = [];
  readonly closes: Array<{ readonly code?: number; readonly reason?: string }> = [];

  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void {
    if (typeof data !== "string") {
      throw new Error("The graphics client must never send binary frames");
    }
    this.sent.push(data);
  }

  close(code?: number, reason?: string): void {
    this.closes.push({ ...(code === undefined ? {} : { code }), ...(reason === undefined ? {} : { reason }) });
    this.readyState = 3;
    this.onclose?.(
      new CloseEvent("close", {
        code: code ?? 1000,
        ...(reason === undefined ? {} : { reason }),
      }),
    );
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }

  receive(value: unknown): void {
    const data = typeof value === "string" ? value : JSON.stringify(value);
    this.onmessage?.(new MessageEvent("message", { data }));
  }

  receiveBinary(data: ArrayBuffer): void {
    this.onmessage?.(new MessageEvent("message", { data }));
  }

  serverClose(): void {
    this.readyState = 3;
    this.onclose?.(new CloseEvent("close", { code: 1006 }));
  }
}

function requests(socket: FakeSocket): Array<Record<string, unknown>> {
  return socket.sent.map((encoded) => JSON.parse(encoded) as Record<string, unknown>);
}

function requestBody(message: Record<string, unknown>): Record<string, unknown> {
  return message.request as Record<string, unknown>;
}

function response(
  request: Record<string, unknown>,
  result: Readonly<Record<string, unknown>>,
): Record<string, unknown> {
  return {
    protocol: request.protocol,
    sessionId: "session-1",
    messageId: `response-${String(request.messageId)}`,
    kind: "response",
    replyTo: request.messageId,
    ok: true,
    result,
  };
}

function failure(
  request: Record<string, unknown>,
  category: string,
  message: string,
): Record<string, unknown> {
  return {
    protocol: request.protocol,
    sessionId: "session-1",
    messageId: `response-${String(request.messageId)}`,
    kind: "response",
    replyTo: request.messageId,
    ok: false,
    error: { category, message },
  };
}

function initialize(
  socket: FakeSocket,
  figures: readonly { readonly figureId: string; readonly revision: number }[] = [
    { figureId: "figure-1", revision: 1 },
  ],
): void {
  socket.open();
  const initializeRequest = requests(socket)[0];
  if (initializeRequest === undefined) {
    throw new Error("initialize request missing");
  }
  socket.receive(
    response(initializeRequest, {
      type: "initialize",
      implementation: { name: "openmat-server", version: "0.1.0" },
      capabilities: limits,
      figures,
    }),
  );
}

function rootObject(id = "root"): Record<string, unknown> {
  return {
    id,
    generation: 1,
    objectRevision: 1,
    kind: "figure",
    parentId: null,
    children: [],
    properties: {
      number: 1,
      nameCodeUnits: [],
      numberTitle: true,
      visible: true,
      backgroundRgba: [1, 1, 1, 1],
      initialLogicalSizeCssPixels: [560, 420],
      positionCssPixels: [100, 100, 560, 420],
      nextPlot: "add",
    },
  };
}

function emptySnapshot(figureId: string, revision: number): Record<string, unknown> {
  return {
    type: "figureSnapshot",
    figureId,
    revision,
    rootId: `${figureId}-root`,
    objects: [rootObject(`${figureId}-root`)],
    referencedBuffers: [],
  };
}

function bufferedSnapshot(): Record<string, unknown> {
  const descriptor = {
    bufferId: "buffer-y",
    dtype: "f64",
    shape: [1, 1],
    order: "columnMajor",
    endianness: "little",
    byteOffset: 0,
    byteLength: 8,
  };
  return {
    type: "figureSnapshot",
    figureId: "figure-1",
    revision: 1,
    rootId: "root",
    objects: [
      { ...rootObject(), children: ["axes"] },
      {
        id: "axes",
        generation: 1,
        objectRevision: 1,
        kind: "axes2d",
        parentId: "root",
        children: ["line"],
        properties: {
          positionNormalized: [0.1, 0.1, 0.8, 0.8],
          xScale: "linear",
          yScale: "linear",
          xLimits: [0, 1],
          yLimits: [0, 1],
          xLimitsMode: "auto",
          yLimitsMode: "auto",
          nextPlot: "replace",
          gridX: false,
          gridY: false,
          colorOrderIndex: 1,
          titleId: null,
          xLabelId: null,
          yLabelId: null,
        },
      },
      {
        id: "line",
        generation: 1,
        objectRevision: 1,
        kind: "lineSeries",
        parentId: "axes",
        children: [],
        properties: {
          xData: descriptor,
          yData: descriptor,
          colorRgba: [0, 0.45, 0.74, 1],
          lineWidthCssPx: 1,
          lineStyle: "solid",
          marker: "none",
          markerSizeCssPx: 6,
          displayNameCodeUnits: [],
          visible: true,
          clipping: true,
        },
      },
    ],
    referencedBuffers: [descriptor],
  };
}

function omgp(
  offset: number,
  payload: readonly number[],
  final: boolean,
): ArrayBuffer {
  const headerBytes = new TextEncoder().encode(
    JSON.stringify({
      type: "bufferChunk",
      transferId: "transfer-y",
      bufferId: "buffer-y",
      offset,
      totalBytes: 8,
      final,
    }),
  );
  const bytes = new Uint8Array(16 + headerBytes.length + payload.length);
  bytes.set([0x4f, 0x4d, 0x47, 0x50]);
  const view = new DataView(bytes.buffer);
  view.setUint16(4, 1, true);
  view.setUint16(6, 0, true);
  view.setUint32(8, headerBytes.length, true);
  view.setUint32(12, payload.length, true);
  bytes.set(headerBytes, 16);
  bytes.set(payload, 16 + headerBytes.length);
  return bytes.buffer;
}

function clientHarness(
  extraDiscoveries: readonly FigureDiscovery[] = [],
  primaryDiscovery: FigureDiscovery = discovery,
) {
  const sockets: FakeSocket[] = [];
  const urls: string[] = [];
  const scheduled: Array<() => void> = [];
  const events: GraphicsClientEvent[] = [];
  const client = new GraphicsV1Client("session-1", primaryDiscovery, {
    url: "ws://127.0.0.1:51589/graphics/v1",
    socketFactory: (url) => {
      urls.push(url);
      const socket = new FakeSocket();
      sockets.push(socket);
      return socket;
    },
    reconnectDelayMs: () => 0,
    setTimer: (callback) => {
      scheduled.push(callback);
      return scheduled.length;
    },
    clearTimer: vi.fn(),
  });
  client.setDiscoveries(extraDiscoveries);
  client.subscribe((event) => events.push(event));
  client.connect();
  const socket = sockets[0];
  if (socket === undefined) {
    throw new Error("socket missing");
  }
  return { client, socket, sockets, scheduled, events, urls };
}

describe("GraphicsV1Client", () => {
  it("replays cached figures only to the new subscriber and broadcasts real updates", () => {
    const { client, socket, events } = clientHarness();
    initialize(socket);
    const snapshotRequest = requests(socket)[1];
    socket.receive(
      response(snapshotRequest ?? {}, {
        type: "getSnapshot",
        snapshot: emptySnapshot("figure-1", 1),
      }),
    );
    const existingEvents = [...events];
    const lateEvents: GraphicsClientEvent[] = [];
    const unsubscribe = client.subscribe((event) => lateEvents.push(event));

    expect(events).toEqual(existingEvents);
    expect(lateEvents).toMatchObject([
      { type: "connection", status: { phase: "attached" } },
      { type: "figureReady", figure: { figureId: "figure-1", revision: 1 } },
    ]);

    const update = (revision: number) => socket.receive({
      protocol: "openmat-graphics-v1",
      sessionId: "session-1",
      messageId: `delta-${revision}`,
      kind: "event",
      event: {
        type: "figureDelta",
        figureId: "figure-1",
        baseRevision: revision - 1,
        revision,
        operations: [],
        addedBuffers: [],
        releasedBufferIds: [],
      },
    });
    update(2);
    expect(events.at(-1)).toMatchObject({
      type: "figureReady", figure: { revision: 2 },
    });
    expect(lateEvents.at(-1)).toEqual(events.at(-1));
    expect(lateEvents).toHaveLength(3);

    unsubscribe();
    update(3);
    expect(events.at(-1)).toMatchObject({
      type: "figureReady", figure: { revision: 3 },
    });
    expect(lateEvents).toHaveLength(3);
    client.dispose();
  });

  it.each([false, true])(
    "waits for complete buffers before replaying a pending snapshot (previous figure: %s)",
    (hasPreviousFigure) => {
      const { client, socket, events } = clientHarness();
      initialize(socket);
      if (hasPreviousFigure) {
        socket.receive(
          response(requests(socket)[1] ?? {}, {
            type: "getSnapshot",
            snapshot: emptySnapshot("figure-1", 1),
          }),
        );
        client.setDiscoveries([{ ...discovery, revision: 2 }]);
      }
      const revision = hasPreviousFigure ? 2 : 1;
      socket.receive(
        response(requests(socket).at(-1) ?? {}, {
          type: "getSnapshot",
          snapshot: { ...bufferedSnapshot(), revision },
        }),
      );
      socket.receive(
        response(requests(socket).at(-1) ?? {}, {
          type: "getBuffer",
          transferId: "transfer-y",
          bufferId: "buffer-y",
          totalBytes: 8,
          chunkBytes: 4,
          chunkCount: 2,
        }),
      );
      socket.receiveBinary(omgp(0, [1, 2, 3, 4], false));
      const existingEvents = [...events];
      const lateEvents: GraphicsClientEvent[] = [];
      client.subscribe((event) => lateEvents.push(event));
      expect(events).toEqual(existingEvents);
      expect(lateEvents).toMatchObject([
        { type: "connection", status: { phase: "attached" } },
      ]);

      socket.receiveBinary(omgp(4, [5, 6, 7, 8], true));
      expect(lateEvents).toHaveLength(2);
      expect(lateEvents.at(-1)).toMatchObject({
        type: "figureReady", figure: { figureId: "figure-1", revision },
      });
      expect(lateEvents.at(-1)).toEqual(events.at(-1));
      const ready = lateEvents.at(-1);
      if (ready?.type !== "figureReady") {
        throw new Error("complete figure missing");
      }
      expect(ready.figure.buffers.get("buffer-y")).toEqual(
        new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]),
      );
      client.dispose();
    },
  );

  it("resyncs one stale close request and reports a retry failure", () => {
    const { client, socket, events } = clientHarness();
    initialize(socket);
    const snapshotRequest = requests(socket)[1];
    if (snapshotRequest === undefined) {
      throw new Error("snapshot request missing");
    }
    socket.receive(
      response(snapshotRequest, {
        type: "getSnapshot",
        snapshot: emptySnapshot("figure-1", 1),
      }),
    );

    expect(client.closeFigure("figure-1")).toBe(true);
    const firstClose = requests(socket)[2];
    expect(requestBody(firstClose ?? {})).toEqual({
      type: "closeFigure",
      figureId: "figure-1",
      expectedRevision: 1,
    });
    socket.receive(
      failure(
        firstClose ?? {},
        "graphics.revisionConflict",
        "Figure revision does not match expectedRevision",
      ),
    );
    const resync = requests(socket)[3];
    expect(requestBody(resync ?? {}).type).toBe("resyncFigure");
    socket.receive(
      response(resync ?? {}, {
        type: "resyncFigure",
        snapshot: emptySnapshot("figure-1", 2),
      }),
    );
    const retry = requests(socket)[4];
    expect(requestBody(retry ?? {})).toEqual({
      type: "closeFigure",
      figureId: "figure-1",
      expectedRevision: 2,
    });
    socket.receive(
      failure(
        retry ?? {},
        "graphics.revisionConflict",
        "Figure changed again",
      ),
    );
    expect(events).toContainEqual({
      type: "figureCloseFailed",
      figureId: "figure-1",
      category: "graphics.revisionConflict",
      message: "Figure changed again",
    });
    expect(requests(socket)).toHaveLength(5);
  });

  it("sends initialize first, then snapshots, then buffer requests", () => {
    const { client, socket, events } = clientHarness();
    socket.open();
    let sent = requests(socket);
    expect(sent).toHaveLength(1);
    expect(requestBody(sent[0] ?? {}).type).toBe("initialize");
    expect(JSON.stringify(sent[0])).toContain("top-secret-token");

    const initializeRequest = sent[0];
    if (initializeRequest === undefined) {
      throw new Error("initialize request missing");
    }
    socket.receive(
      response(initializeRequest, {
        type: "initialize",
        implementation: { name: "openmat-server", version: "0.1.0" },
        capabilities: limits,
        figures: [{ figureId: "figure-1", revision: 1 }],
      }),
    );
    sent = requests(socket);
    expect(requestBody(sent[1] ?? {}).type).toBe("getSnapshot");

    const snapshotRequest = sent[1];
    if (snapshotRequest === undefined) {
      throw new Error("snapshot request missing");
    }
    socket.receive(
      response(snapshotRequest, {
        type: "getSnapshot",
        snapshot: bufferedSnapshot(),
      }),
    );
    sent = requests(socket);
    expect(requestBody(sent[2] ?? {})).toEqual({
      type: "getBuffer",
      bufferId: "buffer-y",
    });

    const bufferRequest = sent[2];
    if (bufferRequest === undefined) {
      throw new Error("buffer request missing");
    }
    socket.receive(
      response(bufferRequest, {
        type: "getBuffer",
        transferId: "transfer-y",
        bufferId: "buffer-y",
        totalBytes: 8,
        chunkBytes: 4,
        chunkCount: 2,
      }),
    );
    socket.receiveBinary(omgp(0, [1, 2, 3, 4], false));
    expect(events.some((event) => event.type === "figureReady")).toBe(false);
    socket.receiveBinary(omgp(4, [5, 6, 7, 8], true));
    const ready = events.find((event) => event.type === "figureReady");
    expect(ready).toMatchObject({
      type: "figureReady",
      figure: { figureId: "figure-1", revision: 1 },
    });
    client.dispose();
  });

  it.each([
    ["gap", [omgp(4, [5, 6, 7, 8], true)]],
    [
      "overlap",
      [omgp(0, [1, 2, 3, 4], false), omgp(2, [3, 4, 5, 6], false)],
    ],
  ])("closes and reconnects after a binary %s", (_name, frames) => {
    const { socket, scheduled } = clientHarness();
    initialize(socket);
    const snapshotRequest = requests(socket)[1];
    if (snapshotRequest === undefined) {
      throw new Error("snapshot request missing");
    }
    socket.receive(
      response(snapshotRequest, {
        type: "getSnapshot",
        snapshot: bufferedSnapshot(),
      }),
    );
    const bufferRequest = requests(socket)[2];
    if (bufferRequest === undefined) {
      throw new Error("buffer request missing");
    }
    socket.receive(
      response(bufferRequest, {
        type: "getBuffer",
        transferId: "transfer-y",
        bufferId: "buffer-y",
        totalBytes: 8,
        chunkBytes: 4,
        chunkCount: 2,
      }),
    );

    frames.forEach((frame) => socket.receiveBinary(frame));
    expect(socket.closes.at(-1)?.code).toBe(4002);
    expect(scheduled).toHaveLength(1);
  });

  it("retains the rendered revision and requests resync on a delta gap", () => {
    const { client, socket, events } = clientHarness();
    initialize(socket);
    const snapshotRequest = requests(socket)[1];
    if (snapshotRequest === undefined) {
      throw new Error("snapshot request missing");
    }
    socket.receive(
      response(snapshotRequest, {
        type: "getSnapshot",
        snapshot: emptySnapshot("figure-1", 1),
      }),
    );
    expect(
      events.find((event) => event.type === "figureReady"),
    ).toMatchObject({ figure: { revision: 1 } });

    socket.receive({
      protocol: "openmat-graphics-v1",
      sessionId: "session-1",
      messageId: "delta-gap",
      kind: "event",
      event: {
        type: "figureDelta",
        figureId: "figure-1",
        baseRevision: 2,
        revision: 3,
        operations: [],
        addedBuffers: [],
        releasedBufferIds: [],
      },
    });
    const resync = requests(socket).find(
      (request) => requestBody(request).type === "resyncFigure",
    );
    expect(requestBody(resync ?? {})).toEqual({
      type: "resyncFigure",
      figureId: "figure-1",
      knownRevision: 1,
    });
    expect(
      events.filter((event) => event.type === "figureReady"),
    ).toHaveLength(1);
    client.dispose();
  });

  it("reconnects after transport close but never reconnects after dispose", () => {
    const { client, socket, sockets, scheduled } = clientHarness();
    initialize(socket);
    socket.serverClose();
    expect(scheduled).toHaveLength(1);
    scheduled.shift()?.();
    expect(sockets).toHaveLength(2);

    const second = sockets[1];
    if (second === undefined) {
      throw new Error("reconnect socket missing");
    }
    second.open();
    client.dispose();
    expect(second.closes.at(-1)?.code).toBe(1000);
    expect(scheduled).toHaveLength(0);
  });

  it("uses one initialized socket for multiple discovered Figures", () => {
    const secondDiscovery: FigureDiscovery = {
      ...discovery,
      figureId: "figure-2",
    };
    const { client, socket, sockets, events } = clientHarness([secondDiscovery]);
    initialize(socket, [
      { figureId: "figure-1", revision: 1 },
      { figureId: "figure-2", revision: 1 },
    ]);
    const snapshots = requests(socket).filter(
      (request) => requestBody(request).type === "getSnapshot",
    );
    expect(snapshots.map((request) => requestBody(request).figureId)).toEqual([
      "figure-1",
      "figure-2",
    ]);
    for (const request of snapshots) {
      const figureId = String(requestBody(request).figureId);
      socket.receive(
        response(request, {
          type: "getSnapshot",
          snapshot: emptySnapshot(figureId, 1),
        }),
      );
    }
    expect(
      events
        .filter((event) => event.type === "figureReady")
        .map((event) =>
          event.type === "figureReady" ? event.figure.figureId : "",
        ),
    ).toEqual(["figure-1", "figure-2"]);
    expect(sockets).toHaveLength(1);
    client.dispose();
  });

  it("redacts the attachment token from user-visible initialization errors", () => {
    const { client, socket, events } = clientHarness();
    socket.open();
    const request = requests(socket)[0];
    if (request === undefined) {
      throw new Error("initialize request missing");
    }
    socket.receive({
      protocol: "openmat-graphics-v1",
      sessionId: "session-1",
      messageId: "failed-init",
      kind: "response",
      replyTo: request.messageId,
      ok: false,
      error: {
        category: "graphics.unauthorized",
        message: "Rejected top-secret-token",
      },
    });
    const status = events.at(-1);
    expect(status).toMatchObject({
      type: "connection",
      status: { phase: "error", message: "Rejected [REDACTED]" },
    });
    expect(JSON.stringify(status)).not.toContain("top-secret-token");
    expect(socket.closes.at(-1)?.code).toBe(4008);
    client.dispose();
  });

  it("selects v2 from discovery and commits limits without forging a snapshot", () => {
    const v2Discovery: FigureDiscovery = {
      schemaVersion: 2,
      graphicsProtocol: "openmat-graphics-v2",
      endpoint: "/graphics/v2",
      attachToken: discovery.attachToken,
      figureId: discovery.figureId,
      revision: discovery.revision,
    };
    const { client, socket, events, urls } = clientHarness([], v2Discovery);
    initialize(socket);
    expect(urls[0]).toMatch(/\/graphics\/v2$/);
    const sent = requests(socket);
    expect(sent[0]?.protocol).toBe("openmat-graphics-v2");
    const snapshotRequest = sent[1];
    if (snapshotRequest === undefined) {
      throw new Error("snapshot request missing");
    }
    const axes = {
      id: "axes",
      generation: 1,
      objectRevision: 1,
      kind: "axes2d",
      parentId: "figure-1-root",
      children: [],
      properties: {
        positionNormalized: [0.1, 0.1, 0.8, 0.8],
        xScale: "linear",
        yScale: "linear",
        xLimits: [0, 1],
        yLimits: [0, 1],
        xLimitsMode: "auto",
        yLimitsMode: "auto",
        nextPlot: "replace",
        gridX: false,
        gridY: false,
        colorOrderIndex: 1,
        titleId: null,
        xLabelId: null,
        yLabelId: null,
      },
    };
    socket.receive(
      response(snapshotRequest, {
        type: "getSnapshot",
        snapshot: {
          ...emptySnapshot("figure-1", 1),
          objects: [
            { ...rootObject("figure-1-root"), children: ["axes"] },
            axes,
          ],
        },
      }),
    );
    expect(
      events.filter((event) => event.type === "figureReady"),
    ).toHaveLength(1);

    client.setAxesLimits("figure-1", 1, [-2, 8], [0.25, 16]);
    const limitRequests = requests(socket).filter(
      (request) => requestBody(request).type === "setAxesLimits",
    );
    expect(limitRequests).toHaveLength(1);
    expect(requestBody(limitRequests[0] ?? {})).toEqual({
      type: "setAxesLimits",
      figureId: "figure-1",
      expectedRevision: 1,
      xLimits: [-2, 8],
      yLimits: [0.25, 16],
    });
    const limitRequest = limitRequests[0];
    if (limitRequest === undefined) {
      throw new Error("limits request missing");
    }
    socket.receive(
      response(limitRequest, {
        type: "setAxesLimits",
        committedRevision: 2,
      }),
    );
    expect(
      events.filter((event) => event.type === "figureReady"),
    ).toHaveLength(1);

    socket.receive({
      protocol: "openmat-graphics-v2",
      sessionId: "session-1",
      messageId: "limits-delta-1",
      kind: "event",
      event: {
        type: "figureDelta",
        figureId: "figure-1",
        baseRevision: 1,
        revision: 2,
        operations: [
          {
            type: "upsertObject",
            object: {
              ...axes,
              objectRevision: 2,
              properties: {
                ...axes.properties,
                xLimits: [-2, 8],
                yLimits: [0.25, 16],
                xLimitsMode: "manual",
                yLimitsMode: "manual",
              },
            },
          },
        ],
        addedBuffers: [],
        releasedBufferIds: [],
      },
    });
    expect(
      events.filter((event) => event.type === "figureReady").at(-1),
    ).toMatchObject({
      figure: {
        revision: 2,
        snapshot: {
          objects: [
            {},
            {
              properties: {
                xLimits: [-2, 8],
                yLimits: [0.25, 16],
                xLimitsMode: "manual",
                yLimitsMode: "manual",
              },
            },
          ],
        },
      },
    });

    client.setAxesCamera("figure-1", 2, [55, 24], 0.8);
    const cameraRequest = requests(socket).find(
      (request) => requestBody(request).type === "setAxesCamera",
    );
    expect(cameraRequest).toBeDefined();
    expect(requestBody(cameraRequest ?? {})).toEqual({
      type: "setAxesCamera",
      figureId: "figure-1",
      expectedRevision: 2,
      view: [55, 24],
      cameraScale: 0.8,
    });
    if (cameraRequest === undefined) {
      throw new Error("camera request missing");
    }
    socket.receive(
      response(cameraRequest, {
        type: "setAxesCamera",
        committedRevision: 3,
      }),
    );
    client.dispose();
  });

  it("keeps setAxesLimits unavailable on a v1 discovery", () => {
    const { client, socket } = clientHarness();
    initialize(socket);
    expect(() =>
      client.setAxesLimits("figure-1", 1, [0, 1], [0, 1]),
    ).toThrow(/graphics-v2/);
    expect(() =>
      client.setAxesCamera("figure-1", 1, [45, 30], 1),
    ).toThrow(/graphics-v2/);
    expect(
      requests(socket).some(
        (request) => requestBody(request).type === "setAxesLimits",
      ),
    ).toBe(false);
    client.dispose();
  });

  it("requires and transmits the explicit Axes target selected by graphics-v3", () => {
    const v3Discovery: FigureDiscovery = {
      schemaVersion: 3,
      graphicsProtocol: "openmat-graphics-v3",
      endpoint: "/graphics/v3",
      attachToken: discovery.attachToken,
      figureId: discovery.figureId,
      revision: discovery.revision,
    };
    const { client, socket, urls } = clientHarness([], v3Discovery);
    initialize(socket);
    expect(urls[0]).toMatch(/\/graphics\/v3$/);
    expect(() =>
      client.setAxesLimits("figure-1", 1, [0, 1], [0, 1]),
    ).toThrow(/axesId/);

    client.setAxesLimits("figure-1", 1, [-2, 8], [0.25, 16], "axes-2");
    client.setAxesCamera("figure-1", 1, [55, 24], 0.8, "axes-2");
    const bodies = requests(socket).map(requestBody);
    expect(bodies.find((body) => body.type === "setAxesLimits")).toEqual({
      type: "setAxesLimits",
      figureId: "figure-1",
      axesId: "axes-2",
      expectedRevision: 1,
      xLimits: [-2, 8],
      yLimits: [0.25, 16],
    });
    expect(bodies.find((body) => body.type === "setAxesCamera")).toEqual({
      type: "setAxesCamera",
      figureId: "figure-1",
      axesId: "axes-2",
      expectedRevision: 1,
      view: [55, 24],
      cameraScale: 0.8,
    });
    client.dispose();
  });
});
