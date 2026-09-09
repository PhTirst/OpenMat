# OpenMat kernel protocol v2

This document freezes the additive aggregate-observation and negotiation changes
relative to `openmat-kernel-v1`. Unless replaced below, every v1 rule remains
part of v2, including all v0 requests, responses, events, ordering, error,
unknown-event, and bootstrap rules inherited by v1.

V2 does not modify a v0 or v1 JSON payload. It adds a new negotiated identifier,
aggregate capabilities, and a v2-only `AggregatePreview` containing recursive
`ExactValue` objects.

## Identifier and bootstrap initialization

The negotiated identifier is `openmat-kernel-v2`.

Initialization remains a v0 bootstrap exchange:

1. A production v2 client sends `initialize` in an `openmat-kernel-v0`
   envelope and offers these unique identifiers in this preference order:
   `openmat-kernel-v2`, `openmat-kernel-v1`, `openmat-kernel-v0`.
2. The kernel selects the first offered identifier it supports. A v2 kernel
   supports v2, v1, and v0. No common identifier returns
   `protocol.noCommonVersion`.
3. The initialize response is the last v0 envelope and names the selection in
   `result.data.negotiatedProtocol`.
4. The immediately following envelope, including an `idle` event, uses the
   selected identifier. Mid-session fallback or renegotiation is forbidden.

An old v0 kernel may ignore the new capability fields and select v0. A v1
kernel may select v1 using the unmodified v1 rules. A v2 implementation must
also continue to accept a conforming v1 client's `[v1,v0]` offer and a v0-only
client's offer without requiring v2 fields.

Golden v2 bootstrap request:

```json
{
  "protocol": "openmat-kernel-v0",
  "sessionId": "session-2",
  "messageId": "client-1",
  "kind": "request",
  "request": {
    "type": "initialize",
    "params": {
      "client": {"name": "openmat-web", "version": "2.0.0"},
      "supportedProtocols": [
        "openmat-kernel-v2",
        "openmat-kernel-v1",
        "openmat-kernel-v0"
      ],
      "capabilities": {
        "executionModes": ["file", "cell", "repl"],
        "displayMimeTypes": ["text/plain"],
        "maxPreviewElements": 4096,
        "maxStringElementCodeUnits": 16384,
        "maxPreviewCodeUnits": 65536,
        "maxAggregateNodes": 16384,
        "maxAggregateElements": 65536,
        "maxAggregateDepth": 32,
        "interrupt": true,
        "workspaceDelta": true
      }
    }
  }
}
```

Golden successful response; this remains a bootstrap-v0 envelope:

```json
{
  "protocol": "openmat-kernel-v0",
  "sessionId": "session-2",
  "messageId": "kernel-1",
  "kind": "response",
  "replyTo": "client-1",
  "ok": true,
  "result": {
    "type": "initialize",
    "data": {
      "negotiatedProtocol": "openmat-kernel-v2",
      "implementation": {"name": "openmat-kernel", "version": "2.0.0"},
      "capabilities": {
        "executionModes": ["file", "cell", "repl"],
        "displayMimeTypes": ["text/plain"],
        "maxPreviewElements": 4096,
        "maxStringElementCodeUnits": 16384,
        "maxPreviewCodeUnits": 65536,
        "maxAggregateNodes": 16384,
        "maxAggregateElements": 65536,
        "maxAggregateDepth": 32,
        "interrupt": true,
        "workspaceDelta": true
      }
    }
  }
}
```

### Capability validation

If v2 is offered, all six numeric limits shown above are required. The three
v1 limits retain their exact v1 meaning and validation. Numeric limits are safe
JSON integers. The accepted ranges are:

| Capability | Minimum | Hard maximum |
| --- | ---: | ---: |
| `maxPreviewElements` | 1 | 4,096 |
| `maxStringElementCodeUnits` | 1 | 16,384 |
| `maxPreviewCodeUnits` | 1 | 65,536 |
| `maxAggregateNodes` | 1 | 16,384 |
| `maxAggregateElements` | 1 | 65,536 |
| `maxAggregateDepth` | 0 | 32 |

`maxPreviewCodeUnits` must not be less than
`maxStringElementCodeUnits`. No ordering constraint exists between
`maxPreviewElements` and `maxAggregateElements`: the latter may deliberately
make a top-level preview shorter.

The result is the component-wise minimum of the valid client values, kernel
limits, and hard maxima. Offering v2 with a missing, malformed, out-of-range, or
inconsistent field returns `protocol.invalidCapabilities`; the kernel does not
silently select v1 or repair a value. Because v2 also uses the v1 string limits,
an invalid v1 capability in a v2 offer is equally fatal.

If v1 is selected, the response and all later behavior are exactly the frozen
v1 contract and the three v2-only aggregate fields are omitted. If v0 is
selected, all v1- and v2-only fields are omitted and behavior is exactly v0.

Unknown or mismatched envelope protocols after initialization remain protocol
failures. Unknown request, response-result, preview, or exact-value kinds are
failures; they are not a request to guess a later version. Unknown event types
retain the forward-compatible v0 rule.

## Bounded inspection result union

V2 retains the v1 `inspect` request, its one-based ranges, its positive
`params.maxElements`, and every structural safe-integer and checked-product
rule. `params.maxElements` is at most the negotiated `maxPreviewElements`.

For a non-cell/non-struct value, a successful v2 inspect result is the exact v1
`MatrixPreview` JSON shape and uses the exact v1 `PreviewValue` table. V2 does
not add a v1 preview kind or change v1 handling of non-finite complex values.

For a cell or struct, a successful v2 inspect result is `AggregatePreview`.
The canonical cell variant contains exactly these fields:

```text
{
  class: "cell",
  dimensions: [safe-integer, ...],
  complex: false,
  selectedRange: {start: [safe-integer, ...], size: [safe-integer, ...]},
  kind: "cell",
  items: [ExactValue, ...],
  truncation: {truncated: boolean, omittedElements: safe-integer},
  usage: {nodes: safe-integer, elements: safe-integer,
          codeUnits: safe-integer, depth: safe-integer}
}
```

The canonical struct variant contains exactly these fields:

```text
{
  class: "struct",
  dimensions: [safe-integer, ...],
  complex: false,
  selectedRange: {start: [safe-integer, ...], size: [safe-integer, ...]},
  kind: "struct",
  fields: [field-name, ...],
  records: [{field-name: ExactValue, ...}, ...],
  truncation: {truncated: boolean, omittedElements: safe-integer},
  usage: {nodes: safe-integer, elements: safe-integer,
          codeUnits: safe-integer, depth: safe-integer}
}
```

`dimensions` is the complete workspace value shape. `selectedRange` uses the
same rank and in-bounds rules as v1. A selection with a zero extent preserves
that exact shaped extent and returns no top-level elements.

`items` or `records` is the longest whole top-level column-major prefix allowed
by the request limit, negotiated limits, recursive budgets, and encoded frame
bound. A cell item is its complete contained value. A struct record contains
one complete value for every field. Nested `ExactValue` objects are never
truncated.

The top-level invariants are:

```text
returned = items.length                         // cell
returned = records.length                       // struct
selected = checked_product(selectedRange.size)

returned + truncation.omittedElements == selected
truncation.truncated == (truncation.omittedElements != 0)
returned <= min(request.maxElements, negotiated maxPreviewElements)
```

Cell items and struct records are ordered by the selected aggregate's
column-major element traversal. The `fields` array is the only representation
of observable field order. It contains unique names and is present even for a
shaped empty struct. Every record's key set is exactly the `fields` set, with no
missing or extra member. JSON object-member order is semantically ignored;
deterministic producers emit record members in `fields` order.

Field names match the first-tranche ASCII subset
`^[A-Za-z][A-Za-z0-9_]*$`. Unicode names, empty names, and other strings are not
losslessly representable by this tranche.

## Recursive `ExactValue`

An `ExactValue` uses the conformance schema-v2 normalized value shape, not the
element-tagged v1 `PreviewValue` shape. Every variant has these common fields:

```text
{
  class: nonempty-string,
  size: [safe-integer, ...],
  ndims: safe-integer,
  numel: safe-integer,
  complex: boolean,
  kind: exact-value-kind,
  ... one canonical payload ...
}
```

`size` has at least two dimensions, `ndims == size.length`, and `numel` equals
the checked product of `size`. Every flat payload has exactly `numel` entries
in column-major order. Each exact node has `numel` no greater than the
negotiated `maxPreviewElements`; unlike the root preview selection, a nested
node is never prefix-truncated.

The canonical variants are:

| `kind` | Required `class` / `complex` | Required payload |
| --- | --- | --- |
| `numeric` | `double` or `single` | `real` and `imag`, arrays of number strings |
| `integer` | one of the eight v1 integer classes | `integer`, an array of `{real, imaginary}` decimal strings |
| `logical` | `logical`, `false` | `logical`, an array of Booleans |
| `char` | `char`, `false` | `code_units`, an array of integers in `0..65535` |
| `string` | `string`, `false` | parallel `string_code_units` and `missing` arrays |
| `cell` | `cell`, `false` | `items`, an array of complete `ExactValue` objects |
| `struct` | `struct`, `false` | ordered `fields` and column-major `records` |

No other payload property is present for a variant. In particular, an
`ExactValue` never contains `dimensions`, `selectedRange`, `truncation`,
`usage`, or a v1 `PreviewValue` object.

Number strings follow the schema-v2 grammar:

```text
NaN | +Inf | -Inf | -?(0|[1-9][0-9]*)(.[0-9]+)?([eE][+-]?[0-9]+)?
```

The decimal point in the grammar is literal. Existing schema-v2 numeric
normalization and comparison rules determine equivalent finite formatting;
JSON numbers are not substituted for these strings. If `complex` is false,
every numeric `imag` entry is exactly `"0"`. If it is true, at least one
imaginary component is not numerically zero.

Integer strings retain the exact v1 grammar, signedness, and fixed-width range
rules. If `complex` is false every `imaginary` component is exactly `"0"`; if
true at least one is nonzero. JSON numbers and `f64` conversion are forbidden.

A missing string has `missing: true` and an empty parallel code-unit sequence.
An empty non-missing string has the same empty sequence and `missing: false`.
Surrogate code units are exact and are not validated as Unicode scalar text.

For an exact cell, `items.length == numel`, including zero for any shaped empty
cell. Items are in cell column-major order. For an exact struct,
`records.length == numel`; records are column-major and values within each
record are traversed in `fields` order. An unfielded scalar struct uses
`records: [{}]`; a fielded shaped empty struct retains `fields` and uses
`records: []`.

Although conformance schema-v2 structurally contains `kind: "object"`, v2 does
not accept object or function-handle `ExactValue` payloads. Encountering either
as a top-level value or anywhere in a returned aggregate candidate fails inspect
with `workspace.unsupportedValue`. `Nothing`, opaque VM markers, and every
other unlisted kind fail the same way.

## Traversal, counters, and deterministic ordering

Traversal is depth-first pre-order. The `AggregatePreview` root is depth zero
and counts as one node. A top-level cell content or struct field value is depth
one. A child of an exact cell item or exact struct field is one depth greater
than its containing exact aggregate. Records and field-schema entries are not
nodes.

Before attempting top-level elements, the producer validates the root shape and
schema, starts `nodes` at one, `elements` at zero, `depth` at zero, and charges
each root struct field name once in `fields` order. The complete selected extent
is not charged to `elements`; one root element is charged only when its complete
cell item or struct record is committed to the returned prefix. This permits a
large selected range to be represented by a bounded prefix.

For each candidate top-level element, the producer uses temporary counters and
temporary JSON storage. It charges one root element, then traverses the complete
candidate as follows:

1. On entering an exact value, validate identity/kind and canonical metadata,
   check its depth and per-node `numel`, then charge one node and that node's
   complete `numel`.
2. A char charges its code units in array order. A string checks and charges
   each element's code units in array order; missing elements charge zero.
3. An exact cell traverses items in column-major order.
4. An exact struct charges each field name once in `fields` order, then
   traverses records column-major and each record's values in `fields` order.
   Record-key repetitions do not charge code units again.

`codeUnits` counts only exact char data, exact string data, and field-schema
names, all as UTF-16 code units. Protocol metadata such as class names, kind
tags, session identifiers, and JSON member names is not part of this semantic
counter; it is bounded separately by encoded frame size.

`usage` reports only the committed returned tree:

- `nodes` is the root plus every returned `ExactValue` node;
- `elements` is the number of returned root elements plus the sum of `numel`
  for every returned exact node;
- `codeUnits` is the semantic total defined above;
- `depth` is the greatest committed node depth and is zero for a preview with
  no returned child.

A decoder or verifier recomputes all four values; it never trusts `usage` as a
claim by the producer.

### Check and error precedence

For a visited node, checks occur in this order:

1. repeated active identity on the current recursion path;
2. supported kind and canonical class/kind/payload metadata;
3. depth;
4. checked shape product and the per-node element limit;
5. node count, aggregate element count, payload code units, and recursive
   children in the traversal order above.

Every addition and shape product uses checked arithmetic before allocation or
serialization. A runtime value whose dimensions, selected count, omission
count, or counter cannot be represented as a safe JSON integer fails with
`workspace.previewLimit`. An incoming inspect request whose range product is not
a safe JSON integer fails the inherited `protocol.validation` request boundary.
A received success payload with an inconsistent product, counter, or unsafe
integer is instead a protocol decoding failure.

An identity is active only while its node is on the recursion stack; sharing
the same identity in two completed sibling paths is not a cycle. Supported
value-semantic cell and struct graphs normally have no identity cycle, but the
guard is mandatory for adapters and future value kinds.

## Whole-top-level-element truncation

The negotiated budgets are:

```text
nodes     <= maxAggregateNodes
elements  <= maxAggregateElements
codeUnits <= maxPreviewCodeUnits
depth     <= maxAggregateDepth
```

Each individual string element also has at most
`maxStringElementCodeUnits`, and each exact node has at most
`maxPreviewElements` elements.

If a complete candidate is valid but committing it would exceed an aggregate
node, aggregate element, total code-unit, effective top-level element, or frame
budget, the producer discards that candidate's temporary counters and JSON,
returns the previously committed prefix, and reports that candidate and every
later selected element as omitted. Elements after this boundary need not be
visited. This can validly return an empty but truncated prefix when the first
complete candidate does not fit a cumulative budget.

Hard failures discovered while visiting a candidate discard every earlier
candidate and fail the complete inspect request. These include an individual
exact node over the per-node limit, an individual string over its per-string
limit, excessive depth, a cycle, an unsupported value, invalid internal value
metadata, or checked-arithmetic failure. Invalid internal canonical metadata is
an inherited `engine.invalidPreview` producer failure rather than a workspace
resource limitation. A nested payload is never shortened to turn any hard
failure into success.

The root struct schema is indivisible. If its complete field list cannot fit the
semantic code-unit or encoded-frame budget, inspect fails with
`workspace.previewLimit`; fields are never removed to obtain a preview.

## One-mebibyte encoded frame bound

Every complete JSON text frame is at most 1,048,576 bytes. The size is the
actual UTF-8 byte sequence sent for the entire envelope, with no BOM, including
all strings, escaping, structural punctuation, response metadata, preview,
truncation, and usage fields. Pretty-printed golden examples are JSON-semantic
examples and do not prescribe whitespace or object-member byte order.

A producer tentatively serializes each complete aggregate prefix in the final
response envelope. If adding a candidate would cross the frame bound, it rolls
back that whole candidate and uses the truncation rule above. If the indivisible
aggregate header itself crosses the bound, the producer returns the small
`workspace.previewLimit` error response instead. If even a mandatory error
envelope cannot fit, the transport terminates the connection as an over-size
protocol failure; it never emits an over-size frame.

An incoming frame over the hard bound is rejected by the transport. Integer
overflow or a failed encoded-size calculation is treated as exceeding the
bound, never as permission to wrap, allocate unchecked storage, or emit a
partial frame.

## Stable inspect failures

V2 freezes these categories:

| Category | Meaning |
| --- | --- |
| `workspace.previewLimit` | indivisible per-node, per-string, root-schema, arithmetic, semantic-counter, or encoded-frame limit prevents an exact result |
| `workspace.previewDepth` | a visited exact node would be deeper than the negotiated maximum |
| `workspace.cyclicValue` | an identity repeats on the active recursion path |
| `workspace.unsupportedValue` | object, function handle, VM marker, invalid field-name repertoire, or another unaccepted exact kind is encountered |

Error prose remains non-normative. Once the top-level truncation boundary has
been established, later unvisited values do not retroactively cause a hard
failure.

An implementation-internal serializer invariant failure is not relabeled as a
successful preview; it returns the inherited `engine.invalidPreview` category.
A peer receiving malformed `AggregatePreview` or `ExactValue` JSON treats it as
a decoding/protocol failure rather than one of the workspace categories above.

## Canonical positive goldens

### Nested cell with ordered shaped-empty struct

This full cell preview contains a numeric scalar followed by a fielded `0x3`
struct. The struct has no records but retains field order. Root elements count
as two; the scalar exact node contributes one element and the empty struct node
contributes zero.

```json
{
  "class": "cell",
  "dimensions": [1, 2],
  "complex": false,
  "selectedRange": {"start": [1, 1], "size": [1, 2]},
  "kind": "cell",
  "items": [
    {
      "class": "double",
      "size": [1, 1],
      "ndims": 2,
      "numel": 1,
      "complex": false,
      "kind": "numeric",
      "real": ["7"],
      "imag": ["0"]
    },
    {
      "class": "struct",
      "size": [0, 3],
      "ndims": 2,
      "numel": 0,
      "complex": false,
      "kind": "struct",
      "fields": ["beta", "alpha"],
      "records": []
    }
  ],
  "truncation": {"truncated": false, "omittedElements": 0},
  "usage": {"nodes": 3, "elements": 3, "codeUnits": 9, "depth": 1}
}
```

### Top-level shaped-empty struct

```json
{
  "class": "struct",
  "dimensions": [0, 3],
  "complex": false,
  "selectedRange": {"start": [1, 1], "size": [0, 3]},
  "kind": "struct",
  "fields": ["f"],
  "records": [],
  "truncation": {"truncated": false, "omittedElements": 0},
  "usage": {"nodes": 1, "elements": 0, "codeUnits": 1, "depth": 0}
}
```

### Whole-element truncation

With negotiated `maxAggregateElements: 2`, the first root element plus its
logical scalar exact node consume both element units. The second complete item
is omitted; no nested truncation marker is introduced.

```json
{
  "class": "cell",
  "dimensions": [1, 2],
  "complex": false,
  "selectedRange": {"start": [1, 1], "size": [1, 2]},
  "kind": "cell",
  "items": [
    {
      "class": "logical",
      "size": [1, 1],
      "ndims": 2,
      "numel": 1,
      "complex": false,
      "kind": "logical",
      "logical": [true]
    }
  ],
  "truncation": {"truncated": true, "omittedElements": 1},
  "usage": {"nodes": 2, "elements": 2, "codeUnits": 0, "depth": 1}
}
```

## Canonical negative goldens

This struct exact value is invalid because the record key set omits `alpha`:

```json
{
  "class": "struct",
  "size": [1, 1],
  "ndims": 2,
  "numel": 1,
  "complex": false,
  "kind": "struct",
  "fields": ["beta", "alpha"],
  "records": [
    {
      "beta": {
        "class": "logical",
        "size": [1, 1],
        "ndims": 2,
        "numel": 1,
        "complex": false,
        "kind": "logical",
        "logical": [true]
      }
    }
  ]
}
```

This exact cell is invalid because nested truncation is forbidden and
`items.length` is not `numel`:

```json
{
  "class": "cell",
  "size": [1, 2],
  "ndims": 2,
  "numel": 2,
  "complex": false,
  "kind": "cell",
  "items": [],
  "truncation": {"truncated": true, "omittedElements": 2}
}
```

This field schema is invalid because names are unique:

```json
{
  "class": "struct",
  "size": [0, 0],
  "ndims": 2,
  "numel": 0,
  "complex": false,
  "kind": "struct",
  "fields": ["valid", "valid"],
  "records": []
}
```

This otherwise structurally shaped schema is unsupported by the first tranche
because the field name is outside the ASCII identifier subset. A producer
returns `workspace.unsupportedValue`; a success decoder rejects it:

```json
{
  "class": "struct",
  "size": [0, 0],
  "ndims": 2,
  "numel": 0,
  "complex": false,
  "kind": "struct",
  "fields": ["\u03b1"],
  "records": []
}
```

This object-shaped value may pass the pre-existing schema-v2 structural branch,
but it is not a v2 `ExactValue`. A producer encountering it returns
`workspace.unsupportedValue`; a decoder receiving it as success rejects the
payload:

```json
{
  "class": "ExampleHandle",
  "size": [1, 1],
  "ndims": 2,
  "numel": 1,
  "complex": false,
  "kind": "object",
  "fields": [],
  "records": [{}]
}
```

## Decoder and verifier invariants

A conforming implementation has focused positive and negative tests for at
least these checks:

1. Bootstrap envelopes remain v0, supported identifiers are unique and ordered,
   capabilities are complete and valid, and the first post-bootstrap envelope
   uses exactly the negotiated identifier.
2. V0/v1 captured payloads and goldens decode with their original meaning; an
   aggregate success payload under v0 or v1 is rejected.
3. Structural integers are safe, ranks agree, range starts are one-based,
   selected extents are in bounds, and every shape product is checked.
4. The preview variant agrees with `class` and `kind`; cell and struct have
   exactly their canonical property sets and `complex: false`.
5. `ExactValue` class, kind, complex marker, payload property, payload length,
   integer grammar/range, string missing bits, and code-unit ranges agree.
6. Every exact cell has one complete item per `numel`. Every exact struct has
   unique ordered ASCII fields, one record per `numel`, and exactly the field
   set in each record. JSON member order is ignored.
7. Top-level values, cell items, and struct records traverse column-major;
   struct fields traverse in `fields` order.
8. Root depth and all semantic counters are recomputed in the specified order;
   reported `usage`, negotiated limits, per-node limits, and per-string limits
   all agree.
9. Only a top-level prefix may be omitted. Returned plus omitted equals the
   checked selected count, the Boolean truncation flag is canonical, and no
   nested node contains truncation or usage metadata.
10. Tentative candidate failure rolls back all counters and JSON. A hard failure
    discards every partial success and returns one structured error.
11. Active-path identity detection distinguishes a cycle from harmless sibling
    sharing and executes before recursion can exhaust the host stack.
12. The actual complete UTF-8 frame is within 1,048,576 bytes. Boundary tests
    cover exactly-at-limit success, one-byte-over whole-element rollback,
    indivisible-header failure, incoming over-size rejection, and checked-size
    overflow.

## V1 and v0 downgrade behavior

V1 and v0 messages, capabilities, captures, and golden payloads do not change.
If initialization selects v1 or v0, exact inspect of a cell or struct returns
`workspace.unsupportedValue`; it never returns an `AggregatePreview`, v0 text,
or a relabeled matrix/object placeholder. The existing v0/v1 downgrade rules
for char, string, integer, floating, logical, and all other values remain exact.

Display events remain non-normative and may perform an explicitly lossy human
rendering. Inspect, workspace change detection, CLI observation, conformance
serialization, and comparison must not parse display text back into value truth.

## Conformance mapping

When a v2 inspect backs a schema-v2 conformance producer, protocol failures map
to runner synthetic outcomes as follows:

| Protocol category | Synthetic category | Summary status |
| --- | --- | --- |
| `workspace.unsupportedValue` | `unsupported-payload` | `unsupported` |
| `workspace.previewLimit` | `payload-limit` | `unsupported` |
| `workspace.previewDepth` | `payload-limit` | `unsupported` |
| `workspace.cyclicValue` | `cyclic-payload` | `unsupported` |
| `engine.invalidPreview` | `invalid-observation` | `internal` |

Malformed successful payloads, mismatched usage, record sets, or serializer
invariants map to `invalid-observation` and summary status `internal`. These
synthetic outcomes are not MATLAB language errors and are never committed as
successful reference values.
