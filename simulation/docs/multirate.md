# Synchronous multirate simulation

Model schema 5 adds sample-time propagation and independent periodic clocks.
The existing reference and LLVM numerical backends run the same compiled model;
ordinary m execution is unchanged. See [RFC 0016](../../docs/rfcs/0016-synchronous-multirate-simulation-v1.md).

## Try it

In Model Editor, select **多速率 PI · 10 ms / 100 ms**. The continuous plant
is controlled every 10 ms and observed on a separate 100 ms branch. Run it and
switch Scope: `plant_scope`, `control_scope`, and `monitor_scope` record
1001, 101, and 11 points respectively over one second. This example uses native
blocks and can be saved as a single `.omsim` file.

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run simulation/examples/multirate-control.omsim.json
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- import-slx my-model.slx --slx-profile multirate-v1 --parameters parameters.m --output my-model.omsim
```

New editor SLX imports use `multirate-v1`. A saved `control-v1` asset retains
that profile when parameters are reapplied; older assets with no profile field
also retain `control-v1`. CLI SLX commands still default to `legacy`.

For an existing single-rate model, click **启用多速率采样** in model settings
before assigning independent periods to discrete m components. This explicitly
upgrades the saved model to schema 5; opening an older model alone does not.

## Sampling and supported blocks

`model.sampleTimes` maps block IDs to `{ "kind": "inherited" }`, `continuous`,
`constant`, or `{ "kind": "discrete", "period": 0.01 }`. Omitted entries use
the block's default or declared component period. Existing Unit Delay blocks
can use `settings.sampleTime`; an explicit inherited entry opts into inference.
Forward propagation follows signals; bounded backward propagation resolves
inherited sources from downstream constraints. Conflicting or unresolved rates
produce block diagnostics. Set the period explicitly when inference is ambiguous.

The inspector displays both the configured setting and the last checked actual
rate. Numerical edits invalidate checked rates. Scope records at its input's
rate. CSV exports use the union of observed times and leave a channel cell empty
when that channel did not sample; held values are not extra observations.
The streaming frame buffer still contains bounded accepted solver frames.

| Block | Supported behavior |
| --- | --- |
| Unit Delay | Independent fixed period or inherited rate; initial output followed by the preceding sample |
| Zero-Order Hold | Sample at its own clock and hold between hits; also samples continuous signals |
| Discrete-Time Integrator | Forward Euler, finite scalar gain, internal initial state, no reset/limit/extra ports |
| Discrete State-Space | Real double, fixed matrices and state vector; m lifecycle callbacks embedded on SLX import |
| Discrete Transfer Fcn | Proper SISO, dialog coefficients and initial state, sample based Direct form II; callbacks embedded on import |
| Rate Transition | Single task, explicit/inferred periodic output, zero offset, integer-related periods; both integrity/deterministic flags on or both off |

For deterministic slow-to-fast Rate Transition, the initial output is retained
for one source period; subsequent outputs are delayed by that source period.
With both flags off, it reads the latest source sample. Fast-to-slow reads the
current source at the target hit with either supported flag configuration.
These rules were checked against independently authored R2022b models. Insert
Rate Transition explicitly across different discrete rates entering stateful
blocks. Use Zero-Order Hold to sample a continuous signal.

Schema-5 discrete m components can declare distinct positive periods or `-1`
for inherited output/update sampling. A component with continuous and discrete
states needs an explicit positive update period and retains continuous outputs.
The version-1 reusable component-library format still requires a positive
period; choose one before exporting an inherited component to that format.

## Boundaries

This phase supports synchronous, single-task clocks. Start time and offsets must
be zero; periods and stop time must be integer multiples of `maxStep`, up to
one billion base ticks, with at most 64 distinct clocks. No asynchronous tasks,
triggered/enabled subsystems, frame-based processing, fixed-point arithmetic,
automatic rate-transition insertion, or algebraic-loop solver are provided.
Other restrictions of [SLX import](slx-import.md) still apply.

Fixed-step RK4 retains its existing stage semantics. Optional CVODE is bounded
by clock hits and continuous Step events and restarts after discontinuities.
CVODE handling is checked separately; it is not presented as an implementation
of a Simulink variable-step solver.

`/simulation/v5` and protocol `openmat-simulation-v5` use the existing listener.
They add `importSlxMultirate`, resolved `sampling` metadata and frame
`sampleHits` clock IDs. Routes v1-v4 reject schema 5. Collected schema-5 runs
use result schema 2 with `sampling`; older collected results remain schema 1.
Model/authoring versions and collected-result versions are independent.

## Verification

Portable tests cover inheritance, clock coincidences, held values, state
publication, file ordering, cancellation, failure atomicity, version gates and
unsupported rate/SLX settings. Original R2022b models cover 14 trajectories,
including a continuous plant, 10 ms controller and 100 ms monitor. Numerical
comparisons check sample times and values with reference and LLVM execution.

```powershell
cargo test --manifest-path simulation/Cargo.toml --workspace
./simulation/tools/Invoke-SlxOracle.ps1 -Profile multirate
# Set OPENMAT_SLX_MULTIRATE_ORACLE_DIR to that command's output directory.
cargo test --manifest-path simulation/Cargo.toml -p openmat-sim-slx --test multirate_oracle -- --ignored --nocapture
```

The LLVM oracle also requires `OPENMAT_SIM_LLVM_LIBRARY`. CVODE tests require
`OPENMAT_SIM_SUNDIALS_DIRECTORY`. Native dependencies, generated SLX files and
local MATLAB observations stay in ignored caches and are excluded from source.
