import { describe, expect, it } from "vitest";
import {
  KERNEL_PROTOCOL_V0,
  KERNEL_PROTOCOL_V1,
  MAX_PREVIEW_CODE_UNITS,
  MAX_STRING_ELEMENT_CODE_UNITS,
  SUPPORTED_KERNEL_PROTOCOLS,
  createBootstrapInitializeRequest,
  parseBootstrapKernelServerMessage,
  parseKernelServerMessage,
  type PreviewDecodeLimits,
  type V1Capabilities,
  type V1MatrixPreview,
} from "./kernel-v1";

const capabilities: V1Capabilities = {
  executionModes: ["cell"],
  displayMimeTypes: ["text/plain"],
  maxPreviewElements: 256,
  maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
  maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
  interrupt: true,
  workspaceDelta: true,
};

const v0Capabilities = {
  executionModes: ["cell"],
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
    sessionId: "session-1",
    messageId: "initialize-response",
    kind: "response",
    replyTo: "initialize-request",
    ok: true,
    result: {
      type: "initialize",
      data: {
        negotiatedProtocol,
        implementation: { name: "openmat-kernel", version: "1.0.0" },
        capabilities: negotiatedCapabilities,
      },
    },
  };
}

function inspectResponse(preview: unknown) {
  return {
    protocol: KERNEL_PROTOCOL_V1,
    sessionId: "session-1",
    messageId: "inspect-response",
    kind: "response",
    replyTo: "inspect-request",
    ok: true,
    result: { type: "inspect", data: preview },
  };
}

function parsePreview(
  preview: unknown,
  limits?: PreviewDecodeLimits,
) {
  return parseKernelServerMessage(
    inspectResponse(preview),
    KERNEL_PROTOCOL_V1,
    limits,
  );
}

function singleValuePreview(
  previewClass: V1MatrixPreview["class"],
  complex: boolean,
  value: unknown,
) {
  return {
    class: previewClass,
    dimensions: [1, 1],
    complex,
    selectedRange: { start: [1, 1], size: [1, 1] },
    values: [value],
    truncation: { truncated: false, omittedElements: 0 },
  };
}

describe("kernel-v1 bootstrap", () => {
  it("sends the preference-ordered offer in a v0 bootstrap envelope", () => {
    const request = createBootstrapInitializeRequest(
      "session-1",
      "initialize-request",
      {
        client: { name: "openmat-web", version: "1.0.0" },
        supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
        capabilities,
      },
    );

    expect(request).toMatchObject({
      protocol: KERNEL_PROTOCOL_V0,
      request: {
        type: "initialize",
        params: {
          supportedProtocols: [KERNEL_PROTOCOL_V1, KERNEL_PROTOCOL_V0],
          capabilities: {
            maxStringElementCodeUnits: 16_384,
            maxPreviewCodeUnits: 65_536,
          },
        },
      },
    });
  });

  it("accepts v1 negotiation and exact v0 downgrade responses", () => {
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V1, capabilities),
        capabilities,
      ),
    ).toMatchObject({
      messageType: "response",
      response: {
        protocol: KERNEL_PROTOCOL_V0,
        result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V1 } },
      },
    });
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V0, v0Capabilities),
        capabilities,
      ),
    ).toMatchObject({
      response: {
        result: { data: { negotiatedProtocol: KERNEL_PROTOCOL_V0 } },
      },
    });
  });

  it("rejects malformed negotiated capabilities and unknown selections", () => {
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V1, {
          ...capabilities,
          maxStringElementCodeUnits: 0,
        }),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V1, {
          ...capabilities,
          maxStringElementCodeUnits: 200,
          maxPreviewCodeUnits: 199,
        }),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V1, {
          ...capabilities,
          maxPreviewCodeUnits: capabilities.maxPreviewCodeUnits + 1,
        }),
        capabilities,
      ),
    ).toBeNull();
    expect(
      parseBootstrapKernelServerMessage(
        initializeResponse(KERNEL_PROTOCOL_V0, {
          ...v0Capabilities,
          maxStringElementCodeUnits: 1,
        }),
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
        initializeResponse(KERNEL_PROTOCOL_V1, capabilities),
        capabilities,
        [KERNEL_PROTOCOL_V0],
      ),
    ).toBeNull();
  });
});

describe("kernel-v1 strict preview decoding", () => {
  it("preserves char code units, isolated surrogates, empty, and missing", () => {
    const charPreview = {
      class: "char",
      dimensions: [1, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 3] },
      values: [
        { kind: "charCodeUnit", value: 65 },
        { kind: "charCodeUnit", value: 55_357 },
        { kind: "charCodeUnit", value: 56_898 },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    const stringPreview = {
      class: "string",
      dimensions: [1, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 3] },
      values: [
        { kind: "string", codeUnits: [55_357], missing: false },
        { kind: "string", codeUnits: [], missing: false },
        { kind: "string", codeUnits: [], missing: true },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };

    expect(parsePreview(charPreview)).toMatchObject({
      response: { result: { data: charPreview } },
    });
    expect(parsePreview(stringPreview)).toMatchObject({
      response: { result: { data: stringPreview } },
    });
  });

  it("accepts uint64 max and signed complex integers without number coercion", () => {
    const unsigned = singleValuePreview("uint64", false, {
      kind: "integer",
      real: "18446744073709551615",
      imaginary: "0",
    });
    const complexInteger = singleValuePreview("int64", true, {
      kind: "integer",
      real: "-9223372036854775808",
      imaginary: "-9",
    });

    expect(parsePreview(unsigned)).not.toBeNull();
    expect(parsePreview(complexInteger)).not.toBeNull();
    const parsed = parsePreview(unsigned);
    expect(
      parsed?.messageType === "response" &&
        parsed.response.ok &&
        parsed.response.result.type === "inspect"
        ? parsed.response.result.data.values[0]
        : null,
    ).toEqual({
      kind: "integer",
      real: "18446744073709551615",
      imaginary: "0",
    });
  });

  it("enforces class-kind, complex, and finite floating-point rules", () => {
    expect(
      parsePreview(
        singleValuePreview("double", true, {
          kind: "complex",
          real: 1,
          imaginary: -2,
        }),
      ),
    ).not.toBeNull();
    expect(
      parsePreview(
        singleValuePreview("double", true, {
          kind: "special",
          value: "nan",
        }),
      ),
    ).toBeNull();
    expect(
      parsePreview(
        singleValuePreview("single", true, {
          kind: "complex",
          real: Number.POSITIVE_INFINITY,
          imaginary: 0,
        }),
      ),
    ).toBeNull();
    expect(
      parsePreview(
        singleValuePreview("logical", false, {
          kind: "number",
          value: 1,
        }),
      ),
    ).toBeNull();
    expect(
      parsePreview(
        singleValuePreview("string", false, {
          kind: "text",
          value: "lossy",
        }),
      ),
    ).toBeNull();
    expect(
      parsePreview(
        singleValuePreview("char", true, {
          kind: "charCodeUnit",
          value: 65,
        }),
      ),
    ).toBeNull();
  });

  it("rejects unsafe dimensions, bad ranges, count, and truncation", () => {
    const valid = singleValuePreview("char", false, {
      kind: "charCodeUnit",
      value: 65,
    });
    expect(
      parsePreview({ ...valid, dimensions: [1, Number.MAX_SAFE_INTEGER + 1] }),
    ).toBeNull();
    expect(parsePreview({ ...valid, dimensions: [1] })).toBeNull();
    expect(
      parsePreview({
        ...valid,
        dimensions: [2, 2],
        selectedRange: { start: [2, 2], size: [2, 1] },
      }),
    ).toBeNull();
    expect(
      parsePreview({
        ...valid,
        dimensions: [Number.MAX_SAFE_INTEGER, 2],
        selectedRange: { start: [1, 1], size: [1, 1] },
      }),
    ).toBeNull();
    expect(
      parsePreview({
        ...valid,
        truncation: { truncated: true, omittedElements: 0 },
      }),
    ).toBeNull();
    expect(
      parsePreview({
        ...valid,
        truncation: { truncated: false, omittedElements: 1 },
      }),
    ).toBeNull();
    expect(
      parsePreview(valid, {
        maxPreviewElements: 0,
        maxStringElementCodeUnits: 1,
        maxPreviewCodeUnits: 1,
      }),
    ).toBeNull();
  });

  it("rejects out-of-range code units and noncanonical integer decimals", () => {
    for (const value of [-1, 65_536, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      expect(
        parsePreview(
          singleValuePreview("char", false, {
            kind: "charCodeUnit",
            value,
          }),
        ),
      ).toBeNull();
    }

    for (const [previewClass, real, imaginary] of [
      ["uint64", "18446744073709551616", "0"],
      ["uint8", "-1", "0"],
      ["int8", "128", "0"],
      ["int64", "01", "0"],
      ["int64", "-0", "0"],
      ["int64", "1.0", "0"],
      ["int64", 1, "0"],
      ["int64", "1", "2"],
    ] as const) {
      expect(
        parsePreview(
          singleValuePreview(previewClass, false, {
            kind: "integer",
            real,
            imaginary,
          }),
        ),
      ).toBeNull();
    }
    expect(
      parsePreview(
        singleValuePreview("int64", true, {
          kind: "integer",
          real: "1",
          imaginary: "0",
        }),
      ),
    ).toBeNull();
  });

  it("enforces missing canonical form and string code-unit budgets", () => {
    expect(
      parsePreview(
        singleValuePreview("string", false, {
          kind: "string",
          codeUnits: [65],
          missing: true,
        }),
      ),
    ).toBeNull();
    expect(
      parsePreview(
        singleValuePreview("string", false, {
          kind: "string",
          codeUnits: [65_536],
          missing: false,
        }),
      ),
    ).toBeNull();
    expect(
      parsePreview(
        singleValuePreview("string", false, {
          kind: "string",
          codeUnits: Array.from(
            { length: MAX_STRING_ELEMENT_CODE_UNITS + 1 },
            () => 65,
          ),
          missing: false,
        }),
      ),
    ).toBeNull();

    const aggregateValues = [
      ...Array.from({ length: 4 }, () => ({
        kind: "string",
        codeUnits: Array.from(
          { length: MAX_STRING_ELEMENT_CODE_UNITS },
          () => 65,
        ),
        missing: false,
      })),
      { kind: "string", codeUnits: [65], missing: false },
    ];
    expect(
      parsePreview({
        class: "string",
        dimensions: [1, 5],
        complex: false,
        selectedRange: { start: [1, 1], size: [1, 5] },
        values: aggregateValues,
        truncation: { truncated: false, omittedElements: 0 },
      }),
    ).toBeNull();
  });
});
