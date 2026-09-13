# RFC 0018: native hierarchy, standard blocks and experiment data

Status: accepted for the user-approved native modeling milestone.

Scope: `simulation/`, `apps/web/src/simulation/`, the ModelEditor integration in
`apps/web/src/App.tsx`, simulation server routes/tests, and this milestone's docs.
Existing language, kernel protocol and C kernel ABI contracts remain in force.

## Serialized contract

Model and authoring schema 7 add optional `parent` on blocks. IDs remain globally
unique. A parent must be a `subsystem` block; nesting is limited to 32 levels.
Connections join blocks within the same parent system. A virtual subsystem has
`inputs` and `outputs` counts (0..64), with ports `in1..inN` and `out1..outN`.
Its child `inport` and `outport` blocks have contiguous, unique one-based `port`
numbers. Internal Inport has output `out`; Outport has input `in`. Flattening
resolves boundary aliases before numerical scheduling, preserves leaf block IDs,
and neither introduces state nor imposes execution order. Container/inner port
sample-time overrides are rejected. No enabled, triggered or atomic semantics.

A root Inport carries optional embedded `data: {times, values}`; `values` is a
row-major JSON array of time rows, each containing the fixed-width signal vector.
Compilation requires data for every root input. Times are finite, strictly
increasing, nonnegative, at most 4096 rows, width 1..64, at most 65536 values per
input and 262144 total parameter/data values per model. Linear interpolation and
endpoint hold are explicit OpenMat semantics. No filesystem lookup occurs during
simulation. Known data knots become integration boundaries in schema 7 for RK4
and CVODE. They do not create extra discrete clock hits. Root Outports are recorded
as observations alongside Scopes; their block IDs identify the returned channels.

`standard` blocks carry a tagged `operation`: elementwise `product` with a string
of 2..64 `*`/`/` operations, `mux` with 2..64 inputs, `demux` with explicit positive
output widths, `stateSpace` with real rectangular A/B/C/D and internal initial
state, or `transferFcn` with proper numerator/denominator polynomials and zero IC.
State-Space supports 1..32 states, inputs and outputs; Transfer Fcn 0..32 states.
Routing permits fixed-width real vectors; matrix parameters do not introduce
matrix-valued signals. Standard blocks lower directly to existing numerical IR.

The versioned `/simulation/v7` route shares the kernel listener. Schema 1..6 are
accepted there without changing their numerical rules; earlier routes reject 7.
CLI accepts schema-7 authoring documents and embedded sources. Existing SLX
profiles retain their contracts; this milestone does not claim new SLX semantics.

## User flow and verification

The editor can group selected peer blocks, generate boundary ports, navigate and
edit children, ungroup, copy entire nested subsystems, undo/redo and save/reopen.
All new blocks have icons and typed parameter controls. CSV time-series import
uses the first column as time; data is embedded and a user can inspect/replace it.
Completed simulation results can be assigned to a user-named m workspace matrix,
one row per selected Scope/Outport observation, time in column 1. This explicit
action uses the current kernel session's existing execute request with validated
numeric literals and a validated variable identifier. It is bounded by source
size, preserves user data on validation failure and reports execution failure.
No automatic kernel execution takes place on file import/open.

Tests cover hierarchical equivalence and feedback, nested copy/save/remapping,
invalid boundaries and data, standard numerical functions, irregular-knot
integration, native LLVM/CVODE, editor workflows and m workspace round trips.
Existing original R2022b fixtures may be reused for standard-block comparisons;
generated MATLAB artifacts remain ignored. Physical conserving networks and IDA
are a later milestone.
