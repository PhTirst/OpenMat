# RFC 0013: Static m-function blocks and CVODE

Status: Implementation contract for the user-approved next simulation milestone.

## Boundary and versioning

Add pure, statically compiled m-function blocks and optional CVODE integration.
The ordinary m-language bytecode VM and accepted language behavior do not change.
The first function subset is finite real double scalars/fixed column vectors,
local assignments, arithmetic, constant one-based indexing, column construction
and a small documented mathematical intrinsic set. Dynamic execution, reflection,
I/O, implicit state, calls to arbitrary functions and control flow are rejected
with source locations. Stateful S-function lifecycle compatibility remains later
work; Integrator and UnitDelay continue to own states.

Numerical model schema 2 adds `mFunction` blocks. Existing schema 1 remains
readable and cannot silently opt into source execution. Each function block
declares a relative `.m` source path, entry function, named input widths, named
parameter snapshots and one output width (`out`). Its file must contain one
function with the declared signature. All input dependencies are direct
feedthrough in this first subset. Function source is an immutable, bounded
bundle supplied explicitly at compile/run time, never loaded by the numerical
compiler. Imported SLX callbacks and S-functions remain rejected.

Authoring files persist relative source references. The UI uses the existing
workspace/document session for source editing and saves; the run snapshot uses
current editor text when open. File content changes invalidate displayed results
and are isolated from already-running jobs. Unsupported versions, absent source,
invalid shape and stale file revisions are explicit errors.

`/simulation/v2` and `openmat-simulation-v2` add a source bundle and explicit
execution options. The old endpoint remains available for old numerical models.
LLVM and SUNDIALS library paths come only from trusted host configuration, never
from client/model data. Missing native dependencies fail explicitly. Catalog
reports backend/solver availability. Run acknowledgements identify the selected
backend and solver; terminal results include integration statistics.

## Compilation

Reuse the existing lossless parser and HIR. Resolve call/index ambiguity using
the static local symbol table. Track shape separately from scalar SSA, preserve
column-major and one-based semantics, inline the function into the model program
and enforce expansion limits before allocation. Extend the shared verified
numerical instructions and both reference/LLVM implementations consistently.
Generated arithmetic keeps strict IEEE operations; approved mathematical calls
use explicit host C ABI symbols. No general native symbol lookup is exposed to
model source. This is static numerical compilation, not general m-language JIT.

## Continuous solver

Keep RK4 available. Add a solver boundary beneath the existing scheduler, and a
separate native CVODE adapter using the documented C ABI. Start with serial
double precision, Adams/BDF and bounded dense linear algebra. Tolerances, maximum
step and method are explicit options. The native dependency is optional, pinned
by version and checksum; only upstream binary packages are downloaded outside
version control, with notices preserved.

CVODE trial evaluations read held discrete outputs and never commit state or
emit Scope samples. Stop times prevent integration past a discrete hit. After a
hit the scheduler publishes all pending delay values together and reinitializes
solver history for the changed right-hand side. Cancellation or failure leaves
the last accepted public state intact. All FFI resources have deterministic
ownership, callbacks contain Rust panics, and failures include solver context.

## Acceptance

Test scalar/vector arithmetic and indexing against independently authored
results, source diagnostics and rejection of dynamic features, old-model
compatibility, snapshot isolation, native LLVM parity, nonlinear pendulum and
stiff ODE solutions, sample-boundary semantics, tolerances, native lifetime and
cancellation. Verify actual browser source-edit/save/reopen/run workflows,
native dependency unavailable behavior, and both solvers. Update CLI and guides,
run relevant checks and public-source guards, and make focused local commits.
