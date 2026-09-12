# RFC 0010: Simulation kernel v0 and shared numerical compilation

Status: Implementation contract for the opt-in simulation milestone.

## Scope and compatibility

Add a headless causal block-diagram simulator in the independent `simulation/`
Cargo workspace. This is a new OpenMat modeling API, not a claim of Simulink or
SLX compatibility. It does not change the accepted MATLAB R2022b language
baseline, the existing bytecode VM, kernel protocols, root workspace, or desktop
distribution. SUNDIALS, a graphical editor, general m-language JIT, DAE solving,
zero crossings and multiple sample rates are subsequent milestones.

## Model document

UTF-8 JSON with `schemaVersion: 1`, a name, settings, blocks and connections.
Block IDs and named ports are persistent identity; document order and optional
editor positions do not determine execution. Unknown fields, invalid ports,
multiple drivers, missing required inputs, invalid sizes and non-finite
parameters are errors. No user source or native library paths are executed from
the model document. Parameters are numerical snapshots in this version.

Signals are real double scalars or fixed-width column vectors. A one-element
vector represents a scalar. Constants and state initial conditions determine
widths; Sum inputs must have equal widths. Gain accepts one coefficient or one
per element; it is explicitly elementwise, not a matrix multiplication block.
The editor metadata is optional and has no numerical effect.

Initial blocks: Constant, Sum, Gain, Integrator, UnitDelay and Scope. Scope is
a signal observer; it does not evaluate the runtime or cause state updates.
The required port names are `out` for producers, `in` for unary consumers and
`in0`, `in1`, ... for Sum in the same order as its `signs` array.

## Model IR and numerical IR

The model compiler checks connections, orders instantaneous output dependencies,
assigns state storage and records each scope and state origin. Integrator and
UnitDelay outputs are state reads and therefore break direct-feedthrough cycles.
Their input dependencies are evaluated after those outputs exist. Instantaneous
algebraic loops are rejected with an offending block/port; no delay is inserted.

The numerical IR is a verified, immutable, typed f64 SSA program. Instructions
are input reads, finite constants, addition, multiplication and negation. The
compiler scalarizes fixed-width signals. Inputs are `[time, continuous states,
held discrete outputs]`. Outputs are `[continuous derivatives, next discrete
values, scope values]`. This numerical layer has no graph or time scheduler
dependency and is the future entry point for typed m-language numeric functions.

## Time and state semantics

Simulation time is independent of wall time and presentation. Start and stop
times must be finite and ordered, and the maximum integration step positive.
Discrete blocks share one positive sample period. Hits are computed as
`start + integer_tick * sampleTime`, never by accumulated sample-time additions
or floating-point modulo. Integration steps end at the next hit or stop time.
Coincident boundaries are coalesced within a documented round-off tolerance.

Integrator output is its continuous state; its input is its derivative. RK4
trial evaluations do not change discrete state, emit scope data or mutate the
model. At each accepted time the continuous state is committed, due discrete
outputs are updated, and scope data is recorded once, on the right side of the
sample boundary. All UnitDelay blocks update simultaneously.

UnitDelay has an explicit initial held output. At hit zero it captures its
input for the *next* hit. At hit k > 0 it publishes the value captured at hit
k-1, computes all signals from the newly published outputs, and captures new
inputs for hit k+1. Between hits its output remains held. This prevents either
an extra delay or a premature continuous input change during RK4 trials.

A runner supports initialization and accepted-step advancement with cooperative
cancellation. A failed or cancelled run is terminal and retains its last accepted
time, states and observations, including when the final candidate evaluation fails.
Buffers are preallocated;
trial evaluations do not allocate trajectories. The convenience full-result
collector has explicit sample/value limits; streaming callers can consume one
frame at a time. This is a storage bound, not a resource scheduling policy.

Boundary coalescing uses `8 * f64::EPSILON * max(abs(a), abs(b), MIN_POSITIVE)`.
Step sizes and sample periods must be at least `64 * f64::EPSILON` times the
maximum absolute start/stop time (floored at `MIN_POSITIVE`). Tick numbers are
unsigned 32-bit integers; exhaustion is an explicit terminal error.

The v0 compiler limits models to 10,000 blocks, 64 inputs per Sum, 262,144 total
parameter/state components, 262,144 total numerical output components, 1,000,000
instructions and 1,000,000 stored signal references. Expanded instructions and
fanout storage are checked before allocating each block's contribution. The CLI
also limits model JSON input to 16 MiB. These are predictable prototype bounds,
not multi-user resource isolation.

## Execution backends and ABI

The reference interpreter and LLVM backend execute the exact same verified
numerical program and are checked against one another. A runner rejects a
backend built for a different program. Kernel ABI v1 is a C-callable function
with an ABI version, input/output pointers and 64-bit element counts, returning
a numeric status. See `simulation/include/openmat_sim_kernel.h`. Rust types,
allocators and unwinding do not cross this boundary. Generated functions contain
no callbacks or external symbol dependencies. No fast-math flags are emitted.

LLVM is an explicitly selected, trusted optional native dependency. The adapter
loads LLVM's C API and uses ORC LLJIT; it does not substitute an external C
compiler or report an interpreter fallback as native execution. A missing or
incompatible LLVM library is a clear error. LLVM objects and generated code
stay alive for every kernel invocation. No LLVM binary or third-party source
is committed or added to desktop packages by this milestone.

## Acceptance

Project-authored fixtures and tests cover the analytic solution of x'=1-x,
RK4 step convergence, one-period delay sequences, simultaneous delay updates,
sampled continuous feedback with non-aligned integration steps, final boundaries,
vector widths, cancellation, model diagnostics and JSON round trips. Native
Windows x64 acceptance executes actual ORC-generated machine code, compares
trajectories with the reference backend and validates the C ABI rejection paths.
The explicit native acceptance test must fail if LLVM is missing; optional local
test configuration must never silently claim that native verification passed.

## References

- LLVM ORC: https://llvm.org/docs/ORCv2.html
- LLVM C LLJIT API: https://llvm.org/doxygen/group__LLVMCExecutionEngineLLJIT.html
