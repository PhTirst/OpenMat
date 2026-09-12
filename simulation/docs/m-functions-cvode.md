# Pure m function blocks and CVODE

This milestone adds `M Function` to the graphical editor and makes LLVM and
CVODE selectable through the existing native server. The ordinary m-language
bytecode VM is unchanged. This is a pure numerical function block, not MATLAB's
complete S-function lifecycle or MATLAB Function block compatibility.

Schema-3 [stateful components](stateful-components.md) now add OpenMat's own
initialize/outputs/derivatives/update callbacks, private states, fixed matrices,
bounded loops and a project component library. This guide retains the earlier
schema-2 M Function subset and describes the shared native runtime setup.

## Try it in the editor

1. Start the source checkout normally and open **模型编辑器**.
2. Choose **非线性摆 · m 函数**. The example creates a uniquely named `.m`
   draft in the shared workbench document session. Existing source files are
   preserved. F5 runs it with the reference kernel and RK4 without native extras.
3. Double-click **M Function**, or click **编辑 m 函数** in its inspector.
   The bottom code pane uses the same Monaco document and LSP as the workbench.
4. Edit the function and run again. Each run freezes the model, parameters,
   execution options and source text. Unsaved source edits participate in the
   run; later edits mark existing curves as outdated.
5. Save the model. `.omsim` contains versioned JSON and relative `.m` references;
   referenced source drafts are saved alongside it. Reopen the model to retain
   the same solver settings and function definitions. Clicking a source diagnostic
   opens the file at its one-based line and UTF-16 column.

The inspector declares the entry name, ordered inputs (name and width), ordered
parameter values, and one output width. Inputs precede parameters in the m
signature. Use **应用函数接口** to commit these fields; edit the m signature to
match. Every input is conservatively treated as direct feedthrough.

The authored example [pendulum.m](../examples/pendulum.m) is also valid ordinary
m syntax:

```matlab
function dx = pendulum(x, p)
omega = x(2);
acceleration = -p(1) * sin(x(1)) - p(2) * omega;
dx = [omega; acceleration];
end
```

Here `x` has width 2 and `p = [9.81; 0.2]`. The Integrator holds the two
continuous states; the function returns their derivatives.

## Supported subset

- Exactly one function per source file, one output, whole local assignments.
- Finite real double scalars and fixed column vectors. Ports are 1–4096 wide;
  at most 64 input/parameter declarations per block.
- `+`, `-`, unary `+`/`-`, scalar multiplication/division, and `.*`/`./` with
  equal-width columns or scalar expansion. No matrix product/division yet.
- Constant one-based integer literal indexing such as `x(2)` and column
  construction such as `[a; b]`.
- Elementwise `sin`, `cos`, `exp`, `sqrt`, `abs`, `tanh`. Local variables shadow
  intrinsic names for indexing; recursion is rejected.

Control flow, loops, indexed writes, scripts, arbitrary calls, dynamic shapes,
complex numbers, matrices/row vectors, cells, classes, function handles,
`eval`, workspace reflection, I/O, globals and persistent state are rejected.
The source diagnostic explains where compilation stopped. Statements are bounded
to 256 significant tokens and 32 delimiter levels; split long expressions into
local assignments. Sources are bounded to 64 KiB each, 64 files, 1 MiB total,
and 4 MiB of cumulative per-block source lowering work.

The existing handwritten lexer/parser and HIR resolve calls versus indexing.
Functions lower directly into the same verified scalar SSA as built-in blocks.
LLVM 22 runs its O2 pipeline without fast-math flags. Only six explicitly bound
C ABI math functions are visible to generated code. No arbitrary m program is
executed or JIT-compiled as a side effect of compiling a model.

## Optional native runtimes on Windows x64

From the repository root, prepare verified dependencies before starting OpenMat:

```powershell
$env:OPENMAT_SIM_LLVM_LIBRARY = & ./simulation/tools/Prepare-Llvm.ps1
$env:OPENMAT_SIM_SUNDIALS_DIRECTORY = & ./simulation/tools/Prepare-Sundials.ps1
pwsh -NoProfile -File tools/openmat-dev/Invoke-OpenMatDev.ps1
```

The scripts validate pinned upstream archive hashes and extract into ignored
`simulation/.openmat` directories. `-Destination`, `-ArchiveDirectory` and
`-Offline` support an external cache. The SUNDIALS preparation extracts only
the serial double/int64 non-MPI CVODE runtime, its configuration header and
licenses. Third-party sources and binaries are not added to the repository.

Environment variables belong to the host process; requests cannot choose a
native library path. The catalog reports whether the host configured each
optional runtime. Actual loading/ABI failures are explicit and do not fall
back to a different solver/backend. Start a new server after changing its
environment. Existing installers do not gain these changes automatically.

Select empty canvas to reveal model settings. **计算后端** chooses reference
or LLVM; **求解器** chooses RK4, CVODE Adams or CVODE BDF. CVODE uses adaptive
steps with the model's maximum-step bound, relative tolerance and absolute
tolerance. Its current dense linear solver accepts 1–2048 continuous states.
Models without continuous states use RK4's existing discrete scheduler.

The solver cannot cross a sample hit or stop time. Trial RHS calls read held
discrete outputs without committing state. At a hit, OpenMat validates and
commits the simultaneous discrete update, then reinitializes CVODE history.
Cancellation or a callback/numerical failure leaves the last committed OpenMat
snapshot intact. Rust callback panics are caught before the C boundary.
Scope reports accepted native steps and RHS evaluation counts; its tooltip
also shows error-test failures, nonlinear iterations and reinitializations.

## Command line

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- check simulation/examples/pendulum.omsim.json
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run simulation/examples/pendulum.omsim.json --backend llvm --solver cvode-bdf --rtol 1e-8 --atol 1e-10
```

`--solver cvode-adams`, `--sundials-directory PATH` and `--llvm-library PATH`
are also available. The CLI reads both raw numeric JSON and `.omsim` wrappers;
its flags choose execution (wrapper editor/execution metadata is ignored).
Source paths resolve under the model's directory; paths resolving outside it
are rejected. `emit-llvm` emits textual kernel IR, whose declared
`openmat_math_*` symbols require the corresponding C ABI bindings.

## Files and limits of this milestone

Source files are relative to the model directory; new unsaved models use
Current Folder as their base. Save As copies referenced sources under the new
directory with exclusive creation. A conflicting target is reported. Ordinary
saves use file revisions; edits made during a write remain dirty. Multi-file
saving is not an atomic filesystem transaction: a failed save can leave already
written files, and the model is reported as unsaved until the save completes.
For function models, both Web and Desktop currently save within Current Folder
through the workspace path dialog. Source-free desktop models retain their
native Save As behavior. Copy the `.omsim` and its relative `.m` files together
when moving a model to another workspace.

The new `/simulation/v2` and `openmat-simulation-v2` use the server's existing
listener. `/simulation/v1` remains available for legacy schema-1 models.
New function support does not widen SLX compatibility: arbitrary imported
MATLAB Function/S-function blocks are still unsupported.

Full MATLAB S-function compatibility, multiple rates, zero-crossing/reset events, general
algebraic-loop solving, DAE/IDA, ordinary m JIT and C code generation are future
work. This phase establishes the shared pure numerical IR and ODE integration
boundary needed for that work.
