# RFC 0003: Lossless char, string, and integer wire contract

Status: Accepted

## Decision

OpenMat adds `openmat-kernel-v1` and conformance schema version 2 as additive,
versioned contracts. The frozen `openmat-kernel-v0` protocol, conformance case
schema version 1, observation schema version 1, their references, and their
comparators remain readable and verifiable indefinitely. No v0 or schema-v1
payload changes meaning.

The first v1 value tranche transports:

- `char` as exact UTF-16 code units;
- each `string` element as exact UTF-16 code units plus an independent missing
  bit;
- `int8`, `uint8`, `int16`, `uint16`, `int32`, `uint32`, `int64`, and `uint64`
  with either real or complex fixed-width storage.

Complex integer storage is in scope. Complex integer arithmetic is not part of
this wire contract and is not implied by the ability to inspect or compare a
complex integer value.

Rust enum layout, native endianness, and serde's incidental representation are
not an ABI. The stable representation is the JSON shape in
`spec/protocol/kernel-v1.md` and the new schema files.

## Exact value representation

`char` elements are unsigned 16-bit code units in `0..65535`. A surrogate pair
occupies two array elements, and an isolated surrogate remains a valid exact
value. No protocol, oracle, CLI, or comparator path may pass char truth through
UTF-8 or Unicode-scalar validation.

A string array contains one code-unit sequence and one missing bit per array
element. Missing is not inferred from an empty sequence. The canonical missing
element has `missing: true` and an empty code-unit sequence; an empty non-missing
string has the same empty sequence and `missing: false`. Lossy replacement is
permitted only in a display layer and never becomes value truth.

Every integer element has `real` and `imaginary` components. Components are JSON
strings matching `0|-?[1-9][0-9]*`; unsigned classes additionally prohibit a
minus sign, and both components must fit the declared class. A real integer
array sets `complex: false` and every `imaginary` component is exactly `"0"`.
A complex integer array sets `complex: true`; at least one imaginary component
is nonzero. JSON numbers and conversion through `f64` are prohibited.

For existing floating-point kinds, a `complex: true` single or double preview
uses `kind: "complex"` for every element and both JSON-number components must be
finite. The one-spelling `special` kind is valid only for a non-complex real
element. If either component of any selected complex floating-point element is
NaN or infinity, v1 inspect fails with `workspace.unsupportedValue` until a
future protocol defines an exact two-component special representation.

All value and preview payloads carry an exact class, canonical shape, and an
explicit `complex` marker. Array payloads are flat in column-major element
order. Shapes have at least two dimensions. `ndims` equals `size.length`, and
`numel` equals the checked product of `size`; empty dimensions are preserved.
For conformance values, each element payload has exactly `numel` entries. For a
kernel preview, `values.length + truncation.omittedElements` equals the checked
product of `selectedRange.size`.

These cross-field and checked-product rules are semantic validation rules in
addition to JSON Schema validation. Draft 2020-12 cannot compare an array's
length to a sibling numeric property or compute a product.

## Bounded previews

Version 1 keeps the v0 maximum of 4,096 preview elements and adds two negotiated
hard ceilings:

- at most 16,384 UTF-16 code units in one string element;
- at most 65,536 UTF-16 code units across all returned string elements in one
  preview.

The per-element ceiling bounds the single-element bypass of an element-count
limit. It represents 32 KiB of raw UTF-16. The aggregate ceiling represents
128 KiB of raw UTF-16 and, even with worst-case five-digit JSON integers and
separators, remains comfortably below the existing one-mebibyte transport frame
budget. The aggregate limit is four times the per-element limit so useful
multi-element previews remain possible without making one element unbounded.

The negotiated limits may be lower but must be positive, and the total limit
must not be lower than the per-element limit. Missing elements consume zero code
units. While building the bounded column-major prefix, a string element reached
before another budget establishes the truncation boundary and exceeding the
per-element limit makes the inspect request fail with
`workspace.unsupportedValue`; it is never split or replaced. If complete
elements individually fit but the aggregate limit or effective element-count
limit is reached, the kernel returns the longest whole-element prefix that fits
and reports the remaining element count through the existing truncation fields.
The effective element-count limit is the smaller of the request's `maxElements`
and the negotiated limit. Elements after that boundary need not be visited.
`char` already consumes one code unit per array element and is bounded by the
same element limit.

## Version negotiation and downgrade

Initialize is a bootstrap exchange. A v1-capable client sends the initialize
request in an `openmat-kernel-v0` envelope and lists
`["openmat-kernel-v1", "openmat-kernel-v0"]` in preference order. New
v1-specific capability fields are safe in that request because v0 ignores
unknown fields. The initialize response is also a v0 envelope and names the
selected protocol in `negotiatedProtocol`. Starting with the next envelope,
both peers use the selected protocol. Thus an old v0 kernel can select v0, while
a new kernel selects the first client-offered version it supports.

A client that offers v1 must provide valid v1 code-unit capabilities. A
malformed offer fails initialization rather than silently changing bounds. No
common version fails with `protocol.noCommonVersion`. Protocol changes are
forbidden after successful initialization, and a mismatched or unknown envelope
version is a protocol failure rather than a request to guess or downgrade.

On v0 downgrade, char and integer inspection fails with
`workspace.unsupportedValue`. A string may use the old `text` value only when
every element is non-missing and exactly representable as Unicode scalar text;
otherwise it also fails. A peer must never invent replacement characters,
coerce an integer to a JSON number, or relabel a value to make v0 accept it.

## Conformance schema version 2

`case-v2.schema.json` and `observation-v2.schema.json` preserve the existing
normalized value kinds and replace the lossy boundaries as follows:

- every value requires `complex` in addition to class and shape metadata;
- char uses flat `code_units`, each in `0..65535`;
- string uses parallel `string_code_units` and `missing` arrays;
- integer uses an `integer` array of `{real, imaginary}` decimal-component
  objects.

The parallel string arrays and every flat element array have exactly `numel`
entries in column-major order. A missing string's parallel code-unit entry is
empty. Integer component range and signedness are checked against `class`, and
the complex marker obeys the same canonical rule as the kernel protocol.

A manifest with `schema_version: 1` continues to use the old case schema,
serializer, observation schema, and comparator. A manifest with
`schema_version: 2` atomically uses all four v2 components and requires kernel
v1 for OpenMat observations. Mixing a v1 manifest, observation, reference, or
comparator with a v2 component is an error. Any unknown schema version or value
`kind` fails before comparison; implementations do not fall through to the
latest known version.

The v2 schemas enforce structural limits of 4,096 elements and 16,384 code units
per string element. The runner additionally checks the 65,536 aggregate
code-unit limit and all computed count, shape, signedness, and integer-range
invariants. Reference observations are updated only by an explicit oracle
command and are never rewritten as a side effect of comparison.

A canonical v2 manifest names version 2 at the root; the expected value uses
the same v2 shape as an observation:

```json
{
  "schema_version": 2,
  "id": "wire_string_exact",
  "description": "String code units preserve an isolated surrogate and missing.",
  "source": "programs/wire_string_exact.m",
  "tags": ["r2022b", "string"],
  "expected": {
    "outcome": "ok",
    "value": {
      "class": "string",
      "size": [1, 2],
      "ndims": 2,
      "numel": 2,
      "complex": false,
      "kind": "string",
      "string_code_units": [[55357], []],
      "missing": [false, true]
    }
  }
}
```

## Migration and rollback

Implementation proceeds additively: land the contracts, add v1 codecs and
bootstrap negotiation while retaining v0 golden tests, add v2
serializer/validator/comparator routing while retaining v1 routing, and only
then add v2 cases and references.

During migration, schema-v1 cases may continue over kernel v0 when their values
are losslessly representable by v0. V2 cases require kernel v1. A v2 value is never
automatically rewritten into a v1 reference because isolated surrogates,
missing strings, and wide integer components may not survive that conversion.
The v2 conformance schema can describe non-finite complex components as
independent normalized strings, but the v1 kernel inspect path cannot yet emit
such an OpenMat observation and reports it as unsupported.

Rollback disables v2 case selection and makes clients offer only v0; it does
not delete or mutate v1 protocol captures, v2 manifests, or references. Old v0
and schema-v1 artifacts remain usable throughout rollout and rollback.

## Golden normalized values

A complex integer value is represented without JSON-number conversion:

```json
{
  "class": "int64",
  "size": [1, 2],
  "ndims": 2,
  "numel": 2,
  "complex": true,
  "kind": "integer",
  "integer": [
    {"real": "-9223372036854775808", "imaginary": "0"},
    {"real": "7", "imaginary": "-9"}
  ]
}
```

An isolated surrogate, an empty string, and missing remain distinct:

```json
{
  "class": "string",
  "size": [1, 3],
  "ndims": 2,
  "numel": 3,
  "complex": false,
  "kind": "string",
  "string_code_units": [[55357], [], []],
  "missing": [false, false, true]
}
```

## Consequences

Protocol, kernel, CLI, web, oracle, and comparator implementations must validate
the same canonical rules instead of relying on serde success alone. Unsupported
or over-limit values produce structured failure or whole-element truncation;
lossy display conversion remains explicitly outside the contract.
