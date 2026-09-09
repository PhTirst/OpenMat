# OpenMat kernel protocol v1

This document freezes the additive value and negotiation changes relative to
`openmat-kernel-v0`. Unless replaced below, every v0 request, response, event,
ordering rule, envelope field, error rule, and unknown-event rule remains part
of v1.

## Identifiers and bootstrap initialization

The negotiated v1 protocol identifier is `openmat-kernel-v1`.

Initialization uses v0 as a bootstrap so an installed v0 kernel can genuinely
downgrade:

1. The client sends initialize in an `openmat-kernel-v0` envelope. A production
   v1 client offers `openmat-kernel-v1` first and `openmat-kernel-v0` second in
   `supportedProtocols`. Entries are unique and ordered by client preference.
2. The kernel selects the first offered identifier it supports. A v1 kernel
   also supports v0. No common identifier returns `protocol.noCommonVersion`.
3. The initialize response remains an `openmat-kernel-v0` envelope and reports
   the selection in `result.data.negotiatedProtocol`.
4. The envelope immediately following the initialize response, including an
   `idle` status event, uses the selected identifier. Every later envelope in
   the session uses that identifier. Mid-session fallback is forbidden.

An unknown or mismatched envelope protocol after initialization is a protocol
failure. Unknown message kinds, request types, response-result types, and
`PreviewValue.kind` values are failures. Unknown event types retain the v0 rule:
they are ignored because events have an explicit forward-compatible unknown
representation.

Golden bootstrap request:

```json
{
  "protocol": "openmat-kernel-v0",
  "sessionId": "session-1",
  "messageId": "client-1",
  "kind": "request",
  "request": {
    "type": "initialize",
    "params": {
      "client": {"name": "openmat-web", "version": "1.0.0"},
      "supportedProtocols": ["openmat-kernel-v1", "openmat-kernel-v0"],
      "capabilities": {
        "executionModes": ["file", "cell", "repl"],
        "displayMimeTypes": ["text/plain"],
        "maxPreviewElements": 4096,
        "maxStringElementCodeUnits": 16384,
        "maxPreviewCodeUnits": 65536,
        "interrupt": true,
        "workspaceDelta": true
      }
    }
  }
}
```

Golden successful response; it is the last bootstrap-v0 envelope:

```json
{
  "protocol": "openmat-kernel-v0",
  "sessionId": "session-1",
  "messageId": "kernel-1",
  "kind": "response",
  "replyTo": "client-1",
  "ok": true,
  "result": {
    "type": "initialize",
    "data": {
      "negotiatedProtocol": "openmat-kernel-v1",
      "implementation": {"name": "openmat-kernel", "version": "1.0.0"},
      "capabilities": {
        "executionModes": ["file", "cell", "repl"],
        "displayMimeTypes": ["text/plain"],
        "maxPreviewElements": 4096,
        "maxStringElementCodeUnits": 16384,
        "maxPreviewCodeUnits": 65536,
        "interrupt": true,
        "workspaceDelta": true
      }
    }
  }
}
```

If v1 is offered, both v1 code-unit capability fields are required. Each must be
a positive safe JSON integer. `maxStringElementCodeUnits` is at most 16,384;
`maxPreviewCodeUnits` is at most 65,536 and is not less than the per-element
limit. The result contains the component-wise minimum of client, kernel, and
hard limits. An invalid v1 capability offer returns
`protocol.invalidCapabilities`; it does not silently select different bounds.
When v0 is selected, the response omits the two v1-only fields and all later
behavior is exactly v0.

## Bounded inspection

V1 retains `MAX_PREVIEW_ELEMENTS = 4096` and the v0 `inspect` request. The
negotiated limits also impose:

- `MAX_STRING_ELEMENT_CODE_UNITS = 16384`;
- `MAX_PREVIEW_CODE_UNITS = 65536`.

All structural JSON integers, including dimensions, ranges, counts, and limits,
are non-negative safe integers no greater than 9,007,199,254,740,991. Range
starts are one-based. Shapes have at least two dimensions. `dimensions`,
`selectedRange.start`, and `selectedRange.size` have the same rank, and a
nonempty selected extent lies within the full dimensions. Products are checked
without overflow.

`MatrixPreview` adds a required `complex` Boolean. Its `values` are the longest
column-major prefix of the selected range that the negotiated element and code-
unit budgets permit. The invariant is:

```text
values.length + truncation.omittedElements
    == product(selectedRange.size)
truncation.truncated == (truncation.omittedElements != 0)
values.length <= min(request.maxElements, negotiated maxPreviewElements)
```

`inspect.params.maxElements` is a positive integer no greater than the
negotiated `maxPreviewElements`.

Every returned string element is whole. Missing elements have zero code units.
If complete individually valid elements exceed the aggregate code-unit budget,
the kernel stops before the first element that would cross it and reports that
element and all later selected elements as omitted. While building the prefix,
if an element reached before another budget establishes the truncation boundary
exceeds the negotiated per-element limit, the entire inspect request fails with
`workspace.unsupportedValue`; no partial element is returned. Elements after an
already established boundary need not be visited. A producer never inserts a
replacement code unit or UTF-8 replacement character.

The hard limits are chosen so one element cannot bypass the element-count bound:
16,384 code units are 32 KiB of raw UTF-16, while the 65,536-code-unit aggregate
is 128 KiB raw and remains below the existing one-mebibyte frame budget even as
a JSON array of five-digit decimal integers and separators.

## Preview value kinds

`PreviewValue` remains an internally tagged JSON object with a camelCase `kind`.
V1 adds these canonical kinds:

```text
charCodeUnit { value: integer 0..65535 }
integer      { real: decimal-string, imaginary: decimal-string }
string       { codeUnits: [integer 0..65535], missing: boolean }
```

An integer decimal string matches `0|-?[1-9][0-9]*`. It has no leading plus,
leading zero, whitespace, exponent, decimal point, or negative zero. Components
of an unsigned class cannot be negative. Both components must be within the
fixed-width range identified by `MatrixPreview.class`. JSON numbers and `f64`
intermediates are forbidden. When `complex` is false, every imaginary component
is exactly `"0"`; when it is true, at least one is nonzero.

For `kind: "string"`, `missing: true` requires an empty `codeUnits` array. An
empty array with `missing: false` is an empty non-missing string. Surrogate code
units are accepted individually and are never validated as Unicode scalar
text.

The class, complex marker, and canonical emitted kinds agree as follows:

| `class` | `complex` | canonical value kinds |
| --- | --- | --- |
| `char` | false | `charCodeUnit` |
| `string` | false | `string` |
| `logical` | false | `logical` |
| eight integer classes | false/true | `integer` |
| `double`, `single` | false | `number`, `special` |
| `double`, `single` | true | `complex` |

The eight integer class strings are `int8`, `uint8`, `int16`, `uint16`, `int32`,
`uint32`, `int64`, and `uint64`. V1 transports real and complex storage for all
eight. This table does not define arithmetic or promotion behavior.

For a complex single or double preview, every selected element uses
`kind: "complex"`, including elements whose imaginary component is numerically
zero, and both components must be finite JSON numbers. The existing `special`
kind carries only one spelling and therefore cannot identify which component is
non-finite. If either component of any selected complex floating-point element
is NaN or infinity, the entire inspect request returns
`workspace.unsupportedValue`; it is not truncated and is not encoded as
`special` or bare `missing`. A future lossless dual-component special encoding
requires another protocol version.

The following is a negative example and must be rejected as a v1 preview rather
than interpreted as a complex NaN:

```json
{
  "class": "double",
  "dimensions": [1, 1],
  "complex": true,
  "selectedRange": {"start": [1, 1], "size": [1, 1]},
  "values": [{"kind": "special", "value": "nan"}],
  "truncation": {"truncated": false, "omittedElements": 0}
}
```

The v0 `text` and bare `missing` kinds remain known to shared decoders for old
captures, but they are non-canonical in a v1 inspect response and must not carry
char, integer, string, or unsupported-value truth. Values outside the table fail
with `workspace.unsupportedValue`. Unknown preview kinds are a decoding failure;
adding another kind requires another protocol version.

Golden char preview:

```json
{
  "class": "char",
  "dimensions": [1, 3],
  "complex": false,
  "selectedRange": {"start": [1, 1], "size": [1, 3]},
  "values": [
    {"kind": "charCodeUnit", "value": 65},
    {"kind": "charCodeUnit", "value": 55357},
    {"kind": "charCodeUnit", "value": 56898}
  ],
  "truncation": {"truncated": false, "omittedElements": 0}
}
```

Golden exact complex-integer preview:

```json
{
  "class": "int64",
  "dimensions": [1, 2],
  "complex": true,
  "selectedRange": {"start": [1, 1], "size": [1, 2]},
  "values": [
    {
      "kind": "integer",
      "real": "-9223372036854775808",
      "imaginary": "0"
    },
    {"kind": "integer", "real": "7", "imaginary": "-9"}
  ],
  "truncation": {"truncated": false, "omittedElements": 0}
}
```

Golden string preview; the first element is an isolated high surrogate, the
second is empty, and the third is missing:

```json
{
  "class": "string",
  "dimensions": [1, 3],
  "complex": false,
  "selectedRange": {"start": [1, 1], "size": [1, 3]},
  "values": [
    {"kind": "string", "codeUnits": [55357], "missing": false},
    {"kind": "string", "codeUnits": [], "missing": false},
    {"kind": "string", "codeUnits": [], "missing": true}
  ],
  "truncation": {"truncated": false, "omittedElements": 0}
}
```

## V0 downgrade behavior

V0 messages and golden payloads do not change. Under a v0 selection, inspect of
char or integer returns `workspace.unsupportedValue`. String may use v0 `text`
only when all selected elements are non-missing and exactly representable as
Unicode scalar text. An isolated surrogate or missing string returns
`workspace.unsupportedValue`. Display events remain non-normative and may
perform an explicitly lossy rendering; inspect and workspace-change detection
may not reuse that rendering as value truth.
