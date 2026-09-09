# MATLAB R2022b `global` / `persistent` conformance boundary

Status: clean-room design and conformance note. This is not an accepted RFC and
does not override `spec/` or `docs/rfcs/`.

This note joins the existing frontend observations with normalized MATLAB
R2022b value observations for executable storage behavior. The initial corpus
capture used parent `pre-publication baseline`; compiler and
runtime storage are now integrated on the main development branch.

## Evidence and observation boundary

All programs are OpenMat-authored. They were executed as black-box probes by
the repository oracle using the installed
`$env:MATLAB_EXE`. Only normalized class, shape,
numeric payload, and OpenMat-owned outcome data were retained. No MATLAB source,
tests, documentation, messages, or diagnostic prose was copied.

The seven storage cases use conformance schema version 1 because their complete
observable results are doubles. The current schema observes only
`openmat_result` after success or an OpenMat-owned error category after failure.
It has no warning field and no independent workspace/persistent-store snapshot.
Consequently:

- a storage effect is asserted only when a later expression copies it into
  `openmat_result`;
- the warning emitted by the sampled declaration-after-assignment program is
  intentionally not asserted;
- internal allocation time, storage identity, and unobserved bindings are not
  inferred from a successful result.

Focused capture command:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -Tag global-persistent `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

On 2026-08-25 this selected seven cases and MATLAB R2022b matched all seven
manifest expectations.

Against the stated parent, the focused OpenMat differential runner selected the
same seven cases and reported `0` passed, `7` failed, `0` unsupported, and `0`
internal (exit code `2`). Every actual observation was an OpenMat
`syntax-error`, so both the manifest and reference comparisons differed on
outcome. This is the precise compiler-integration gap at that snapshot, not a
conformance pass.

After compiler/runtime integration, the same focused runner was repeated on
2026-08-25 and reported `7` passed, `0` failed, `0` unsupported, and `0`
internal. That later run closes the executable source-language boundary for
the seven cases without erasing the earlier baseline result.

## Normalized R2022b observations

| Case | Normalized result | Boundary established |
| --- | --- | --- |
| `global_missing_empty` | `double`, size `0x0`, no elements | Declaring the sampled absent global makes its readable value the canonical empty double. |
| `global_preserves_existing` | `23` | A declaration following an existing script value does not replace that value. MATLAB also emitted a warning, which is outside the schema. |
| `global_script_function_share` | `[5, 11, 11]` | A script and a same-file local function that both declare the name share the value; the function write is visible when control returns to the script. |
| `persistent_counter_initialization` | `[1, 10, 0, 11, 0, 12]` | The first reached declaration reads empty, conditional initialization stores `10`, and later calls read and increment the stored value. |
| `persistent_function_isolation` | `[1, 100, 2, 110]` | Two different local functions using the same source name `state` retain independent values. |
| `persistent_conditional_declaration` | `[-1, 0, 8, 1, -1, 0, 9, 0]` | Calls that skip the declaration branch return normally; the first call that enters it observes empty, and the second entered call observes retained state. |
| `persistent_array_repeated_update` | `[2, 4, 6, 2, 7, 6, 2, 7, 6, 2, 5, 6]` | A persistent array can be read, updated by indexed assignment, and read again on a later call. |

The conditional case does not prove whether an implementation allocated an
unobservable slot during the skipped call. It proves the language-visible
boundary available to this runner: skipping the statement has no visible
effect, and the first actual entry still observes an empty persistent value.

The array case proves ordinary repeated value reads and writes. It does not
observe alias identity, detach timing, or copy-on-write implementation details.

## Corpus isolation

Every case is self-contained and must pass independently of selection order.
The programs therefore use case-unique global and local-function names and do
not rely on state established by another manifest.

The two runners implement that rule differently:

- the OpenMat runner starts a fresh CLI process for every manifest;
- the MATLAB oracle may execute a selected batch in one MATLAB process, but it
  calls each script from a fresh `execute_case` function workspace.

The unique names prevent MATLAB process-global or function-persistent state
from coupling otherwise independent cases. These cases establish repeated
calls only within one program execution; they make no cross-case or
cross-session lifetime claim.

## Surface syntax and scope facts retained from frontend probes

Earlier OpenMat-authored R2022b probes established these syntax boundaries:

- `global` and `persistent` accept whitespace-separated identifiers; a comma,
  semicolon, or physical line break terminates the declaration, while an
  ellipsis continues it;
- declaration initializers, indexed items, field items, and missing names are
  rejected;
- `global` is admitted in scripts and functions; `persistent` is admitted in
  functions but rejected directly in a script;
- declarations are admitted inside conditional function bodies and methods but
  rejected directly in a `classdef` body;
- a sampled persistent name conflicting with an input, output, prior
  assignment, or global name was rejected;
- duplicate `global` declarations completed, whereas sampled duplicate
  persistent declarations were rejected;
- declaring a global after assignment completed and produced a warning.

The parser and HIR preserve declaration names and source positions rather than
rewriting declarations as assignments. Control-flow bodies inherit their
enclosing script/function context, and a nested or local function starts a new
function scope.

### Lossless CST and recovery

The frontend representation remains:

```text
GlobalStmt | PersistentStmt
|- NameExpr*                 valid recovered names, in source order
`- Error*                    invalid declaration items, in source order
```

The statement owns the exact token interval from its keyword through its final
item or delimiter. Whitespace, comments, ellipses, physical newlines, commas,
and semicolons remain in the lossless token stream. In particular, continuation
text is not normalized away.

Comma and semicolon are statement terminators rather than name-list separators.
Invalid adjacent syntax such as `name.field` is retained as an `Error` item;
recovery resumes at a later whitespace-separated item or statement boundary.
The OpenMat-owned parser diagnostics are:

| Code | Condition | Recovery result |
| --- | --- | --- |
| `OMP0007` | no declaration name | retain an empty declaration node |
| `OMP0008` | non-identifier declaration item | retain an `Error` child and continue |
| `OMP0009` | repeated name in one `persistent` declaration | retain both name spans |
| `OMP0053` | declaration directly in a class body | retain the declaration under class recovery |

The parser does not decide cross-statement first-use, input/output conflicts,
or runtime storage.

### HIR and editor representation

Declarations are not assignments. HIR retains:

```text
StmtKind::Declaration(DeclarationStatement)

DeclarationStatement {
    kind: DeclarationKind,       // Global | Persistent
    names: Vec<Name>,            // source-ordered text and spans
    form: DeclarationForm,
    context: DeclarationContext, // Script | Function | ClassDefinition
}

DeclarationForm =
    IdentifierList
  | MissingNames
  | InvalidItems
  | DuplicatePersistentNames
```

`OMH0004` rejects a persistent declaration outside a function and either
declaration directly in a class definition. Malformed declarations keep their
recovered names and form; item syntax remains a parser concern and lexical
context remains an HIR concern.

The LSP traverses declarations through nested control flow and functions. It
classifies keywords and recovered variable names, creates navigable bindings
only for well-formed context-valid declarations, and retains an internal
binding kind that distinguishes ordinary, global, persistent, and conflicting
names. Definition, references, rename, hover, completion, semantic tokens, and
diagnostic ranges therefore do not need executable lowering to reinterpret the
declaration as an assignment.

## Compiler and runtime model required by the observations

Name binding and storage selection are related but distinct:

```text
BindingStorage =
    Local(LocalSlot)
  | WorkspaceGlobal(ConstantId)
  | Persistent(PersistentSlot)
```

For `global`, a valid declaration classifies the name as workspace-global for
reads and writes in that scope. Executing the declaration must create a missing
binding as an empty double without overwriting a value already present. A
same-named declaration in another scope selects the same session workspace
entry. Every name-rooted path, including ordinary assignment and aggregate
write-back, must honor that classification.

For `persistent`, the compiler assigns a function-owned slot rather than a
frame local or workspace global. Executing the declaration creates the empty
value only when the function identity and slot have no stored value; later
calls preserve it. The storage key must distinguish the two local functions in
`persistent_function_isolation` even though both source variables are named
`state`.

Declarations in control flow retain an instruction at their source position.
The compiler must not hoist a `DeclarePersistent` out of the sampled branch or
convert it into a per-call initialization. Binding analysis can remain lexical
while the declaration's language-visible initialization effect remains
source-ordered.

Persistent arrays use normal language value operations. Loading an array for
indexed update and storing the result must not introduce a scalar-only storage
special case. The current corpus deliberately checks values rather than a Rust
sharing strategy.

The accepted stable boundary is the versioned bytecode instruction/slot model,
not Rust enum layout. No protocol or conformance-schema change is required for
these seven cases because they expose their state through an ordinary bounded
numeric result.

## Same-file local-function boundary

Five cases use local functions at the end of a script. The intended boundary is
narrow:

- the entry is executed in file mode;
- local functions are resolved from that same source unit;
- repeated calls to one local function use one persistent identity;
- two distinct local functions use distinct persistent identities;
- a local function and its calling script share a global only when both declare
  that global name.

The corpus does not generalize this to nested closures, anonymous functions,
methods, separate function files, path shadowing, reloads, or edits. Those need
separate probes and identity decisions.

## Deliberately uncovered lifetime and reset behavior

This tranche does not cover:

- `clear name`, `clear global`, `clear functions`, `clear <function>`,
  `clear all`, or any persistent reset caused by them;
- interpreter reset, kernel shutdown/restart, multiple kernels, or multiple
  MATLAB processes;
- global sharing across separate script/function files, linked modules,
  nested functions, methods, or class loading;
- persistent identity under recursion, nested functions, closures, methods,
  module replacement, path changes, or source reload;
- declaration conflicts beyond the retained frontend probes;
- unwind, cancellation, or partially completed writes;
- copy-on-write alias isolation, value/handle objects, cells, structs, strings,
  integer arrays, or complex arrays in persistent storage;
- warning identity, warning ordering, warning state, or warning text.

These are compatibility unknowns, not permission to choose incidental host
behavior. They require new OpenMat-authored probes and, where observation needs
warning or hidden-store data, an accepted schema/runner extension before a
conformance claim can be made.
