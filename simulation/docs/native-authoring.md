# Native subsystems, standard blocks and experiment data

Schema 7 adds native virtual hierarchy and experiment input/output to the model
editor. These features use the existing Rust numerical engine, reference or LLVM
backend, and RK4 or configured CVODE. They do not add Simscape physical networks,
general m-language JIT, or new SLX compatibility profiles.

## Try the complete workflow

1. Open **模型编辑器 → 打开示例 → 实验数据 · 子系统 PI 控制**.
2. Double-click **PI 控制器** on the canvas or object tree. Its child Inport,
   proportional Gain, Integrator, Sum and Outport appear. The breadcrumb and
   **返回上层** return to the parent system.
3. Select **比例 Kp = 2** and edit its gain. Undo/redo works across navigation.
   The plant at the root is a native Transfer Fcn, `1 / (s + 1)`.
4. Select **实验输入与观测** to inspect the embedded data. **导入 CSV 数据**
   reads [experiment-input.csv](../examples/experiment-input.csv); **应用数据**
   validates and commits the replacement. The second data channel is a synthetic
   observation, not a measurement from a physical experiment.
5. Run the model. In Scope choose **响应对比**, which contains reference,
   simulation and observation channels. The root **对象响应** Outport is also
   available as a separate observation.
6. Enter a variable name (default `simout`) and click **写入 m 工作区** after the
   run finishes. The current kernel receives an ordinary numeric matrix with time
   in column 1 and all selected Scope channels in subsequent columns. A matching
   existing variable is replaced only by this explicit action.
7. Close the model editor and run [analyze-experiment.m](../examples/analyze-experiment.m)
   in the ordinary source editor. It reads `simout`, plots the comparison and
   computes RMSE. Its expected columns are time, reference, simulation, observation.

The same original, self-contained model is
[experiment-control.omsim.json](../examples/experiment-control.omsim.json).
It can be opened from a different folder or checked using the CLI:

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- check simulation/examples/experiment-control.omsim.json
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run simulation/examples/experiment-control.omsim.json
```

## Create and edit hierarchy

Select peer computational blocks and choose **创建子系统** in the toolbar or
context menu. Crossing wires become Inport/Outport boundaries automatically;
fanout from the same signal shares a boundary port. Existing boundary blocks
stay in their current system. Add an empty **Subsystem** from the library when
starting a new design, then enter it and add its internal blocks and ports.

**展开子系统** moves child blocks to the parent and reconnects the boundary
wires. Copy/paste and deletion include all descendants. Connections must stay
within one system; use boundary ports to cross levels. Deleting a boundary port
removes its parent connection and renumbers the surviving ports consistently.
Disconnected graphs may be saved for later editing; native validation requires
all inputs to be connected before running.

Hierarchy is virtual: the compiler resolves the connections before scheduling
and preserves the original leaf block IDs and states. It adds no execution order,
sample rate, enable/trigger behavior or atomic boundary. Configure sample time on
the numerical blocks. Nesting is limited to 32 levels and each system has at most
64 inputs and 64 outputs. Saved JSON uses globally unique block IDs and an optional
`parent` ID. Child ports use one-based `port` numbers; container handles are
`in1..inN` and `out1..outN`.

## Standard library blocks

| Block | Current behavior |
| --- | --- |
| Product | 2–64 `*`/`/` input operations, elementwise; scalar broadcasting |
| Mux | Concatenate 2–64 fixed-width real input signals |
| Demux | Split one vector into 2–64 explicitly sized outputs |
| State-Space | `dx/dt = A*x + B*u`, `y = C*x + D*u`, explicit initial state |
| Transfer Fcn | Proper real numerator/denominator polynomials, descending powers, zero initial state |

State-Space accepts 1–32 states, inputs and outputs. Transfer Fcn accepts 0–32
states and a finite nonzero leading denominator coefficient. Use State-Space for
nonzero internal initial conditions. These blocks are continuous; discrete
transfer functions and matrix-valued signals are not introduced here. Mux/Product
outputs and the entire Demux input are limited to 4096 elements.

Matrix entry accepts finite literals such as `[-1 0; 0 -2]`. Change all affected
matrices before pressing **应用参数**; incompatible dimensions leave the previous
block intact. The fields do not evaluate arbitrary m expressions. Standard
blocks lower directly to numerical IR, alongside the existing bounded m component
subset; the ordinary m VM continues using bytecode.

## Data and result semantics

A root Inport embeds `{times, values}`; each row of `values` contains all channels
at one time. CSV/TSV uses time in seconds first, with an optional header beginning
with `time`, `time(s)`, `t` or `时间`. Data cells are numeric literals. Quoted cells,
dates, missing values and expressions are not supported.

- Time is finite, nonnegative and strictly increasing. All rows have equal width.
- Each input accepts 1–4096 rows, 1–64 channels and at most 65536 signal values;
  the model-wide authoring parameter/data budget is 262144 values. CSV uploads
  are limited to 2 MiB. The data is saved inside the model.
- Inputs interpolate linearly and hold endpoint values outside the supplied
  range. Add a Zero-Order Hold when a discrete sample clock is needed.
- Input knots are mandatory integration boundaries for schema 7. RK4 retains
  its base grid and splits steps at intervening knots. CVODE publishes its own
  accepted adaptive steps and stops at knots and discrete clock boundaries;
  output point counts may differ between solvers. Input knots do not cause
  extra discrete updates.
- Schema 7 retains the synchronous scheduler's `startTime = 0` rule and requires
  the stop time/discrete periods to lie on the `maxStep` grid.

Root Outports and Scopes record signals using their resolved sample rate. The
workspace action exports every recorded channel for the selected observation,
including channels outside the eight-channel display window. It is available
after successful completion while the m kernel is ready. The variable name must
be a valid ASCII identifier of at most 63 characters. The numeric assignment is
limited to 512 KiB; use the existing CSV export for larger results.

Opening/importing a model never executes m code in the ordinary workspace. Data
binding in this version is embedded input, and result export is an explicit
one-way assignment. It does not read arbitrary workspace expressions or keep a
live bidirectional binding. The selected result retains its original run snapshot
even if the model is subsequently edited; the editor indicates stale results.

This milestone introduced `/simulation/v7` on the configured native listener.
The current editor uses `/simulation/v8` for [conditional execution](conditional-subsystems.md).
Earlier routes reject schema 7; v7 also accepts schemas 1–6 with their existing rules.
Older OpenMat versions cannot open schema-7 authoring documents. Original SLX
packages keep their separate import/parameter workflow and are not rewritten.
See [RFC 0018](../../docs/rfcs/0018-native-model-authoring-v1.md) for the contract.

## Milestone validation

Verified on Windows x64 from the source checkout:

| Check | Result |
| --- | --- |
| Web typecheck, full Vitest suite, production Vite build | Passed; 609 tests |
| Simulation workspace ordinary tests | 114 passed; 26 opt-in tests skipped by default |
| Server ordinary tests | 98 passed; 2 opt-in tests skipped by default |
| Configured LLVM/SUNDIALS adapter tests | 18 passed, including both CVODE methods and irregular input knots |
| Configured native server transport acceptance | Passed |
| Clippy with warnings denied, simulation workspace and server | Passed |
| Real browser → schema-7 simulation → m workspace → Figure | Passed; LLVM + CVODE BDF produced `simout` of size 840 × 4 and RMSE about 0.0083 |
| Saved model moved without its CSV file, then CLI run with LLVM + CVODE BDF | Passed; 840 frames |

The browser also exercised subsystem navigation, parameter edit/undo, CSV import,
save and the standard inspector. Editor tests cover nested copy/remapping,
group/ungroup, boundary renumbering, save/reopen and invalid input preservation.
MATLAB-generated differential oracle fixtures were not regenerated for this
milestone; analytical trajectories and existing ordinary SLX regressions were used.

Commands, from the repository root (configure the native runtime environment
using [the runtime guide](m-functions-cvode.md) before opt-in tests):

```powershell
pnpm --dir apps/web typecheck
pnpm --dir apps/web exec vitest run --maxWorkers=4
pnpm --dir apps/web exec vite build
cargo test --manifest-path simulation/Cargo.toml --locked --workspace
cargo test --locked -p openmat-server
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim-llvm -p openmat-sim-sundials -- --include-ignored
cargo test --locked -p openmat-server --test simulation_transport v2_native_solver_selection -- --ignored
cargo clippy --manifest-path simulation/Cargo.toml --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p openmat-server --all-targets -- -D warnings
```
