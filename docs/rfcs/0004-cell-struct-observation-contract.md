# RFC 0004: Cell and struct value and observation contract

Status: Accepted

## Decision

OpenMat accepts cell arrays and structures as the third Milestone 6 value
tranche. The tranche is complete only when phases C1 through C3 below and the
cumulative Milestone 6 gates all pass. An implementation of construction or
scalar access alone must not be described as the completed cell/struct tranche.

This RFC accepts four coordinated boundaries:

1. Cell and struct values use one canonical shaped-array model. Struct storage
   is field-major and there is no scalar representation special case.
2. Comma-separated lists use VM-internal typed pack registers. A `ValuePack` is
   not a language `Value`, and aggregate write-back is transactional through
   `AssignPlace`. These incompatible bytecode changes establish bytecode
   version `12.0`.
3. Exact recursive cell/struct inspection is introduced only by
   `openmat-kernel-v2`. The frozen v0 and v1 wire contracts do not gain an
   aggregate preview kind.
4. Conformance schema version 2 already expresses recursive values. Its
   producer and semantic-validator policy gains the traversal limits and
   deterministic counting rules in this RFC; no schema version 3 is created.

`spec/protocol/kernel-v2.md` is the normative wire contract. This RFC is the
normative language, VM, rollout, and conformance-policy decision. The earlier
`docs/design/cell-struct-value-model.md` remains informative and does not
override either accepted document.

## Canonical aggregate value model

Cell arrays and struct arrays use the existing canonical `Shape` rules:

- shapes have at least two dimensions and preserve empty dimensions;
- `numel` is the checked product of the dimensions;
- linear storage and traversal are column-major;
- the language boundary is one-based while internal offsets are checked and
  zero-based;
- clones share copy-on-write storage until a successful mutation detaches it;
- an index result is a materialized contiguous aggregate in the first tranche.

A cell element is one complete `Value`. There is no special scalar-cell variant.
Consequently `{}` is a `0x0` cell, `{[]}` is a `1x1` cell containing one `0x0`
double, and `cell(0, 3)` preserves its `0x3` shape.

Every element of a struct array shares one ordered field schema. Storage is
field-major: for each field, one column contains a complete `Value` for every
struct element in struct column-major order. Every field column has exactly
`numel` entries. Field lookup metadata never defines observable order; the
schema's ordered name list does. Rust layout, enum layout, and storage sharing
are not an ABI.

There is no scalar-struct variant. `struct()` is an unfielded `1x1` struct with
one empty record, `struct([])` is an unfielded `0x0` struct, and a shaped empty
struct retains both its exact shape and its complete ordered field schema. A
zero `numel` never discards field names.

Adding a field appends it to the shared schema and fills every otherwise
unassigned record with a real `0x0` double value. Duplicate fields, a field
column of the wrong length, a field count/column count mismatch, or an invalid
shape or field name are construction failures. Failed validation must not
detach storage or publish a partial value.

Language assignment uses the runtime's common `language_copy` boundary:
numeric, character, string, cell, and struct values use copy-on-write value
semantics; value-class objects acquire assignment-copy isolation; handle-class
objects retain identity aliasing. Aggregate insertion, extraction across an
assignment/argument/return boundary, and chained value write-back all use that
policy. A read-only preview or switch candidate iteration does not manufacture
an assignment copy.

## Accepted field-name boundary

The first tranche accepts only the ASCII MATLAB identifier subset matching:

```text
^[A-Za-z][A-Za-z0-9_]*$
```

The check is on the exact field name after a scalar char row or non-missing
scalar string is obtained. Empty names, missing strings, duplicates, and all
names outside this subset are structured field-name errors. Static and dynamic
field names obey the same rule, and a dynamic name expression is evaluated
once. Full Unicode identifier compatibility is deferred and must not be
inferred from host-language identifier libraries.

## Aggregate tranche semantics

### Phase C1: construction and scalar access/write

Cell literal elements are evaluated in source order. Rows form a two-dimensional
shape and must have equal width after explicit pack expansion. Ordinary values
become single cell items without numeric concatenation. Packs splice their
values in pack order. Every inserted value passes through `language_copy`.

The initial `cell(dims...)` built-in accepts checked non-negative real integer
scalar dimensions and fills every item with a real `0x0` double. Other overloads
are structured unsupported behavior until separately accepted.

`C(indices)` returns a cell array even for a scalar selection. `C{indices}`
returns a value pack; a scalar brace selection therefore returns a one-value
pack. Brace access on a non-cell is an aggregate type error and never enters
call resolution. The target and each index expression are evaluated once.

`C(k) = rhs` requires a cell RHS, except for the separately defined deletion
token. A scalar cell RHS may expand where assignment permits it. `C{k} = value`
stores cell content; assigning `[]` through braces stores a real `0x0` double
and does not delete the cell element. Scalar growth fills gaps with real `0x0`
double values. Validation, allocation reservation, and language copies all
complete before the new root is published.

`struct` remains a shadowable callable name represented by unresolved
call-versus-index application. The first constructor tranche accepts:

- `struct()` and `struct([])` with the shapes defined above;
- ordered name/value pairs with scalar char-row or non-missing scalar-string
  names;
- non-cell field inputs replicated as complete values across output records;
- cell field inputs supplying per-record contents, with equal shapes for
  non-scalar cell inputs and scalar-cell expansion.

A shaped empty cell input determines the corresponding shaped empty struct.
Duplicate fields fail. Convenience conversions and reflection functions such
as object-to-struct conversion, `cell2struct`, `struct2cell`, `rmfield`, and
`orderfields` are not accepted by this tranche.

A scalar struct field read returns a one-value pack. A non-scalar read returns
one value per record in record column-major order; a shaped empty read returns
a zero-value pack. A missing field is an error, not an empty value. A new field
written to one record is appended to the schema and every other record receives
a real `0x0` double for that field.

### Phase C2: general indexing, colon, and `end`

Cell paren indexing, cell brace indexing, and struct paren indexing share one
resolved index-plan implementation with numeric integer indices, logical masks,
ranges, standalone colon, collapsed final subscripts, and the existing shape
rules. Selected offsets and results are ordered column-major.

`C(:)` is a column cell vector. `C{:}` is a pack of all contents in cell
column-major order. `end` is an index-context operation, not a workspace value:
with one subscript it is `numel(target)`; with multiple subscripts it uses the
effective extent for the current argument, including final-subscript collapse.
Nested uses bind to their immediate indexed target. If unresolved paren apply
resolves to a callable, an index-bound colon or `end` is an invalid call
argument rather than a global name lookup.

Struct paren indexing preserves the exact schema order. A field read traverses
records column-major. Static and dynamic struct fields have identical value
semantics after name evaluation; class-object access keeps its separate access
and dispatch rules.

### Phase C3: comma lists, assignment, deletion, and growth

Brace selection and non-scalar struct field selection first produce an ordered
pack. Pack consumption is fixed as follows:

- one output consumes the first value and ignores trailing values;
- `N` outputs consume the first `N` values and ignore trailing values;
- fewer than `N` values, including a zero-length pack for a positive output
  count, is an arity error with no partial assignment;
- zero requested outputs consumes no values and is not an arity error, while
  producer, target, and index evaluation side effects still occur;
- a call/apply argument, cell literal, or other explicit splice consumer expands
  the complete pack;
- a pack cannot be stored in a workspace, cell slot, struct field, ordinary
  register, protocol payload, or conformance value.

Simple multi-element content assignment such as `C{1:2} = value` is not scalar
expansion. Multi-place content updates use an explicit bracketed comma-list
place. The runtime materializes and validates the RHS values and every target
place before committing any update.

Paren cell assignment accepts a cell RHS of matching cardinality and accepts a
scalar cell expanded over the selection. A non-cell RHS is a type error except
for `[]` as a deletion token. `C(selection) = []` deletes cell elements;
`C{scalar} = []` stores empty-double content. Multi-dimensional deletion is
accepted only for whole-row, whole-column, or valid one-dimensional linear
deletion shapes established by R2022b cases. An invalid hole deletion is a
dimension error and never silently flattens the value.

Struct paren selection returns the same ordered schema. Struct assignment
accepts an RHS with the same field set; fields are aligned by name and the
destination order remains unchanged. A scalar struct may expand across a
selection. A different field set is a schema error. `S(selection) = []` deletes
records and preserves the schema. Scalar growth fills gap records and all their
fields with real `0x0` doubles.

A simple field assignment to a non-scalar struct is not scalar expansion;
multi-record field updates use a bracketed comma-list target. Successful writes
detach only the necessary aggregate path and field columns. Failed type, shape,
field, arity, allocation, or language-copy checks leave the original root and
all observable aliases unchanged.

## VM and bytecode 12.0 boundary

Bytecode version `12.0` adds a distinct `PackRegister` index space and a checked
`pack_register_count` to each function/frame. `ValuePack` contains an ordered
sequence of `Value` instances but is VM-internal and is not a `Value` variant.
Only explicitly pack-aware instructions may name a pack register or request
expansion.

The minimum accepted instruction boundary includes the equivalent of:

- `BuildCell` with operands that are either one value or an expanded pack;
- `BraceApply` producing a pack;
- aggregate-field read producing a pack;
- `Unpack` with explicit value-register outputs;
- pack-aware apply arguments;
- index-bound `ResolveEnd`;
- one `AssignPlace` containing a name-rooted path of paren, brace, static-field,
  and dynamic-field steps, an assignment mode, and a value or expanded-pack
  source.

Exact Rust type and variant spelling is crate-internal. The semantic verifier
invariants are not:

1. value and pack register indices are checked against their distinct bounds;
2. ordinary arithmetic, conditions, indices, returns, and workspace stores
   cannot reference a pack register;
3. only pack-aware operands can expand a pack, and an unexpanded pack cannot
   escape into `Value` storage;
4. every field operand, apply argument, place step, constant, and register is
   validated before execution;
5. an assignment place is rooted in an assignable local/name boundary in the
   first tranche; temporary-expression lvalues are rejected;
6. `AssignPlace` validates the complete path and RHS, constructs an updated root
   off to the side, and performs at most one root store after success;
7. bytecode 11 and 12 are never guessed or mixed. A version-12 program is
   rejected by a version-11 runtime and vice versa under the existing exact
   bytecode-version rule.

Existing class-object field instructions retain their existing meaning unless
a later accepted bytecode contract explicitly unifies them. `ParenApply`
continues to preserve call-versus-index ambiguity through runtime resolution;
the compiler must not special-case the spelling `cell` or `struct` as a parser
or HIR literal.

## Kernel protocol and downgrade

Exact recursive observation uses the new identifier `openmat-kernel-v2` and the
bootstrap preference order:

```text
["openmat-kernel-v2", "openmat-kernel-v1", "openmat-kernel-v0"]
```

Initialization is still exchanged in v0 envelopes. For non-aggregate values,
v2 retains the v1 `MatrixPreview` with no field, kind, or semantic change and
adds `AggregatePreview` plus recursive `ExactValue` only to a negotiated v2
session. V0 and v1 decoders, captures, goldens, capability behavior, and
downgrade semantics remain unchanged.

Under a v1 or v0 selection, exact inspect of any cell or struct returns
`workspace.unsupportedValue`. It is never relabeled as text, an object, a
matrix, or a missing value. A display event may contain an explicitly lossy
human-readable representation, but display is non-normative and neither the
CLI nor conformance tooling may reconstruct value truth from it.

V2 freezes these hard ceilings; negotiation may select lower valid values:

- `maxPreviewElements <= 4096`;
- `maxAggregateNodes <= 16384`;
- `maxAggregateElements <= 65536`;
- `maxAggregateDepth <= 32`, with root depth zero;
- `maxStringElementCodeUnits <= 16384` UTF-16 code units;
- `maxPreviewCodeUnits <= 65536` UTF-16 code units over all returned char,
  string, and field-name value data;
- one complete UTF-8 JSON text frame, including its envelope, is at most
  1,048,576 bytes.

`spec/protocol/kernel-v2.md` freezes capability validation, canonical JSON,
counter definitions, traversal order, whole-top-level-element truncation,
overflow handling, encoded-size handling, and the stable categories
`workspace.previewLimit`, `workspace.previewDepth`, `workspace.cyclicValue`,
and `workspace.unsupportedValue`.

## Conformance schema-v2 producer and validator policy

Schema-v2 `cell.items` and `struct.fields`/`struct.records` already represent
the accepted recursive payload and field order. Producers and semantic
validators must not add schema-v3, must not reinterpret schema-v1, and must not
depend on JSON object-member order.

RFC 0003 routing remains sufficient for schema-v2 values representable by
kernel-v1. A schema-v2 OpenMat observation whose successful value is or may be a
cell or struct requires a negotiated kernel-v2 session. Selecting v1 or v0 for
such a case produces the synthetic unsupported outcome below; it does not
change the manifest version or fall back to display. This is an additive
transport requirement, not a schema payload change.

For a complete schema-v2 value, traversal is deterministic depth-first pre-order:

1. The root is depth zero. On entering a value, validate its canonical metadata,
   charge one node, then charge that node's complete `numel`.
2. A char node charges its code-unit array in order. A string node charges each
   element's code units in column-major order; missing strings charge zero.
3. A cell recurses through `items` in column-major order.
4. A struct charges each field name once in `fields` order, then traverses
   records in column-major order and values in `fields` order. Repeated JSON
   record keys are representation overhead and do not charge code units again.
5. A child value is one depth greater than its containing cell item or struct
   field. Records and field-schema entries are not nodes.

The semantic policy applies the hard maxima above to a complete conformance
tree. Every value node also has `numel <= 4096`, and each individual string
element has at most 16,384 UTF-16 units. Counts and shape products use checked
arithmetic. Exceeding a limit produces no partial successful observation.
The encoded observation is also checked as UTF-8 JSON without a BOM against
the one-mebibyte producer bound.

The accepted synthetic outcomes and summary mappings are:

| Cause | Synthetic category | Summary status |
| --- | --- | --- |
| unsupported nested kind, including object or function handle | `unsupported-payload` | `unsupported` |
| node, element, per-node, string, code-unit, depth, arithmetic, or encoded-size limit | `payload-limit` | `unsupported` |
| repeated active identity on the current recursion path | `cyclic-payload` | `unsupported` |
| malformed canonical metadata, payload count, record field set, or serializer invariant | `invalid-observation` | `internal` |

These are producer/runner outcomes, not MATLAB language errors and not valid
successful reference truth. Reference updates remain explicit oracle actions.
An existing schema-v2 `kind: "object"` branch does not authorize object exact
observation in this tranche: producer policy reports it as
`unsupported-payload`, and function handles are treated the same way.

The protocol-to-conformance mapping is fixed:

| Kernel category | Conformance synthetic category | Summary status |
| --- | --- | --- |
| `workspace.unsupportedValue` | `unsupported-payload` | `unsupported` |
| `workspace.previewLimit` | `payload-limit` | `unsupported` |
| `workspace.previewDepth` | `payload-limit` | `unsupported` |
| `workspace.cyclicValue` | `cyclic-payload` | `unsupported` |
| `engine.invalidPreview` | `invalid-observation` | `internal` |

## Positive and negative acceptance cases

At minimum, implementation tests cover these positive invariants:

- `{}`, `{[]}`, `cell(0,3)`, `struct()`, `struct([])`, and a fielded `0x3`
  struct preserve distinct shapes and payload counts;
- `{11,22;33,44}` stores and expands as `11,33,22,44`;
- scalar and multi-output pack consumption, too-few-output atomic failure, and
  full argument/cell-literal expansion follow the accepted rules;
- cell/struct scalar expansion, deletion, growth, gap fill, colon, logical and
  range indexing, and `end` use one shape resolver;
- field insertion order survives read, indexing, assignment by equal field set
  in a different order, deletion, growth, exact observation, and comparison;
- nested value-class mutation is isolated while nested handle-class identity
  remains shared;
- v2/v1/v0 bootstrap, shaped-empty exact payloads, top-level whole-element
  truncation, and every exact traversal budget boundary have goldens;
- schema-v2 producer and validator counters agree for the same recursive tree.

Negative tests reject or report the specified structured outcome for duplicate
or non-ASCII field names, mismatched cell rows, invalid constructor overloads,
brace access on a non-cell, missing fields, pack escape, invalid pack/value
registers, too-few comma-list values, invalid deletion shapes, schema mismatch,
failed transactional write-back, nested per-node/per-string/depth overflow,
cycle, object/function exact observation, malformed record key sets, and any
aggregate preview sent under v0 or v1.

## Deferred work and consequences

Full Unicode field names, object/function-handle exact representation, shared
wire identity, cycle serialization, general cell/struct equality, arbitrary
temporary-expression lvalues, struct/object conversion, and convenience
reflection functions remain outside this tranche. Object and function-handle
copy behavior must be tested through supported observable summaries rather than
opaque recursive serialization.

Implementation proceeds additively: accept these contracts, implement value and
bytecode boundaries, implement runtime C1-C3 semantics, then add v2 protocol and
schema-v2 producer/validator routing while retaining all v0/v1 and schema-v1/v2
goldens. No implementation task may edit an older accepted wire or schema to
make an aggregate payload fit.
