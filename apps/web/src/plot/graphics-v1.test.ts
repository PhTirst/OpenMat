import { describe, expect, it } from "vitest";
import {
  GRAPHICS_ENDPOINT,
  GRAPHICS_ENDPOINT_V2,
  GRAPHICS_ENDPOINT_V3,
  GRAPHICS_ENDPOINT_V4,
  GraphicsV1Error,
  decodeOmgpFrame,
  figureAccessibleName,
  figurePositionCssPixels,
  graphicsWebSocketUrl,
  parseGraphicsObject,
  parseFigureDiscovery,
  resolveGraphicsWebSocketUrl,
  type FigureSnapshot,
} from "./graphics-v1";

function encodeFrame(
  header: Readonly<Record<string, unknown>>,
  payload: readonly number[],
): ArrayBuffer {
  const headerBytes = new TextEncoder().encode(JSON.stringify(header));
  const bytes = new Uint8Array(16 + headerBytes.byteLength + payload.length);
  bytes.set([0x4f, 0x4d, 0x47, 0x50]);
  const view = new DataView(bytes.buffer);
  view.setUint16(4, 1, true);
  view.setUint16(6, 0, true);
  view.setUint32(8, headerBytes.byteLength, true);
  view.setUint32(12, payload.length, true);
  bytes.set(headerBytes, 16);
  bytes.set(payload, 16 + headerBytes.byteLength);
  return bytes.buffer;
}

describe("graphics-v1 discovery and binary framing", () => {
  it("derives graphics-v1 from the configured native Kernel listener", () => {
    expect(
      resolveGraphicsWebSocketUrl("ws://127.0.0.1:51589/kernel", {
        protocol: "http:",
        host: "127.0.0.1:5173",
      }),
    ).toBe("ws://127.0.0.1:51589/graphics/v1");
    expect(() =>
      resolveGraphicsWebSocketUrl("ws://127.0.0.1:51589/lsp"),
    ).toThrow(GraphicsV1Error);
  });

  it("accepts only the frozen same-origin Figure discovery shape", () => {
    const discovery = parseFigureDiscovery(
      JSON.stringify({
        schemaVersion: 1,
        graphicsProtocol: "openmat-graphics-v1",
        endpoint: GRAPHICS_ENDPOINT,
        attachToken: "secret-capability",
        figureId: "figure-1",
        revision: 1,
      }),
    );

    expect(discovery).toMatchObject({
      endpoint: "/graphics/v1",
      figureId: "figure-1",
      revision: 1,
    });
    expect(
      graphicsWebSocketUrl({ protocol: "https:", host: "ide.example" }),
    ).toBe("wss://ide.example/graphics/v1");
  });

  it("selects graphics-v2 only from its exact versioned discovery tuple", () => {
    const discovery = parseFigureDiscovery(
      JSON.stringify({
        schemaVersion: 2,
        graphicsProtocol: "openmat-graphics-v2",
        endpoint: GRAPHICS_ENDPOINT_V2,
        attachToken: "secret-capability",
        figureId: "figure-2",
        revision: 7,
      }),
    );

    expect(discovery).toEqual({
      schemaVersion: 2,
      graphicsProtocol: "openmat-graphics-v2",
      endpoint: "/graphics/v2",
      attachToken: "secret-capability",
      figureId: "figure-2",
      revision: 7,
    });
    expect(
      graphicsWebSocketUrl(
        { protocol: "https:", host: "ide.example" },
        discovery.endpoint,
      ),
    ).toBe("wss://ide.example/graphics/v2");
  });

  it("selects graphics-v3 only from its exact multi-Axes discovery tuple", () => {
    const discovery = parseFigureDiscovery(
      JSON.stringify({
        schemaVersion: 3,
        graphicsProtocol: "openmat-graphics-v3",
        endpoint: GRAPHICS_ENDPOINT_V3,
        attachToken: "secret-capability",
        figureId: "figure-3",
        revision: 9,
      }),
    );

    expect(discovery).toEqual({
      schemaVersion: 3,
      graphicsProtocol: "openmat-graphics-v3",
      endpoint: "/graphics/v3",
      attachToken: "secret-capability",
      figureId: "figure-3",
      revision: 9,
    });
    expect(
      graphicsWebSocketUrl(
        { protocol: "https:", host: "ide.example" },
        discovery.endpoint,
      ),
    ).toBe("wss://ide.example/graphics/v3");
  });

  it("selects graphics-v4 for logarithmic Axes scenes", () => {
    const discovery = parseFigureDiscovery(
      JSON.stringify({
        schemaVersion: 4,
        graphicsProtocol: "openmat-graphics-v4",
        endpoint: GRAPHICS_ENDPOINT_V4,
        attachToken: "secret-capability",
        figureId: "figure-4",
        revision: 11,
      }),
    );
    expect(discovery).toMatchObject({
      schemaVersion: 4,
      endpoint: "/graphics/v4",
      figureId: "figure-4",
    });
  });

  it.each([
    ["wrong schema", { schemaVersion: 2 }],
    ["wrong protocol", { graphicsProtocol: "openmat-graphics-v2" }],
    ["cross endpoint", { endpoint: "wss://elsewhere/graphics/v1" }],
    ["empty token", { attachToken: "" }],
    ["zero revision", { revision: 0 }],
  ])("rejects %s without echoing the token", (_name, replacement) => {
    const encoded = JSON.stringify({
      schemaVersion: 1,
      graphicsProtocol: "openmat-graphics-v1",
      endpoint: "/graphics/v1",
      attachToken: "never-echo-this",
      figureId: "figure-1",
      revision: 1,
      ...replacement,
    });

    expect(() => parseFigureDiscovery(encoded)).toThrow(GraphicsV1Error);
    try {
      parseFigureDiscovery(encoded);
    } catch (error: unknown) {
      expect(String(error)).not.toContain("never-echo-this");
    }
  });

  it("decodes an exact OMGP frame and rejects prefix/length mismatches", () => {
    const frame = encodeFrame(
      {
        type: "bufferChunk",
        transferId: "transfer-1",
        bufferId: "buffer-1",
        offset: 0,
        totalBytes: 4,
        final: true,
      },
      [1, 2, 3, 4],
    );
    expect(decodeOmgpFrame(frame)).toMatchObject({
      transferId: "transfer-1",
      bufferId: "buffer-1",
      offset: 0,
      totalBytes: 4,
      final: true,
    });

    const wrongMagic = frame.slice(0);
    new Uint8Array(wrongMagic)[0] = 0;
    expect(() => decodeOmgpFrame(wrongMagic)).toThrow(/magic/);
    expect(() => decodeOmgpFrame(frame.slice(0, -1))).toThrow(/lengths/);
  });

  it("validates additive Axes ticks without forcing tick-label counts to match", () => {
    const axes = {
      id: "axes-1",
      generation: 1,
      objectRevision: 1,
      kind: "axes2d",
      parentId: "figure-1",
      children: [],
      properties: {
        positionNormalized: [0.1, 0.1, 0.8, 0.8],
        backgroundRgba: [1, 1, 1, 1],
        xScale: "linear",
        yScale: "linear",
        xLimits: [0, 10],
        yLimits: [0, 10],
        xLimitsMode: "auto",
        yLimitsMode: "auto",
        nextPlot: "replace",
        gridX: false,
        gridY: false,
        colorOrderIndex: 1,
        titleId: null,
        xLabelId: null,
        yLabelId: null,
        xTick: ["-Infinity", 0, 5, "Infinity"],
        yTick: [],
        xTickLabelCodeUnits: [[65]],
        yTickLabelCodeUnits: [],
        xTickMode: "manual",
        yTickMode: "manual",
        xTickLabelMode: "manual",
        yTickLabelMode: "manual",
        box: true,
        fontSizeCssPx: 10,
        lineWidthCssPx: 2 / 3,
        tickLabelInterpreter: "latex",
      },
    };
    expect(parseGraphicsObject(axes).kind).toBe("axes2d");
    expect(
      parseGraphicsObject({
        ...axes,
        properties: { ...axes.properties, backgroundRgba: null },
      }).kind,
    ).toBe("axes2d");
    expect(
      parseGraphicsObject({
        ...axes,
        properties: {
          ...axes.properties,
          xScale: "log",
          xLimits: [1, 100],
          yDirection: "reverse",
          visible: false,
        },
      }).kind,
    ).toBe("axes2d");
    expect(() =>
      parseGraphicsObject({
        ...axes,
        properties: { ...axes.properties, xTick: [0, 0] },
      }),
    ).toThrow(/strictly increasing/);
    expect(() =>
      parseGraphicsObject({
        ...axes,
        properties: { ...axes.properties, backgroundRgba: [1, -0.1, 1, 1] },
      }),
    ).toThrow(/backgroundRgba/);
  });

  it("validates additive PolarAxes coordinate and orientation fields", () => {
    const polar = {
      id: "polar-axes-1",
      generation: 1,
      objectRevision: 1,
      kind: "axes2d",
      parentId: "figure-1",
      children: [],
      properties: {
        coordinateSystem: "polar",
        thetaAxisUnits: "degrees",
        thetaDirection: "clockwise",
        thetaZeroLocation: "top",
        rAxisLocation: 90,
        positionNormalized: [0.1, 0.1, 0.8, 0.8],
        xScale: "linear",
        yScale: "linear",
        xLimits: [0, 360],
        yLimits: [0, 1.5],
        xLimitsMode: "manual",
        yLimitsMode: "manual",
        nextPlot: "replace",
        gridX: true,
        gridY: true,
        colorOrderIndex: 1,
        titleId: null,
        xLabelId: null,
        yLabelId: null,
      },
    };
    expect(parseGraphicsObject(polar).kind).toBe("axes2d");
    expect(() =>
      parseGraphicsObject({
        ...polar,
        properties: { ...polar.properties, thetaDirection: "reverse" },
      }),
    ).toThrow(/thetaDirection/);
    expect(() =>
      parseGraphicsObject({
        ...polar,
        properties: { ...polar.properties, xScale: "log" },
      }),
    ).toThrow(/linear theta/);
  });

  it("accepts the server's horizontal southOutside legend without rejecting its Figure", () => {
    const legend = {
      id: "legend-1",
      generation: 1,
      objectRevision: 1,
      kind: "legend",
      parentId: "axes-1",
      children: [],
      properties: {
        seriesIds: ["line-1"],
        labelCodeUnits: [[82, 67, 83]],
        location: "southOutside",
        orientation: "horizontal",
        visible: true,
        backgroundRgba: [1, 1, 1, 1],
        borderRgba: [0.1, 0.1, 0.1, 1],
        fontFamilyCodeUnits: [],
        fontSizeCssPx: 12,
        fontWeight: 400,
        interpreter: "tex",
      },
    };
    expect(parseGraphicsObject(legend)).toMatchObject({
      kind: "legend",
      properties: { location: "southOutside", orientation: "horizontal" },
    });
    expect(() =>
      parseGraphicsObject({
        ...legend,
        properties: { ...legend.properties, location: "unsupported" },
      }),
    ).toThrow(/legend.location/);
  });

  it("preserves exact unordered duplicate uint64 marker indices", () => {
    const dataRef = {
      bufferId: "buffer-1",
      dtype: "f64",
      shape: [1, 1],
      order: "columnMajor",
      endianness: "little",
      byteOffset: 0,
      byteLength: 8,
    };
    const line = {
      id: "line-1",
      generation: 1,
      objectRevision: 1,
      kind: "lineSeries",
      parentId: "axes-1",
      children: [],
      properties: {
        xData: dataRef,
        yData: dataRef,
        colorRgba: [0, 0, 1, 1],
        lineWidthCssPx: 1,
        lineStyle: "solid",
        marker: "circle",
        markerSizeCssPx: 6,
        markerIndices: ["18446744073709551615", "1", "1"],
        displayNameCodeUnits: [],
        visible: true,
        clipping: true,
      },
    };
    expect(parseGraphicsObject(line).kind).toBe("lineSeries");
    expect(() =>
      parseGraphicsObject({
        ...line,
        properties: { ...line.properties, markerIndices: ["0"] },
      }),
    ).toThrow(/positive uint64/);
  });

  it("accepts retained high-level chart containers", () => {
    expect(
      parseGraphicsObject({
        id: "stairs-1",
        generation: 1,
        objectRevision: 1,
        kind: "chartGroup",
        parentId: "axes-1",
        children: ["line-1"],
        properties: {
          chartType: "stair",
          visible: true,
          color: { mode: "auto", rgba: null },
          lineWidthCssPx: 2 / 3,
          lineStyle: "solid",
          marker: "none",
          markerSizeCssPx: 8,
          markerFaceColor: { mode: "none", rgba: null },
          markerEdgeColor: { mode: "auto", rgba: null },
          faceColor: null,
          edgeColor: null,
          faceAlpha: null,
          edgeAlpha: null,
          baseValue: null,
          barWidth: null,
          capSizeCssPx: null,
        },
      }),
    ).toMatchObject({
      kind: "chartGroup",
      properties: { chartType: "stair", visible: true },
    });
    expect(() =>
      parseGraphicsObject({
        id: "future-1",
        generation: 1,
        objectRevision: 1,
        kind: "chartGroup",
        parentId: "axes-1",
        children: [],
        properties: { chartType: "future", visible: true },
      }),
    ).toThrow(/unsupported/);
  });

  it("uses Figure Name and NumberTitle for the native window title", () => {
    const root = {
      id: "figure-root",
      generation: 1,
      objectRevision: 1,
      kind: "figure",
      parentId: null,
      children: [],
      properties: {
        number: 7,
        nameCodeUnits: Array.from("Signal", (character) => character.charCodeAt(0)),
        numberTitle: false,
        visible: true,
        backgroundRgba: [1, 1, 1, 1],
        initialLogicalSizeCssPixels: [560, 420],
        positionCssPixels: [101, 202, 560, 420],
        nextPlot: "add",
      },
    };
    const snapshot: FigureSnapshot = {
      type: "figureSnapshot",
      figureId: "figure-7",
      revision: 1,
      rootId: "figure-root",
      objects: [parseGraphicsObject(root)],
      referencedBuffers: [],
    };
    expect(figureAccessibleName(snapshot)).toBe("Signal");
    expect(figurePositionCssPixels(snapshot)).toEqual([101, 202, 560, 420]);
    const numberedRoot = parseGraphicsObject({
      ...root,
      properties: { ...root.properties, numberTitle: true },
    });
    expect(
      figureAccessibleName({ ...snapshot, objects: [numberedRoot] }),
    ).toBe("Figure 7: Signal");
  });

  it("accepts MATLAB column-vector Scatter SizeData and CData", () => {
    const dataRef = (
      bufferId: string,
      shape: readonly [number, number],
    ) => ({
      bufferId,
      dtype: "f64",
      shape,
      order: "columnMajor",
      endianness: "little",
      byteOffset: 0,
      byteLength: shape[0] * shape[1] * 8,
    });
    const scatter = {
      id: "scatter-1",
      generation: 1,
      objectRevision: 1,
      kind: "scatterSeries",
      parentId: "axes-1",
      children: [],
      properties: {
        xData: dataRef("x", [1, 500]),
        yData: dataRef("y", [1, 500]),
        zData: dataRef("z", [1, 500]),
        sizeData: dataRef("size", [500, 1]),
        colorData: dataRef("color", [500, 1]),
        colorDataTarget: "face",
        marker: "circle",
        markerSizeCssPx: 40,
        markerFaceRgba: [0, 0, 0, 0],
        markerEdgeRgba: [0, 0, 0, 0],
        displayNameCodeUnits: [],
        visible: true,
        clipping: true,
      },
    };

    expect(parseGraphicsObject(scatter).kind).toBe("scatterSeries");
    expect(() =>
      parseGraphicsObject({
        ...scatter,
        properties: {
          ...scatter.properties,
          colorData: dataRef("color", [10, 50]),
        },
      }),
    ).toThrow(/scalar or match the series length/);
    expect(() =>
      parseGraphicsObject({
        ...scatter,
        properties: { ...scatter.properties, colorDataTarget: "vertices" },
      }),
    ).toThrow(/colorDataTarget/);
  });

  it("accepts matrix Surface coordinates, smooth color, and a ColorBar object", () => {
    const dataRef = (
      bufferId: string,
      shape: readonly [number, number],
    ) => ({
      bufferId,
      dtype: "f64",
      shape,
      order: "columnMajor",
      endianness: "little",
      byteOffset: 0,
      byteLength: shape[0] * shape[1] * 8,
    });
    const surface = {
      id: "surface-1",
      generation: 1,
      objectRevision: 1,
      kind: "surfaceSeries",
      parentId: "axes-1",
      children: [],
      properties: {
        xData: dataRef("x", [2, 3]),
        yData: dataRef("y", [2, 3]),
        zData: dataRef("z", [2, 3]),
        cData: dataRef("c", [2, 3]),
        faceColor: "interp",
        faceRgba: null,
        edgeColor: "uniform",
        edgeRgba: [0, 0, 0, 1],
        lineWidthCssPx: 2 / 3,
        cDataMapping: "scaled",
        faceAlpha: 1,
        visible: true,
        clipping: true,
      },
    };

    expect(parseGraphicsObject(surface).kind).toBe("surfaceSeries");
    expect(
      parseGraphicsObject({
        id: "colorbar-1",
        generation: 1,
        objectRevision: 1,
        kind: "colorBar",
        parentId: "axes-1",
        children: [],
        properties: {
          visible: true,
          ticks: [-0.5, 0, 0.5],
          ticksMode: "manual",
          tickLabelCodeUnits: [[108, 111, 119], [122, 101, 114, 111]],
          tickLabelsMode: "manual",
        },
      }).kind,
    ).toBe("colorBar");
    expect(() =>
      parseGraphicsObject({
        id: "colorbar-2",
        generation: 1,
        objectRevision: 1,
        kind: "colorBar",
        parentId: "axes-1",
        children: [],
        properties: {
          visible: true,
          ticks: [0.5, 0.5],
          ticksMode: "manual",
        },
      }),
    ).toThrow(/strictly increasing/);
    expect(() =>
      parseGraphicsObject({
        ...surface,
        properties: {
          ...surface.properties,
          yData: dataRef("y", [1, 2]),
        },
      }),
    ).toThrow(/shapes are inconsistent/);
  });

  it("accepts first-class Patch connectivity and rejects invalid color cardinality", () => {
    const dataRef = (
      bufferId: string,
      shape: readonly [number, number],
    ) => ({
      bufferId,
      dtype: "f64",
      shape,
      order: "columnMajor",
      endianness: "little",
      byteOffset: 0,
      byteLength: shape[0] * shape[1] * 8,
    });
    const patch = {
      id: "patch-1",
      generation: 1,
      objectRevision: 1,
      kind: "patchSeries",
      parentId: "axes-1",
      children: [],
      properties: {
        faces: dataRef("faces", [2, 3]),
        vertices: dataRef("vertices", [4, 3]),
        faceVertexCdata: dataRef("cdata", [4, 1]),
        faceColor: "interp",
        faceRgba: null,
        edgeColor: "uniform",
        edgeRgba: [0, 0, 0, 1],
        lineWidthCssPx: 2 / 3,
        lineStyle: "solid",
        cDataMapping: "scaled",
        faceAlpha: 1,
        edgeAlpha: 1,
        visible: true,
        clipping: true,
      },
    };
    expect(parseGraphicsObject(patch).kind).toBe("patchSeries");
    expect(
      parseGraphicsObject({
        ...patch,
        properties: {
          ...patch.properties,
          vertices: dataRef("vertices", [4, 2]),
          faceAlpha: 0.4,
          edgeAlpha: 0.6,
        },
      }).kind,
    ).toBe("patchSeries");
    expect(() =>
      parseGraphicsObject({
        ...patch,
        properties: { ...patch.properties, faceAlpha: 0.4 },
      }),
    ).toThrow(/3D patchSeries pipeline requires opaque alpha/);
    expect(() =>
      parseGraphicsObject({
        ...patch,
        properties: {
          ...patch.properties,
          faceVertexCdata: dataRef("cdata", [3, 1]),
        },
      }),
    ).toThrow(/one value per face or vertex/);
  });
});
