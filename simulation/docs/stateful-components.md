# Stateful m components

OpenMat components combine an explicit interface with compiled m callbacks.
Each instance owns its continuous states `x`, discrete states `q` and constant
parameter values `p`. The graphical editor, reference interpreter, LLVM backend
and RK4/CVODE solvers all use the same definition. Ordinary m execution still
uses the bytecode VM. This interface is specified by [RFC 0014](../../docs/rfcs/0014-stateful-simulation-components-v1.md).

## Try the examples

Start the current source checkout and open **模型编辑器**. The example selector
includes three complete, editable models:

| Example | What to inspect |
| --- | --- |
| 自定义 Unit Delay · m 组件 | An m component with private discrete state, matching the built-in delay |
| 质量—弹簧—阻尼 · m 组件 | Two continuous states, a matrix RHS, and separate position/velocity output ports |
| 离散 PI + 连续系统 | A sampled controller with output saturation and integral hold, connected to a continuous plant |

Select a component in the canvas or object tree. Its inspector exposes the
declared parameter defaults/instance overrides, units and callbacks. Click
**初始化**, **输出**, **连续导数** or **离散更新** to open the actual `.m` file in
the bottom code pane. Only callbacks required by the component are shown.
Source drafts use the workbench's shared Monaco document and LSP. F5 captures
unsaved source changes and runs an immutable model/source snapshot.

Reference + RK4 works without native extras. To select LLVM or CVODE Adams/BDF,
configure the host before startup as described in
[the native runtime guide](m-functions-cvode.md#optional-native-runtimes-on-windows-x64).
The server reports the actual backend, solver and statistics. Loading failures
are explicit. No additional server port is introduced.

## Create and reuse a component

1. Choose **新建自定义组件…** in the library. Set its name, category, icon, input
   and output names/widths, continuous/discrete state counts, and public parameters.
   Each parameter has a default literal and optional display name, unit and bounds.
2. Apply the definition. OpenMat creates callback source drafts. Edit their
   equations through the inspector, connect the ports and choose **检查模型**.
3. Save the model as `.omsim`. Save also writes its referenced `.m` files using
   existing file revisions. Reopening preserves the component definitions,
   instance parameter values, source references and execution settings.
4. Select **保存到项目组件库** to write a `*.omblock.json` description and save
   its referenced sources. The project library discovers these files in Current
   Folder and its children. Use **刷新项目组件库** after external changes.
5. Drag a project component into another model. OpenMat makes uniquely named
   model-local copies of its m functions, with matching entry names. Further
   instances in that model share the definition/code and have independent states
   and parameter overrides.

The icon choices are function, continuous plant, controller, filter and delay;
all component cards and instances display an icon. **编辑组件定义…** changes all
instances of that definition in the current model. Removed ports lose their
incident connections; incompatible parameter overrides are removed so the new
defaults apply. The operation is undoable. Update the callbacks if state/port
dimensions change, then check the model again.

Saved models embed their definitions. Editing a project description does not
silently change an existing model. Different definitions with the same ID are
reported as a conflict. Use **模型内的组件** to add the existing definition, or
give an independent component a new ID in its library JSON. If the interface
matches, adding an instance uses the model's existing code and says so explicitly.
JSON property order does not create a conflict.

A library description references actual m files; editing those files changes
the source available to future imports. Exported descriptions may reference the
same m files as the exporting model. Copy the description and all relative sources
together when distributing a library. Copy a model and its relative sources
together when distributing a model. Save As preserves this relative layout.
Multi-file saves use revision checks but are not atomic filesystem transactions.
Component/function models currently save within Current Folder on both Web and
Desktop, so all referenced sources remain accessible through the workspace API.

## Callback and sampling contract

| Callback signature | Result | Invocation |
| --- | --- | --- |
| `z = initialize(p)` | Column `[x0; q0]` | Once during preparation for each instance |
| `y = outputs(t,x,q,u,p)` | Outputs in declared port order | Whenever the solver needs current outputs |
| `dx = derivatives(t,x,q,u,p)` | Column `dx/dt` | Continuous solver RHS evaluations |
| `z = update(t,x,q,u,p)` | Column containing the next `q` | Once at each accepted sample hit, including the start |

Each source contains one function with one output. The entry name may differ
from the role name, but must match the metadata. Inputs flatten into column `u`
in declared port order. Parameters flatten into column `p` in declared parameter
order; vector parameters contribute multiple consecutive elements. Parameters
are compile-time constants for one run. Absent `x`, `q`, `u` or `p` arguments
are empty columns. State and output sizes are fixed by the definition.

At sample `k`, the model exposes `q[k]`, then computes a pending `q[k+1]`.
That pending vector becomes visible at the next sample hit. All instances publish
their pending states together. The custom Unit Delay therefore starts with its
initial value and exposes the input sampled at zero at time `Ts`.
This convention also gives the example PI controller a one-period held-command
delay. It is an explicit part of this initial scheduler contract.

ODE trial evaluations never call `update`. Output functions are algebraic and
can depend on live inputs between sample hits; a sampled component holds its
output explicitly in `q`. Do not use globals or `persistent` to simulate state.
Candidate continuous state, output observations and the pending discrete state
must all validate before OpenMat commits the step. Failure/cancellation preserves
the last accepted frame and makes that run terminal.

All discrete components currently share one positive sample period. A component's
`sampleTime` must match the model setting. The PI example additionally uses a
public `period` parameter in its integration formula: keep it equal to that
period when editing the example. Arbitrary parameter-to-metadata expressions
are not implemented.

Output dependencies are derived from the compiled program separately for each
output port. A state-only output can break a feedback cycle even if another port
of that component uses its current input. A genuine instantaneous algebraic loop
is rejected with a block/port diagnostic. Users cannot override this analysis
with an incorrect feedthrough flag.

## Compilable m subset

The existing handwritten parser/HIR lowers callbacks into the numerical SSA
used by built-in blocks. The flow and discrete-update programs are separate;
both can execute through reference or LLVM. There is no interpreted callback
fallback hidden inside the optimized run.

- Finite real double signals; local fixed-size matrices in column-major order.
- Matrix literals and concatenation, matrix multiplication, real transpose,
  `+`, `-`, `.*`, `./`, scalar multiplication/division and scalar expansion.
  Matrix division, implicit row/column broadcasting and complex arithmetic are
  outside this subset. Ambiguous concatenations of differently shaped empty
  arrays are rejected.
- Constant one-based `A(i)`/`A(i,j)` indexing and scalar writes into existing
  fixed arrays. `zeros`, `ones`, `reshape(A,m,n)`, `numel` and `size(A,1|2)`.
- Elementwise `sin`, `cos`, `exp`, `sqrt`, `abs`, `tanh`.
- Static integer `for` ranges, including descending/empty ranges. Bounds may use
  compile-time parameter values. No `while`, `break`, dynamic indexing or resizing.
- Scalar comparisons, logical `~`, and `if`/`elseif`/`else` in `initialize` and
  `update`. Every branch must define a compatible fixed shape for a variable
  used afterward. These constructs are rejected in output/derivative callbacks
  until continuous event semantics are implemented.

Reflection (`eval`, `evalin`, `assignin`), arbitrary helper calls, scripts, I/O,
global/persistent state, classes, cells, handles and recursion are rejected.
Unsupported code gets a source path, line and UTF-16 column where available.
The older schema-2 **M Function** block retains its smaller pure column subset;
use a schema-3 component for matrices, loops and lifecycle callbacks.

Bounds include 64 definitions/model, 64 ports/parameters per definition, 4096
elements per fixed array and per declared input/output/parameter vector group,
and 4096 combined states/component. Sources remain limited to 64 files, 64 KiB
each and 1 MiB total. Compilation bounds each loop to 1024 iterations, expanded
statements to 100,000, control nesting to 16, and cumulative callback source work
to 4 MiB. Numerical IR and graph traversal also have explicit limits. These are
validation bounds rather than automatic performance guarantees.

## CLI and verification

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- check simulation/examples/pi-control.omsim.json
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run simulation/examples/pi-control.omsim.json --backend llvm --solver cvode-bdf
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim --test components
cargo test --locked -p openmat-server --test simulation_transport -- --include-ignored
cargo test --locked -p openmat-server component -- --include-ignored
```

The last two commands require configured LLVM/SUNDIALS runtimes. Backend parity
tests cover both reference/LLVM and RK4/CVODE, including positive and negative
PI saturation. Reference tests check analytic plant trajectories, independent
instances, per-port dependency cycles, update counts and failure atomicity.
The CLI's `emit-llvm` currently exports only the flow program; it is not a
complete component executable or C/AOT exporter.

Optional MATLAB acceptance uses only OpenMat-authored equations and functions.
In a separately licensed R2022b installation, add `simulation/tools` to the path
and call `component_oracle(output_directory)` with a fresh directory outside
the repository. This creates `components.json` containing numeric results for
matrix/loop/branch operations and an analytic held-input PI/plant trajectory.
No MATLAB code, messages, generated models or logs are stored in the repository.
Then, in PowerShell, set `OPENMAT_COMPONENT_ORACLE_DIR` to that directory and run:

```powershell
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim --test components matlab_r2022b -- --ignored
```

## Version and compatibility boundary

Components require model and authoring schema 3, served over `/simulation/v3`
with `openmat-simulation-v3`. Older schema documents still open. The v1/v2
routes retain their earlier limits and reject schema-3 models explicitly.
Library JSON uses its own `openmat-component` schema 1; it is not an OPC package.

These callbacks form OpenMat's own compiled component interface. They do not
load arbitrary MATLAB Level-2 S-functions, MATLAB Function blocks or Simulink
library implementations. Existing SLX import compatibility is unchanged.
Zero-crossing/reset events, DAE/IDA, multiple sample rates, executable subsystems,
general buses, C generation and ordinary m JIT remain future work.
