# RFC 0014: Stateful compiled m components

Status: Implementation contract for the user-approved next simulation milestone.

## Scope and versions

Add model/authoring schema 3 and `/simulation/v3` on the existing listener.
Schemas 1/2 and their transports retain their contracts. Ordinary m bytecode,
SLX compatibility, multi-rate scheduling and event/reset semantics do not change.

Schema 3 models contain component definitions, keyed by stable IDs. A component
instance references an ID and stores parameter overrides. Definitions are frozen
in saved models so changing a library description cannot silently change ports or
state dimensions in an existing model. Source drafts still participate in each
immutable run snapshot. A library file uses `openmat-component`, schema 1, in
`*.omblock.json`, with relative `.m` references. No OPC or new archive is involved.

A definition declares a name, category, icon, ordered input/output port widths,
ordered parameter defaults (with optional label, unit and bounds), continuous and
discrete state counts, a discrete update period, and callback source/entry pairs.
Each callback remains one authored function in one file. Parameters flatten into
the column `p`; inputs and outputs flatten in declared port order. State vectors
`x` and `q` are per instance, explicitly owned by the scheduler. Absent states or
inputs are empty columns, never hidden globals or persistent variables.

## Callback contract

- `initialize(p)` returns `[x0; q0]`, evaluated with the instance's constant
  parameters during preparation. Required for stateful components.
- `outputs(t,x,q,u,p)` returns the concatenated output columns.
- `derivatives(t,x,q,u,p)` returns `dx/dt`, required exactly when continuous
  states exist.
- `update(t,x,q,u,p)` returns the next discrete state, required exactly when
  discrete states exist.

All components with discrete state use the model's one shared sample period.
At tick k, q[k] is exposed, outputs are evaluated, and update produces pending
q[k+1]. Pending states publish together at the next tick. This preserves the
existing UnitDelay convention, including tick-zero initialization. Output
callbacks are algebraic and may depend on current inputs between ticks; a sampled
controller holds its output explicitly in q. Trial ODE evaluations never execute
the update program. Candidate state, observations and next pending states must
all validate before any public state is committed. Cancellation/failure is terminal.

## Compilation

Reuse the existing parser/HIR and verified numerical IR. Keep the legacy function
subset unchanged. Schema-3 callbacks add fixed-size local matrices, matrix
products, transpose, constant indexing/writes, and statically bounded for loops.
Scalar comparisons and if/elseif/else are allowed for initialization/discrete
updates; continuous output/derivative callbacks reject conditional switching
until event semantics exist. No arbitrary calls, reflection, dynamic allocation,
workspace access or implicit state are introduced. Bound parsing, expansion and
total inlined work before allocating or entering recursive traversals.

Compile output functions locally, derive dependencies from the actual output
SSA, and schedule individual output ports. A non-feedthrough output can break a
feedback cycle even if another output of the same component uses current input.
False feedthrough declarations cannot bypass graph validation. Real instantaneous
cycles remain errors. Derivatives and updates may consume already-resolved outputs
without introducing an instantaneous output dependency.

The global continuous program produces derivatives and observations. The separate
update program produces pending discrete states. Both use the same scalar IR and
reference/LLVM implementations; state submission remains outside generated code.
Initialization and update programs are pure numerical functions. LLVM/CVODE host
configuration and narrow C ABI boundaries stay intact.

## Editor and acceptance

Discover project `*.omblock.json` descriptions with bounded workspace reads.
Provide component templates, definition editing, per-instance parameter controls,
callback navigation, icons, source-aware save/reopen and explicit definition
conflict handling. Persist sources with existing revision checks and source drafts.

Accept with custom UnitDelay, a mass-spring-damper component, and a sampled PI
controller driving a continuous plant. Check matrix/loop/branch semantics against
independently authored R2022b programs; test multi-instance isolation, per-output
feedthrough, genuine algebraic loops, exact sample scheduling, failure atomicity,
reference/LLVM parity, RK4/CVODE trajectories, old transports and real browser
create/edit/save/reopen/run. Preserve notices and pass public-source checks before
focused local commits. No installer or GitHub publication is part of this task.
