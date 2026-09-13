# RFC 0016: Synchronous multirate simulation and SLX sampling

Status: Implementation contract for the user-approved sampling milestone.

## Scope and version boundary

Add sample-time propagation, synchronous multirate execution, sampled control
blocks, and editor diagnostics. Preserve the accepted schemas 1-4 and SLX
legacy/control-v1 profiles. Introduce model/authoring schema 5, the
`multirate-v1` SLX profile, and `/simulation/v5` on the existing listener.
No changes to ordinary m execution, native ABI, listening ports, or deployment.

Schema 5 adds a `sampleTimes` map keyed by block ID. Values have a `kind` of
`inherited`, `continuous`, `constant`, or `discrete`; discrete values include
`period`. Missing entries use the block's declared or inherited rate. Resolve
rates before numerical compilation. Initially allow zero offsets and explicit
periods on the model's fixed base-step grid, bounded to 64 distinct clocks.
Unknown/conflicting rates, unsupported offsets, and unsupported rate-transition
configurations are errors, never implicit reinterpretations.

Use R2022b observations of independently authored models to define inherited
source rates, same-time ordering, initial output, and cross-rate latency. Support
forward propagation through algebraic blocks and virtual subsystems and bounded
backpropagation to inherited sources. Preserve constants without assigning an
artificial discrete clock. Resolve continuous state outputs as continuous.

## Execution contract

Each periodic group has an integer tick counter and a validated period. A block
with discrete output sampling publishes new outputs only on that group's hits;
other evaluations observe its held outputs. A discrete state exposes q[k] at
tick k, and its update computes pending q[k+1], published on its own next hit.
All state candidates and observations validate before the run commits a frame.
Independent block ordering in the file must not alter results.

Continuous flow and sampled evaluation are separate pure numerical programs
using the existing verified scalar IR and reference/LLVM implementations. The
schema-5 sampled program additionally receives held outputs and active clock
flags and returns new held outputs and pending states. Inactive groups preserve
their published values and pending states. ODE trials never publish sampled
outputs or execute the sampled program. Continuous/discrete state and output
buffers remain instance-owned and bounded; no implicit workspace state exists.

The scheduler bounds integration by the next discrete hit, scheduled source
boundary, and stop time. Fixed-step RK4 retains its established stage semantics;
an explicitly selected CVODE solver restarts at published discontinuities.
CVODE is independently checked for event handling and integration accuracy,
not advertised as a Simulink variable-step solver implementation.

The old custom-component contract remains valid in old schemas. Schema 5 may
schedule distinct component periods. A component with continuous and discrete
states retains continuous outputs and a separate discrete state-update clock.
State-only discrete components have sampled, held outputs and may declare `-1`
to inherit their period in schema 5. Mixed continuous/discrete components must
keep an explicit positive state-update period. Generated SLX discrete
State-Space/Transfer Fcn callbacks use the same numerical IR as other components.

## Blocks and authoring

Extend Unit Delay rate handling; add Zero-Order Hold, Discrete-Time Integrator
(initially forward Euler, internal initial state, no reset/limit ports),
Discrete State-Space, proper SISO Discrete Transfer Fcn, and explicitly bounded
Rate Transition settings confirmed by R2022b. Preserve the distinctions between
sampling/holding, a unit delay, and cross-rate transfer latency.

The v5 transport accepts `importSlxMultirate` and exposes resolved block rates,
clock identities, and per-frame hit identities. Older transports reject schema
5 and the new import operation. Saved schema-5 authoring documents retain the
original SLX, explicit parameters, generated sources, and compiled snapshot;
pending or failed parameter application still blocks execution.

Collected schema-5 runs use result schema 2, including resolved `sampling`;
older collected results remain schema 1. Result versions are independent of
the model/authoring version. Scope rates describe observation of the input
signal, rather than Simulink's union of block execution sample times.

The editor displays configured and resolved sample times, identifies conflicting
connections, and records/displays sampled Scope values at their own hit times.
Add native authoring controls/icons for supported sampled primitives. Numerical
editing, copy/paste, save/reopen, source navigation, and original hierarchy
navigation continue to work. Imported source structure remains read-only.

## Acceptance

Portable tests cover rate propagation, offsets/grid rejection, initial values,
clock coincidences, held outputs, cross-rate transfer, callback/state isolation,
algebraic cycles, cancellation, failure atomicity, and legacy compatibility.
Original R2022b oracle models cover continuous/discrete sources, discrete state
equations, fast/slow branches and a continuous plant with a 10 ms controller and
100 ms monitoring branch. Compare sample times and all observed components in
reference and LLVM runs, including save/reopen snapshots. Keep generated SLX,
reference observations and runtime binaries outside public source.

Verify CVODE boundaries separately. Exercise browser import, rate diagnostics,
run, save/reopen and moved single-file execution. Run relevant Rust/frontend/
desktop checks and the staged public-source check before focused local commits.
No installer generation or GitHub publication is requested.
