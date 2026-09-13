# RFC 0017: Nonlinear control and hybrid events

Status: Implementation contract for the user-approved control/event milestone.

## Boundary

Model/authoring schema 6, SLX profile `hybrid-v1`, and `/simulation/v6` extend
the existing listener. Schemas 1-5 and their import profiles retain their
execution behavior. No ordinary m-language, numerical C ABI, root workspace
manifest, deployment, or published protocol specification is changed.

Support Saturation, Switch, Relational Operator, Logical Operator, Abs, MinMax,
and continuous/discrete forward-Euler integrators with external edge reset.
Use finite real doubles and bounded fixed-width vectors; logical results are
normalized 0/1 in the existing numerical ABI. Compare/logic outputs carry
logical type metadata. Reject unsupported data types/settings explicitly.

## Events

Compile pure event functions from signal dependencies. Fixed-step RK4 keeps
stage evaluation semantics and detects resets only at accepted major steps;
sampled blocks detect changes only at their clock hits. CVODE additionally
locates continuous zero crossings using its rootfinding API. Root evaluations
never publish state or invoke reset actions. The scheduler applies validated
reset candidates and restarts integration after discontinuities.

At a coincident clock boundary, publish pending discrete states, evaluate
sampled inputs, detect reset edges, reset affected states, then recompute
sampled outputs/updates and continuous observations before atomic publication.
Define initial/zero-edge behavior using independently authored R2022b oracles.
Do not silently add hysteresis or perturb physical states to suppress events.
Bound event functions and event work; failures retain the last accepted frame.

Initially support rising/falling/either reset, internal initial conditions,
scalar reset signals, and no reset state-output port or reset algebraic loops.
Continuous event functions must be finite and sufficiently regular for root
bracketing. General tangential/multiple-root detection is not promised.

Frame event records identify the block, event kind and direction. The editor
adds icons, parameter controls, reset ports and Scope event markers. Saved
models remain self-contained, with original SLX structure and source metadata.

## Verification

Portable tests cover broadcasting, logic, widths, cycles, event ordering,
sampling, reset initialization, failure atomicity and old version gates.
R2022b oracles use only OpenMat-authored programs and public model APIs.
Compare fixed-step traces with both reference and LLVM; check CVODE root times
and restarts separately against analytic examples. Exercise import, edit,
save/reopen and moved-file execution in the browser/CLI. Run relevant Rust,
frontend, desktop and public-source checks before focused local commits.
