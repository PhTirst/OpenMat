# RFC 0015: Hierarchical SLX control-model compatibility

Status: Implementation contract for the user-approved compatibility milestone.

## Scope and compatibility

Implement the first four items of the approved roadmap: an independently
authored R2022b differential corpus, ordinary virtual subsystems, explicit
constant parameters and routing, and elementary continuous control blocks.
Keep RFC 0011's `lower()` and the v1-v3 import operations as the legacy profile.
Do not edit the accepted earlier RFCs or change their execution meanings.

The new `control-v1` profile supports nested ordinary virtual subsystems with
Inport/Outport boundaries, fixed real double vector signals, Mux/Demux, Ground,
Terminator, Product, Bias, continuous time-based Sine Wave, State-Space, proper
Transfer Fcn and a bounded Step configuration verified against R2022b. Resolve
named numerical parameters and bounded constant expressions before compilation.
Parameter files are explicitly supplied assignment-only m text; importing never
executes arbitrary scripts, callbacks, masks, dictionaries or embedded binaries.
Unknown settings remain diagnostics, with original SID, path and parameter.

Parameter matrices retain row/column shape; they are not matrix signals. Check
dimensions, initial conditions, direct feedthrough and port numbering before
lowering. Flatten only supported virtual boundaries, preserving source identities
in authoring metadata. Keep the single-rate UnitDelay profile and explicit
ode4/FixedStepDiscrete configuration. Other solvers require explicit selection;
CVODE is not advertised as a Simulink solver implementation.

## Versioned integration

Add numerical model schema 4 and `/simulation/v4` on the existing listener.
Schemas 1-3 and their transports retain their limits. Schema 4 uses the existing
component compiler and adds a native Step source with validated event time and
finite before/after values. All other imported operators reuse generated, public
OpenMat m callback sources and the existing numerical IR. General m execution
remains bytecode-based; this does not introduce ordinary-language JIT.

The compiler records scheduled source boundaries. Fixed-step RK4 evaluates a
continuous Step at each stage time, including its right-hand value at an exact
transition; this reproduces the original R2022b ode4 corpus, including off-grid
transitions. Do not replace that rule with ideal piecewise integration. An
explicitly selected adaptive solver stops at known event boundaries, integrates
the preceding interval using the left limit and restarts with the new source.
Events never update discrete component state between sample hits. This is not
general zero-crossing or sampled-Step support.

The v4 `importSlxControl` operation accepts `name`, a `bytes` array and explicit
assignment-only `parameters` text (default empty). It selects `control-v1` and
returns the structural document, diagnostics, a runnable model and generated
source bundle. Old versions reject schema 4 and the new operation. No new port,
process or production native dependency is introduced.

Authoring schema 4 preserves the SLX source package, explicit parameter text,
original hierarchy, and the compiled numerical snapshot. The editor provides
hierarchical navigation and compatibility diagnostics and supports parameter
re-import. Saved snapshots include generated callback sources so reopening does
not depend on a temporary import directory. Original SLX files are never changed;
SLX export, structural SLX rewriting and arbitrary callback execution are outside
this milestone. Numerical snapshot editing must be visibly distinct from the
preserved source structure and must never silently overwrite source edits.

The authoring envelope adds `sources` (relative .m path to source text) and `slx`
(`name`, base64 `package`, `parameters`, `appliedParameters`, `runnable`, `issues`,
`document`, optional `snapshotEdited`). Changing parameter text marks it pending;
run/check stays disabled until successful re-import. CLI execution also rejects
pending or failed SLX authoring snapshots. Native sources are frozen in the
saved document and displayed read-only; generated components are not separately
exportable to the existing project component-library format in this milestone.

Control-v1 limits State-Space to 1..32 states, inputs and outputs, and SISO
Transfer Fcn to 0..32 states. R2022b built-in Transfer Fcn's omitted denominator
is `[1 2 1]`. Root Inport/Outport external I/O, atomic/enabled/triggered subsystems,
multirate and inherited rates remain unsupported. Sine Wave and Step require
explicit continuous `SampleTime=0`; their omitted R2022b built-in value is `-1`
and must never be silently interpreted as continuous. Step permits scalar
expansion of its before/after vectors, but not discrete/sample-based settings.

## Acceptance

Use only OpenMat-authored fixtures and generators. Portable tests cover nested
ports/fan-out, unsupported subsystems and dependencies, parameter shape/errors,
all new operators, initial states and feedthrough. Run original models in the
locally licensed R2022b and compare every observed component and sample time
with reference and LLVM execution. Check event-boundary behavior separately.
Keep generated SLX packages, observations and logs out of public source.

Exercise real browser import, hierarchy navigation, explicit parameter changes,
run, save and reopen. Run relevant Rust, frontend and public-source checks before
focused local commits. No installer build or GitHub publication is requested.
