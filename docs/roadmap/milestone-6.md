# Milestone 6: base-language breadth

Milestone 6 expands the executable base language without weakening the existing
clean-room or architectural boundaries. It is a sequence of independently
reviewable tranches, not a single claim of MATLAB compatibility.

## Entry baseline

The implementation entry point is
`pre-publication baseline`. On 2026-08-24 the OpenMat runner
executed all 44 checked-in conformance cases and reported 44 passed, 0 failed, 0
unsupported, and 0 internal; the classdef selection independently reported 15
of 15 passed.

The repository also contains 44 checked-in MATLAB R2022b black-box observations.
Those references establish the oracle side of each sampled behavior. The 44/44
OpenMat result is a separate implementation result produced by running OpenMat
and comparing its observations with the manifests and references.

Forty-four project-authored cases are a regression baseline, not a completeness
metric. They do not imply that OpenMat implements all of the MATLAB R2022b base
language, every form of a listed feature, or behaviors that the corpus does not
sample. Milestone 6 grows this corpus cumulatively.

## Dependency and delivery order

The order below is the integration order. Some parser work can be prepared in
parallel, but a later tranche is not accepted until all of its listed
dependencies and the cumulative gate are green.

| Order | Tranche | Depends on | Acceptance boundary |
| ---: | --- | --- | --- |
| 1 | Function handles | Existing function resolution, call metadata, unresolved call-versus-index representation, and runtime value dispatch | Named handles and anonymous functions parse losslessly, lower without prematurely resolving `f(x)`, invoke with multiple inputs/outputs, capture lexical values with defined value/handle behavior, and produce normalized errors for invalid targets or calls. File and local-function handles must obey the existing resolver precedence. |
| 2 | Character and common integer values | General runtime value, array shape, copy-on-write, indexing, comparison, and protocol-observation paths | Single-quoted character arrays preserve class, shape, character codes, concatenation, transpose, and indexing. The agreed common signed/unsigned integer classes preserve exact class and boundary values through construction, conversion, arithmetic selected by the compatibility contract, indexing, and observation serialization. Strings remain distinct from character arrays. |
| 3 | Cell arrays and structures | Character/value support, generalized aggregate storage, copy-on-write, indexing, and bounded observation serialization | Cell construction plus parenthesis/brace indexing and assignment have explicit shape and comma-separated-list behavior. Scalar and shaped structures support field read/write, dynamic field names, construction, indexing, and value-copy isolation. Nested heterogeneous values round-trip through the runner without lossy stringification. |
| 4 | `switch` | Character/string/integer comparison semantics and cell arrays for multi-value case clauses | The selector is evaluated once; the first matching case runs; cell-valued case lists, `otherwise`, nesting, empty/no-match behavior, and the absence of fallthrough match recorded R2022b observations. Parser recovery and compiler diagnostics cover malformed clauses. |
| 5 | `global` and `persistent` | Stable call-frame identity, lexical capture behavior from function handles, aggregate/value storage, and the existing explicit `clear` boundary | `global` sharing is explicit across scripts, functions, and the entry workspace. Each function owns correctly initialized persistent storage across calls. Shadowing, recursive/re-entrant calls, value/handle mutation, and the supported `clear` effects are oracle-backed; unsupported `clear` forms remain structured diagnostics rather than silently changing wider state. |
| 6 | `try`/`catch` | All earlier runtime value/control/scope paths and structured error propagation across bytecode frames | Runtime errors unwind to the nearest handler without corrupting frames, outputs, globals, or persistent state. Plain and bound `catch`, nesting, errors raised through function handles, and rethrow behavior selected by the compatibility contract are covered. Comparison uses OpenMat-owned error categories and observable catch results, never proprietary diagnostic prose. |

Function handles are first because they force callable values, name resolution,
lexical capture, and invocation through the same runtime boundary used by later
scope work. Character/integer values precede aggregates; aggregates then supply
heterogeneous storage and cell-valued `case` lists. `switch` follows the value
equality it consumes. Global/persistent state follows stable call identity and
capture semantics. Exception handling is last because its unwinding rules must
cross every earlier value, control-flow, call, and storage path.

## Per-tranche acceptance gate

A tranche is complete only when all of the following are true:

1. Any new public semantics are accepted in the appropriate contract before
   dependent implementation lands. A task that needs a shared protocol or ABI
   change stops at that boundary and requests coordinator approval.
2. The lossless syntax layer, HIR, compiler/bytecode, runtime, and applicable
   protocol paths have focused positive, negative, recovery, and boundary tests.
   The AST/HIR continues to preserve unresolved call-versus-index ambiguity.
3. OpenMat-owned conformance cases cover the observable behavior and normalized
   error categories. MATLAB references, when needed, are recorded deliberately
   from a licensed local R2022b installation under the clean-room rules; they
   are not inferred from Octave or invented from documentation.
4. The OpenMat runner passes every previously accepted case and every new case
   with zero failed, unsupported, or internal results. The current 44/44 is the
   floor at Milestone 6 entry, not a permanently sufficient case count.
5. Windows quality gates pass: `cargo fmt --all --check`, locked workspace
   tests, strict Clippy over all workspace targets and features including
   `clippy::pedantic`, both conformance PowerShell self-tests, and the complete
   OpenMat conformance run using a locked-built CLI.
6. The Web IDE passes a frozen-lockfile install from
   `apps/web/pnpm-lock.yaml`, followed by `pnpm typecheck`, `pnpm test`, and
   `pnpm build` on the supported Node/pnpm versions.
7. Compatibility documentation states the exact measured checkout and keeps
   MATLAB oracle evidence distinct from OpenMat execution evidence. Remaining
   unsupported syntax, semantics, and coverage gaps are listed explicitly.

CI never invokes MATLAB. It does not download reference observations, commit
generated files, or publish the temporary conformance results and Web build as
source artifacts.

## Native/Web integration gate

Changes that touch the kernel protocol, server lifecycle, or Web transport must
also pass this local Windows gate:

```powershell
pwsh -NoProfile -File tools/openmat-dev/tests/Test-OpenMatDev.ps1
pwsh -NoProfile -File tools/openmat-dev/Invoke-OpenMatDev.ps1 -Smoke `
    -ServerPath .\target\debug\openmat-server.exe
```

`Test-OpenMatDev.ps1` already includes a real .NET `ClientWebSocket` lifecycle
against the native server and needs no browser or MATLAB. It remains a local
gate at Milestone 6 entry: repeatable stability of its child-process timing has
not yet been established on GitHub-hosted Windows runners. It may move into CI
after repeated hosted runs demonstrate that it is stable without new
dependencies; it should not be forced into the required workflow merely from a
single successful local run.

## Milestone exit

Milestone 6 exits when all six tranches have passed their cumulative gates, the
roadmap's dependency order is reflected in focused commits, and the
compatibility documents enumerate both the expanded pass set and its remaining
boundaries. A green exit still means only that the accepted OpenMat-owned corpus
passes against its recorded R2022b observations.

## Non-goals

Milestone 6 does not claim or require:

- complete MATLAB R2022b base-language coverage or a fixed percentage of it;
- identical MATLAB diagnostic identifiers, messages, display text, floating
  bits across numerical providers, or execution performance;
- JIT compilation, MEX compatibility, Simulink, proprietary toolboxes, or a
  pure-browser WebAssembly VM;
- sparse, GPU, distributed, symbolic, table, timetable, or categorical arrays;
- full graphics, desktop-GUI compatibility, or deployment packaging;
- advanced classdef features excluded by `spec/language/classdef-core.md`;
- every standard-library function associated with the new value types; or
- automatic MATLAB execution, reference downloads, generated-output commits,
  or generated-artifact publication by CI.
