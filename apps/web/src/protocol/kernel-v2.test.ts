import { describe, expect, it } from "vitest";
import {
  KERNEL_PROTOCOL_V0,
  KERNEL_PROTOCOL_V1,
  KERNEL_PROTOCOL_V2,
  KERNEL_PROTOCOL_V3,
  MAX_KERNEL_FRAME_BYTES,
  SUPPORTED_KERNEL_PROTOCOLS,
  V2_HARD_LIMITS,
  createBootstrapInitializeRequest,
  hasValidV2Capabilities,
  isKernelFrameWithinLimit,
  parseBootstrapKernelServerMessage,
  parseInspectPreview,
  parseKernelServerMessage,
  parseKernelServerTextFrame,
  validateBootstrapInitializeParams,
  validateInspectRequestParams,
  validateSetVariableElementParams,
  type ExactValue,
  type KernelProtocol,
  type PreviewDecodeLimits,
  type TableAggregatePreview,
  type TableExactValue,
  type V2Capabilities,
} from "./kernel-v2";

const capabilities: V2Capabilities = {
  executionModes: ["cell", "repl"],
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 256,
  maxStringElementCodeUnits: 512,
  maxPreviewCodeUnits: 2_048,
  maxAggregateNodes: 1_024,
  maxAggregateElements: 4_096,
  maxAggregateDepth: 16,
  interrupt: true,
  workspaceDelta: true,
};

const LEGACY_PROTOCOL_OFFER = [
  KERNEL_PROTOCOL_V2,
  KERNEL_PROTOCOL_V1,
  KERNEL_PROTOCOL_V0,
] as const;

const v1Capabilities = {
  executionModes: ["cell", "repl"] as const,
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 256,
  maxStringElementCodeUnits: 512,
  maxPreviewCodeUnits: 2_048,
  interrupt: true,
  workspaceDelta: true,
};

const v0Capabilities = {
  executionModes: ["cell", "repl"] as const,
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 256,
  interrupt: true,
  workspaceDelta: true,
};

function initializeResponse(
  negotiatedProtocol: string,
  negotiatedCapabilities: unknown,
) {
  return {
    protocol: KERNEL_PROTOCOL_V0,
    sessionId: "session-2",
    messageId: "initialize-response",
    kind: "response",
    replyTo: "initialize-request",
    ok: true,
    result: {
      type: "initialize",
      data: {
        negotiatedProtocol,
        implementation: { name: "openmat-kernel", version: "2.0.0" },
        capabilities: negotiatedCapabilities,
      },
    },
  };
}

function inspectResponse(
  preview: unknown,
  protocol: KernelProtocol = KERNEL_PROTOCOL_V2,
) {
  return {
    protocol,
    sessionId: "session-2",
    messageId: "inspect-response",
    kind: "response",
    replyTo: "inspect-request",
    ok: true,
    result: { type: "inspect", data: preview },
  };
}

function logicalScalar(value: boolean) {
  return {
    class: "logical",
    size: [1, 1],
    ndims: 2,
    numel: 1,
    complex: false,
    kind: "logical",
    logical: [value],
  };
}

function numericScalar(value: string, imaginary = "0", complex = false) {
  return {
    class: "double",
    size: [1, 1],
    ndims: 2,
    numel: 1,
    complex,
    kind: "numeric",
    real: [value],
    imag: [imaginary],
  };
}

function numericZeros(size: readonly number[]): ExactValue {
  const numel = size.reduce((product, dimension) => product * dimension, 1);
  return {
    class: "double",
    size,
    ndims: size.length,
    numel,
    complex: false,
    kind: "numeric",
    real: Array.from({ length: numel }, () => "0"),
    imag: Array.from({ length: numel }, () => "0"),
  };
}

function logicalValues(values: readonly boolean[]): ExactValue {
  return {
    class: "logical",
    size: [values.length, 1],
    ndims: 2,
    numel: values.length,
    complex: false,
    kind: "logical",
    logical: values,
  };
}

function exactTable(
  rows: number,
  variableNames: readonly string[],
  variables: readonly ExactValue[],
): TableExactValue {
  return {
    class: "table",
    size: [rows, variableNames.length],
    ndims: 2,
    numel: rows * variableNames.length,
    complex: false,
    kind: "table",
    variableNames,
    variables,
  };
}

function completeCellPreview(items: readonly unknown[]) {
  return {
    class: "cell",
    dimensions: [1, items.length],
    complex: false,
    selectedRange: { start: [1, 1], size: [1, items.length] },
    kind: "cell",
    items,
    truncation: { truncated: false, omittedElements: 0 },
    usage: {
      nodes: 1 + items.length,
      elements: items.length * 2,
      codeUnits: 0,
      depth: items.length === 0 ? 0 : 1,
    },
  };
}

function completeTablePreview(): TableAggregatePreview {
  return {
    class: "table",
    dimensions: [4, 3],
    complex: false,
    selectedRange: { start: [2, 1], size: [2, 3] },
    kind: "table",
    variableNames: ["温度", "Flag😀"],
    variables: [numericZeros([2, 2, 2]), logicalValues([true, false])],
    truncation: { truncated: true, omittedElements: 1 },
    usage: { nodes: 3, elements: 12, codeUnits: 8, depth: 1 },
  };
}

function withLimits(
  changes: Partial<PreviewDecodeLimits>,
): PreviewDecodeLimits {
  return { ...V2_HARD_LIMITS, ...changes };
}

describe("kernel-v2 bootstrap and negotiated envelopes", () => {
  it("constructs the preference-ordered offer in a v0 bootstrap envelope", () => {
    const params = {
      client: { name: "openmat-web", version: "2.0.0" },
      supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
      capabilities,
    };
    const request = createBootstrapInitializeRequest(
      "session-2",
      "initialize-request",
      params,
    );

    expect(validateBootstrapInitializeParams(params)).toBeNull();
    expect(request).toMatchObject({
      protocol: KERNEL_PROTOCOL_V0,
      request: {
        type: "initialize",
        params: {
          supportedProtocols: [KERNEL_PROTOCOL_V3],
          capabilities: {
            maxAggregateNodes: 1_024,
            maxAggregateElements: 4_096,
            maxAggregateDepth: 16,
          },
        },
      },
    });
  });

  it("requires unique offers and all six valid v2 limits", () => {
    const base = {
      client: { name: "openmat-web", version: "2.0.0" },
      supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
      capabilities,
    };
    expect(
      validateBootstrapInitializeParams({
        ...base,
        supportedProtocols: [KERNEL_PROTOCOL_V3, KERNEL_PROTOCOL_V3],
      }),
    ).not.toBeNull();

    for (const invalidCapabilities of [
      { ...capabilities, maxAggregateNodes: 0 },
      { ...capabilities, maxAggregateElements: 65_537 },
      { ...capabilities, maxAggregateDepth: 33 },
      { ...capabilities, maxAggregateDepth: -1 },
      { ...capabilities, maxAggregateNodes: 1.5 },
      { ...capabilities, maxPreviewCodeUnits: 511 },
      { ...capabilities, maxAggregateElements: undefined },
    ]) {
      expect(hasValidV2Capabilities(invalidCapabilities as V2Capabilities)).toBe(
        false,
      );
      expect(
        validateBootstrapInitializeParams({
          ...base,
          capabilities: invalidCapabilities as V2Capabilities,
        }),
      ).not.toBeNull();
    }
    expect(
      hasValidV2Capabilities({ ...capabilities, maxAggregateDepth: 0 }),
    ).toBe(true);
  });

  it("accepts v3 production negotiation and retains explicit legacy decoding", () => {
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V3, capabilities),
        capabilities,
        SUPPORTED_KERNEL_PROTOCOLS,
      ),
    ).toMatchObject({
      messageType: "response",
      response: {
        protocol: KERNEL_PROTOCOL_V0,
        result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V3 } },
      },
    });
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V2, capabilities),
        capabilities,
        LEGACY_PROTOCOL_OFFER,
      ),
    ).toMatchObject({
      messageType: "response",
      response: {
        protocol: KERNEL_PROTOCOL_V0,
        result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V2 } },
      },
    });
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V1, v1Capabilities),
        capabilities,
        LEGACY_PROTOCOL_OFFER,
      ),
    ).not.toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V0, v0Capabilities),
        capabilities,
        LEGACY_PROTOCOL_OFFER,
      ),
    ).not.toBeNull();
  });

  it("rejects malformed, over-offer, contaminated downgrade, and unknown selections", () => {
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V2, {
          ...capabilities,
          maxAggregateElements: capabilities.maxAggregateElements + 1,
        }),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V2, {
          ...capabilities,
          maxAggregateDepth: undefined,
        }),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V1, capabilities),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V0, v1Capabilities),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse("openmat-kernel-future", capabilities),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V2, capabilities),
        capabilities,
        [KERNEL_PROTOCOL_V1, KERNEL_PROTOCOL_V0],
      ),
    ).toBeNull();
  });

  it("requires every post-bootstrap envelope to use the negotiated protocol", () => {
    const idle = {
      protocol: KERNEL_PROTOCOL_V2,
      sessionId: "session-2",
      messageId: "idle-1",
      kind: "event",
      event: { type: "status", data: { status: "idle" } },
    };
    expect(parseKernelServerMessage(idle, KERNEL_PROTOCOL_V2)).not.toBeNull();
    expect(
      parseKernelServerMessage(
        { ...idle, protocol: KERNEL_PROTOCOL_V1 },
        KERNEL_PROTOCOL_V2,
      ),
    ).toBeNull();

    const unknownEvent = {
      ...idle,
      event: { type: "futureEvent", data: { retained: true } },
    };
    expect(parseKernelServerMessage(unknownEvent, KERNEL_PROTOCOL_V2)).toMatchObject(
      { messageType: "unknownEvent" },
    );
    expect(
      parseKernelServerMessage(
        {
          ...inspectResponse({}),
          result: { type: "futureResult", data: {} },
        },
        KERNEL_PROTOCOL_V2,
      ),
    ).toBeNull();
  });

  it("keeps v0/v1 captures unchanged and rejects aggregate downgrade payloads", () => {
    const v0Matrix = {
      class: "double",
      dimensions: [1, 1],
      selectedRange: { start: [1, 1], size: [1, 1] },
      values: [{ kind: "number", value: 1 }],
      truncation: { truncated: false, omittedElements: 0 },
    };
    expect(
      parseKernelServerMessage(
        inspectResponse(v0Matrix, KERNEL_PROTOCOL_V0),
        KERNEL_PROTOCOL_V0,
      ),
    ).not.toBeNull();

    const aggregate = completeCellPreview([logicalScalar(true)]);
    expect(
      parseKernelServerMessage(
        inspectResponse(aggregate, KERNEL_PROTOCOL_V1),
        KERNEL_PROTOCOL_V1,
      ),
    ).toBeNull();
    expect(
      parseKernelServerMessage(
        inspectResponse(aggregate, KERNEL_PROTOCOL_V0),
        KERNEL_PROTOCOL_V0,
      ),
    ).toBeNull();
  });
});

describe("kernel-v3 versioned workspace mutation", () => {
  const matrix = {
    class: "double",
    dimensions: [1, 1],
    complex: false,
    selectedRange: { start: [1, 1], size: [1, 1] },
    values: [{ kind: "number", value: 3 }],
    truncation: { truncated: false, omittedElements: 0 },
  } as const;

  it("decodes revision-paired reads and element-write acknowledgements", () => {
    expect(
      parseKernelServerMessage(
        inspectResponse(
          { revision: 7, preview: matrix },
          KERNEL_PROTOCOL_V3,
        ),
        KERNEL_PROTOCOL_V3,
      ),
    ).toMatchObject({
      messageType: "response",
      response: { result: { data: { revision: 7, preview: matrix } } },
    });

    const setResponse = {
      protocol: KERNEL_PROTOCOL_V3,
      sessionId: "session-2",
      messageId: "set-response",
      kind: "response",
      replyTo: "set-request",
      ok: true,
      result: {
        type: "setVariableElement",
        data: {
          revision: 8,
          variable: {
            name: "A",
            class: "double",
            dimensions: [1, 1],
            complex: false,
            bytes: 8,
          },
        },
      },
    };
    expect(
      parseKernelServerMessage(setResponse, KERNEL_PROTOCOL_V3),
    ).toMatchObject({
      messageType: "response",
      response: { result: { data: { revision: 8 } } },
    });
    expect(
      parseKernelServerMessage(
        {
          ...setResponse,
          result: {
            ...setResponse.result,
            data: { ...setResponse.result.data, revision: -1 },
          },
        },
        KERNEL_PROTOCOL_V3,
      ),
    ).toBeNull();
  });

  it("validates one-based indices, revisions, identifiers, and canonical numbers", () => {
    expect(
      validateSetVariableElementParams({
        name: "matrix_2",
        indices: [2, 3],
        value: { real: "-1.25e+4", imaginary: "+Inf" },
        expectedRevision: 9,
      }),
    ).toBeNull();
    for (const invalid of [
      {
        name: "not valid",
        indices: [1, 1],
        value: { real: "1", imaginary: "0" },
        expectedRevision: 0,
      },
      {
        name: "A",
        indices: [0, 1],
        value: { real: "1", imaginary: "0" },
        expectedRevision: 0,
      },
      {
        name: "A",
        indices: [1, 1],
        value: { real: "01", imaginary: "0" },
        expectedRevision: 0,
      },
      {
        name: "A",
        indices: [1, 1],
        value: { real: "1", imaginary: "0" },
        expectedRevision: -1,
      },
    ]) {
      expect(validateSetVariableElementParams(invalid)).not.toBeNull();
    }
  });
});

describe("kernel-v2 positive exact preview decoding", () => {
  it("accepts the canonical nested cell with an ordered shaped-empty struct", () => {
    const preview = {
      class: "cell",
      dimensions: [1, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 2] },
      kind: "cell",
      items: [
        numericScalar("7"),
        {
          class: "struct",
          size: [0, 3],
          ndims: 2,
          numel: 0,
          complex: false,
          kind: "struct",
          fields: ["beta", "alpha"],
          records: [],
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 3, elements: 3, codeUnits: 9, depth: 1 },
    };

    expect(parseInspectPreview(preview)).toBe(preview);
    expect(
      parseKernelServerMessage(inspectResponse(preview), KERNEL_PROTOCOL_V2),
    ).toMatchObject({ response: { result: { data: preview } } });
  });

  it("preserves a top-level shaped-empty struct and its ordered schema", () => {
    const preview = {
      class: "struct",
      dimensions: [0, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [0, 3] },
      kind: "struct",
      fields: ["f"],
      records: [],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 0, codeUnits: 1, depth: 0 },
    };
    expect(parseInspectPreview(preview)).toBe(preview);
  });

  it("accepts a shaped-empty cell and an unfielded scalar struct", () => {
    const emptyCell = {
      class: "cell",
      dimensions: [0, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [0, 3] },
      kind: "cell",
      items: [],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 0, codeUnits: 0, depth: 0 },
    };
    const unfieldedStruct = {
      class: "struct",
      dimensions: [1, 1],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 1] },
      kind: "struct",
      fields: [],
      records: [{}],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 1, codeUnits: 0, depth: 0 },
    };
    expect(parseInspectPreview(emptyCell)).toBe(emptyCell);
    expect(parseInspectPreview(unfieldedStruct)).toBe(unfieldedStruct);
  });

  it("accepts nested shaped cells, strings, and record member order independent of fields", () => {
    const preview = {
      class: "struct",
      dimensions: [1, 1],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 1] },
      kind: "struct",
      fields: ["beta", "alpha"],
      records: [
        {
          alpha: {
            class: "string",
            size: [1, 2],
            ndims: 2,
            numel: 2,
            complex: false,
            kind: "string",
            string_code_units: [[55_357], []],
            missing: [false, true],
          },
          beta: {
            class: "cell",
            size: [0, 3],
            ndims: 2,
            numel: 0,
            complex: false,
            kind: "cell",
            items: [],
          },
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 3, elements: 3, codeUnits: 10, depth: 1 },
    };
    expect(parseInspectPreview(preview)).toBe(preview);
  });

  it("accepts recursively nested tables with wide variables and Unicode names", () => {
    const nested = exactTable(2, ["Flag"], [logicalValues([true, false])]);
    const table = exactTable(
      2,
      ["温度", "Nested😀"],
      [numericZeros([2, 2, 2]), nested],
    );
    const preview = {
      ...completeCellPreview([table]),
      usage: { nodes: 5, elements: 17, codeUnits: 14, depth: 3 },
    };

    expect(parseInspectPreview(preview)).toBe(preview);
    expect(
      parseKernelServerMessage(inspectResponse(preview), KERNEL_PROTOCOL_V2),
    ).toMatchObject({ response: { result: { data: preview } } });
  });

  it("accepts a nonzero-height table with no variables", () => {
    const empty = exactTable(3, [], []);
    const preview = {
      ...completeCellPreview([empty]),
      usage: { nodes: 2, elements: 1, codeUnits: 0, depth: 1 },
    };

    expect(parseInspectPreview(preview)).toBe(preview);
  });

  it("accepts the canonical top-level table variable prefix", () => {
    const preview = completeTablePreview();

    expect(parseInspectPreview(preview)).toBe(preview);
    expect(
      parseKernelServerMessage(inspectResponse(preview), KERNEL_PROTOCOL_V2),
    ).toMatchObject({ response: { result: { data: preview } } });
  });

  it("preserves 0xN and Nx0 top-level table selections", () => {
    const zeroRows = {
      class: "table",
      dimensions: [0, 3],
      complex: false,
      selectedRange: { start: [1, 2], size: [0, 2] },
      kind: "table",
      variableNames: ["A", "B"],
      variables: [numericZeros([0, 2]), logicalValues([])],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 3, elements: 2, codeUnits: 2, depth: 1 },
    } satisfies TableAggregatePreview;
    const zeroVariables = {
      class: "table",
      dimensions: [3, 0],
      complex: false,
      selectedRange: { start: [1, 1], size: [3, 0] },
      kind: "table",
      variableNames: [],
      variables: [],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 0, codeUnits: 0, depth: 0 },
    } satisfies TableAggregatePreview;

    expect(parseInspectPreview(zeroRows)).toBe(zeroRows);
    expect(parseInspectPreview(zeroVariables)).toBe(zeroVariables);
  });

  it("accepts every scalar leaf without coercing strings or code units", () => {
    const items = [
      numericScalar("NaN", "-0.1e+2", true),
      {
        class: "uint64",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "integer",
        integer: [{ real: "18446744073709551615", imaginary: "0" }],
      },
      logicalScalar(true),
      {
        class: "char",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "char",
        code_units: [55_357],
      },
      {
        class: "string",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "string",
        string_code_units: [[]],
        missing: [false],
      },
    ];
    const preview = {
      ...completeCellPreview(items),
      usage: { nodes: 6, elements: 10, codeUnits: 1, depth: 1 },
    };
    expect(parseInspectPreview(preview)).toBe(preview);
  });

  it("retains the v1 MatrixPreview as the exact non-aggregate leaf", () => {
    const matrix = {
      class: "char",
      dimensions: [1, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 2] },
      values: [
        { kind: "charCodeUnit", value: 55_357 },
        { kind: "charCodeUnit", value: 56_898 },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    expect(parseInspectPreview(matrix)).toBe(matrix);
    expect(
      parseKernelServerMessage(inspectResponse(matrix), KERNEL_PROTOCOL_V2),
    ).not.toBeNull();
  });
});

describe("kernel-v2 budgets, ranges, and whole-element truncation", () => {
  it("validates outgoing inspect names, one-based ranges, products, and limits", () => {
    const valid = {
      name: "workspace_value",
      range: { start: [1, 1], size: [2, 3] },
      maxElements: 6,
    };
    expect(validateInspectRequestParams(valid)).toBeNull();
    for (const params of [
      { ...valid, name: "" },
      { ...valid, range: { start: [1], size: [1] } },
      { ...valid, range: { start: [0, 1], size: [1, 1] } },
      {
        ...valid,
        range: {
          start: [1, 1],
          size: [Number.MAX_SAFE_INTEGER, 2],
        },
      },
      { ...valid, maxElements: 0 },
      { ...valid, maxElements: V2_HARD_LIMITS.maxPreviewElements + 1 },
    ]) {
      expect(validateInspectRequestParams(params)).not.toBeNull();
    }
  });

  it("accepts an exact aggregate-element boundary and a whole-item prefix", () => {
    const preview = {
      class: "cell",
      dimensions: [1, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 2] },
      kind: "cell",
      items: [logicalScalar(true)],
      truncation: { truncated: true, omittedElements: 1 },
      usage: { nodes: 2, elements: 2, codeUnits: 0, depth: 1 },
    };
    const limits = withLimits({ maxAggregateElements: 2 });
    expect(parseInspectPreview(preview, limits)).toBe(preview);
    expect(
      parseInspectPreview(
        {
          ...preview,
          items: [logicalScalar(true), logicalScalar(false)],
          truncation: { truncated: false, omittedElements: 0 },
          usage: { nodes: 3, elements: 4, codeUnits: 0, depth: 1 },
        },
        limits,
      ),
    ).toBeNull();
  });

  it("enforces request maxElements independently from negotiated limits", () => {
    const preview = completeCellPreview([
      logicalScalar(true),
      logicalScalar(false),
    ]);
    expect(parseInspectPreview(preview, V2_HARD_LIMITS, 2)).toBe(preview);
    expect(parseInspectPreview(preview, V2_HARD_LIMITS, 1)).toBeNull();
    expect(parseInspectPreview(preview, V2_HARD_LIMITS, 0)).toBeNull();

    const table = completeTablePreview();
    expect(parseInspectPreview(table, V2_HARD_LIMITS, 2)).toBe(table);
    expect(parseInspectPreview(table, V2_HARD_LIMITS, 1)).toBeNull();
  });

  it("checks node, element, depth, code-unit, per-node, and per-string limits", () => {
    const nested = {
      class: "cell",
      size: [1, 1],
      ndims: 2,
      numel: 1,
      complex: false,
      kind: "cell",
      items: [logicalScalar(true)],
    };
    const preview = {
      ...completeCellPreview([nested]),
      usage: { nodes: 3, elements: 3, codeUnits: 0, depth: 2 },
    };
    expect(
      parseInspectPreview(
        preview,
        withLimits({
          maxAggregateNodes: 3,
          maxAggregateElements: 3,
          maxAggregateDepth: 2,
        }),
      ),
    ).toBe(preview);
    for (const limits of [
      withLimits({ maxAggregateNodes: 2 }),
      withLimits({ maxAggregateElements: 2 }),
      withLimits({ maxAggregateDepth: 1 }),
    ]) {
      expect(parseInspectPreview(preview, limits)).toBeNull();
    }

    const charPreview = {
      ...completeCellPreview([
        {
          class: "char",
          size: [1, 2],
          ndims: 2,
          numel: 2,
          complex: false,
          kind: "char",
          code_units: [1, 2],
        },
      ]),
      usage: { nodes: 2, elements: 3, codeUnits: 2, depth: 1 },
    };
    expect(
      parseInspectPreview(
        charPreview,
        withLimits({ maxStringElementCodeUnits: 2, maxPreviewCodeUnits: 2 }),
      ),
    ).toBe(charPreview);
    expect(
      parseInspectPreview(
        charPreview,
        withLimits({ maxStringElementCodeUnits: 1, maxPreviewCodeUnits: 1 }),
      ),
    ).toBeNull();
    expect(
      parseInspectPreview(
        charPreview,
        withLimits({ maxPreviewElements: 1 }),
      ),
    ).toBeNull();

    const stringPreview = {
      ...completeCellPreview([
        {
          class: "string",
          size: [1, 1],
          ndims: 2,
          numel: 1,
          complex: false,
          kind: "string",
          string_code_units: [[1, 2]],
          missing: [false],
        },
      ]),
      usage: { nodes: 2, elements: 2, codeUnits: 2, depth: 1 },
    };
    expect(
      parseInspectPreview(
        stringPreview,
        withLimits({ maxStringElementCodeUnits: 1, maxPreviewCodeUnits: 2 }),
      ),
    ).toBeNull();

    const rootSchema = {
      class: "struct",
      dimensions: [0, 0],
      complex: false,
      selectedRange: { start: [1, 1], size: [0, 0] },
      kind: "struct",
      fields: ["aa"],
      records: [],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 0, codeUnits: 2, depth: 0 },
    };
    expect(
      parseInspectPreview(
        rootSchema,
        withLimits({ maxStringElementCodeUnits: 2, maxPreviewCodeUnits: 2 }),
      ),
    ).toBe(rootSchema);
    expect(
      parseInspectPreview(
        rootSchema,
        withLimits({ maxStringElementCodeUnits: 1, maxPreviewCodeUnits: 1 }),
      ),
    ).toBeNull();
  });

  it("rejects unsafe shapes, overflowed products, and invalid one-based ranges", () => {
    const valid = completeCellPreview([logicalScalar(true)]);
    for (const malformed of [
      { ...valid, dimensions: [1, Number.MAX_SAFE_INTEGER + 1] },
      { ...valid, dimensions: [Number.MAX_SAFE_INTEGER, 2] },
      { ...valid, dimensions: [1] },
      {
        ...valid,
        selectedRange: { start: [0, 1], size: [1, 1] },
      },
      {
        ...valid,
        dimensions: [2, 2],
        selectedRange: { start: [2, 2], size: [2, 1] },
      },
      {
        ...valid,
        selectedRange: {
          start: [1, 1],
          size: [Number.MAX_SAFE_INTEGER, 2],
        },
      },
    ]) {
      expect(parseInspectPreview(malformed)).toBeNull();
    }
  });

  it("recomputes truncation and usage instead of trusting producer claims", () => {
    const valid = completeCellPreview([logicalScalar(true)]);
    for (const malformed of [
      {
        ...valid,
        truncation: { truncated: true, omittedElements: 0 },
      },
      {
        ...valid,
        truncation: { truncated: false, omittedElements: 1 },
      },
      { ...valid, usage: { ...valid.usage, nodes: 99 } },
      { ...valid, usage: { ...valid.usage, elements: 1 } },
      { ...valid, usage: { ...valid.usage, depth: 0 } },
      {
        ...valid,
        usage: { ...valid.usage, codeUnits: Number.MAX_SAFE_INTEGER + 1 },
      },
    ]) {
      expect(parseInspectPreview(malformed)).toBeNull();
    }
  });
});

describe("kernel-v2 malicious and inconsistent payload rejection", () => {
  it("rejects unknown preview/exact kinds and lossy v0 leaves", () => {
    const valid = completeCellPreview([logicalScalar(true)]);
    expect(parseInspectPreview({ ...valid, kind: "future" })).toBeNull();
    expect(
      parseInspectPreview({
        ...valid,
        items: [{ ...logicalScalar(true), kind: "object" }],
      }),
    ).toBeNull();
    expect(
      parseInspectPreview({
        ...valid,
        items: [
          {
            class: "ExampleHandle",
            size: [1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            kind: "object",
            fields: [],
            records: [{}],
          },
        ],
      }),
    ).toBeNull();
    expect(
      parseInspectPreview({
        class: "string",
        dimensions: [1, 1],
        complex: false,
        selectedRange: { start: [1, 1], size: [1, 1] },
        values: [{ kind: "text", value: "lossy" }],
        truncation: { truncated: false, omittedElements: 0 },
      }),
    ).toBeNull();
  });

  it("requires canonical exact property sets, metadata, and payload lengths", () => {
    const valid = completeCellPreview([logicalScalar(true)]);
    for (const badItem of [
      { ...logicalScalar(true), extra: "forbidden" },
      { ...logicalScalar(true), class: "double" },
      { ...logicalScalar(true), complex: true },
      { ...logicalScalar(true), size: [1, 2] },
      { ...logicalScalar(true), ndims: 3 },
      { ...logicalScalar(true), numel: 2 },
      { ...logicalScalar(true), logical: [] },
      {
        ...logicalScalar(true),
        truncation: { truncated: true, omittedElements: 1 },
      },
    ]) {
      expect(parseInspectPreview({ ...valid, items: [badItem] })).toBeNull();
    }
    expect(parseInspectPreview({ ...valid, extra: true })).toBeNull();
    expect(
      parseInspectPreview({
        ...valid,
        selectedRange: { ...valid.selectedRange, extra: true },
      }),
    ).toBeNull();
  });

  it("validates numeric grammar, complex consistency, and exact integer ranges", () => {
    const valid = completeCellPreview([logicalScalar(true)]);
    for (const badItem of [
      numericScalar("nan"),
      numericScalar("+1"),
      numericScalar("01"),
      numericScalar("1."),
      numericScalar("1e"),
      { ...numericScalar("1"), real: [1] },
      numericScalar("1", "-0", false),
      numericScalar("1", "0.0", true),
      {
        class: "int8",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "integer",
        integer: [{ real: "128", imaginary: "0" }],
      },
      {
        class: "uint8",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "integer",
        integer: [{ real: "-1", imaginary: "0" }],
      },
      {
        class: "int64",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: true,
        kind: "integer",
        integer: [{ real: "1", imaginary: "0" }],
      },
      {
        class: "int64",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "integer",
        integer: [{ real: "01", imaginary: "0" }],
      },
      {
        class: "int64",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "integer",
        integer: [{ real: "9".repeat(100_000), imaginary: "0" }],
      },
    ]) {
      expect(parseInspectPreview({ ...valid, items: [badItem] })).toBeNull();
    }
  });

  it("enforces string missing form, code-unit ranges, fields, and record sets", () => {
    const valid = completeCellPreview([logicalScalar(true)]);
    for (const badItem of [
      {
        class: "string",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "string",
        string_code_units: [[65]],
        missing: [true],
      },
      {
        class: "char",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "char",
        code_units: [65_536],
      },
      {
        class: "struct",
        size: [0, 0],
        ndims: 2,
        numel: 0,
        complex: false,
        kind: "struct",
        fields: ["same", "same"],
        records: [],
      },
      {
        class: "struct",
        size: [0, 0],
        ndims: 2,
        numel: 0,
        complex: false,
        kind: "struct",
        fields: ["α"],
        records: [],
      },
      {
        class: "struct",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "struct",
        fields: ["beta", "alpha"],
        records: [{ beta: logicalScalar(true) }],
      },
      {
        class: "struct",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "struct",
        fields: ["beta"],
        records: [
          { beta: logicalScalar(true), extra: logicalScalar(false) },
        ],
      },
    ]) {
      expect(parseInspectPreview({ ...valid, items: [badItem] })).toBeNull();
    }
  });

  it("rejects malformed exact table schemas and inconsistent variable rows", () => {
    const first = numericZeros([2, 1]);
    const second = logicalValues([true, false]);
    const table = exactTable(2, ["A", "B"], [first, second]);
    const valid = {
      ...completeCellPreview([table]),
      usage: { nodes: 4, elements: 9, codeUnits: 2, depth: 2 },
    };

    for (const malformed of [
      { ...table, variableNames: ["A", "A"] },
      { ...table, variableNames: ["A"] },
      { ...table, variableNames: ["", "B"] },
      { ...table, variables: [first] },
      { ...table, variables: [numericZeros([1, 1]), second] },
      { ...table, class: "struct" },
      { ...table, complex: true },
      { ...table, size: [2, 2, 1], ndims: 3 },
      { ...table, extra: true },
    ]) {
      expect(parseInspectPreview({ ...valid, items: [malformed] })).toBeNull();
    }
  });

  it("rejects malformed top-level tables and counts omitted variables", () => {
    const valid = completeTablePreview();
    for (const malformed of [
      { ...valid, variableNames: ["温度", "温度"] },
      { ...valid, variableNames: ["温度"] },
      { ...valid, variables: valid.variables.slice(0, 1) },
      {
        ...valid,
        variables: [numericZeros([3, 1]), valid.variables[1]],
      },
      { ...valid, dimensions: [4, 3, 1] },
      { ...valid, truncation: { truncated: true, omittedElements: 4 } },
      { ...valid, truncation: { truncated: false, omittedElements: 1 } },
      { ...valid, class: "struct" },
      { ...valid, complex: true },
      { ...valid, extra: true },
    ]) {
      expect(parseInspectPreview(malformed)).toBeNull();
    }
  });

  it("enforces top-level table recursive usage limits and producer claims", () => {
    const preview = completeTablePreview();
    const exactLimits = withLimits({
      maxAggregateNodes: 3,
      maxAggregateElements: 12,
      maxAggregateDepth: 1,
      maxStringElementCodeUnits: 8,
      maxPreviewCodeUnits: 8,
    });

    expect(parseInspectPreview(preview, exactLimits)).toBe(preview);
    for (const limits of [
      withLimits({ maxAggregateNodes: 2 }),
      withLimits({ maxAggregateElements: 11 }),
      withLimits({ maxAggregateDepth: 0 }),
      withLimits({
        maxStringElementCodeUnits: 7,
        maxPreviewCodeUnits: 7,
      }),
    ]) {
      expect(parseInspectPreview(preview, limits)).toBeNull();
    }
    expect(
      parseInspectPreview({
        ...preview,
        usage: { ...preview.usage, elements: 11 },
      }),
    ).toBeNull();
  });

  it("charges exact table names, nodes, elements, and recursive depth", () => {
    const table = exactTable(1, ["A😀"], [
      {
        class: "char",
        size: [1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        kind: "char",
        code_units: [65],
      },
    ]);
    const preview = {
      ...completeCellPreview([table]),
      usage: { nodes: 3, elements: 3, codeUnits: 4, depth: 2 },
    };
    const exactLimits = withLimits({
      maxAggregateNodes: 3,
      maxAggregateElements: 3,
      maxAggregateDepth: 2,
      maxStringElementCodeUnits: 4,
      maxPreviewCodeUnits: 4,
    });

    expect(parseInspectPreview(preview, exactLimits)).toBe(preview);
    for (const limits of [
      withLimits({ maxAggregateNodes: 2 }),
      withLimits({ maxAggregateElements: 2 }),
      withLimits({ maxAggregateDepth: 1 }),
      withLimits({
        maxStringElementCodeUnits: 3,
        maxPreviewCodeUnits: 3,
      }),
    ]) {
      expect(parseInspectPreview(preview, limits)).toBeNull();
    }
    expect(
      parseInspectPreview({
        ...preview,
        usage: { ...preview.usage, codeUnits: 3 },
      }),
    ).toBeNull();
  });

  it("rejects active-path cycles but allows harmless sibling sharing", () => {
    const cyclic: Record<string, unknown> = {
      class: "cell",
      size: [1, 1],
      ndims: 2,
      numel: 1,
      complex: false,
      kind: "cell",
      items: [],
    };
    (cyclic.items as unknown[]).push(cyclic);
    const cyclicPreview = {
      ...completeCellPreview([cyclic]),
      usage: { nodes: 3, elements: 3, codeUnits: 0, depth: 2 },
    };
    expect(parseInspectPreview(cyclicPreview)).toBeNull();

    const shared = logicalScalar(true);
    const sharedPreview = completeCellPreview([shared, shared]);
    expect(parseInspectPreview(sharedPreview)).toBe(sharedPreview);
  });
});

describe("kernel-v2 text frame bound", () => {
  it("measures actual UTF-8 bytes at and over the one-mebibyte boundary", () => {
    expect(isKernelFrameWithinLimit("a".repeat(MAX_KERNEL_FRAME_BYTES))).toBe(
      true,
    );
    expect(
      isKernelFrameWithinLimit("a".repeat(MAX_KERNEL_FRAME_BYTES + 1)),
    ).toBe(false);
    expect(
      isKernelFrameWithinLimit(
        `é${"a".repeat(MAX_KERNEL_FRAME_BYTES - 2)}`,
      ),
    ).toBe(true);
    expect(
      isKernelFrameWithinLimit(
        `é${"a".repeat(MAX_KERNEL_FRAME_BYTES - 1)}`,
      ),
    ).toBe(false);
  });

  it("decodes valid bounded text without stringifying unknown payload kinds", () => {
    const frame = JSON.stringify(
      inspectResponse(completeCellPreview([logicalScalar(true)])),
    );
    expect(
      parseKernelServerTextFrame(frame, KERNEL_PROTOCOL_V2),
    ).not.toBeNull();
    expect(
      parseKernelServerTextFrame("not-json", KERNEL_PROTOCOL_V2),
    ).toBeNull();
    expect(
      parseKernelServerTextFrame(
        " ".repeat(MAX_KERNEL_FRAME_BYTES + 1),
        KERNEL_PROTOCOL_V2,
      ),
    ).toBeNull();
  });
});
