# RFC 0011: Bounded OPC reading and SLX import v0

Status: Implementation contract for the opt-in SLX import milestone.

## Scope

Extend the independent `simulation/` workspace with an OPC reader, structural
SLX loader and an explicitly constrained Simulink R2022b execution profile.
Lower supported models into RFC 0010's existing model, compiler, reference
backend and LLVM ORC adapter. No existing language or server protocol changes.

This milestone does not implement an editor, server jobs, SLX writing, MDL
loading, general m-language JIT or deployable application code generation.
The future OpenMat native OPC format is for PDE/FEM and is outside this work.
No native simulation container format is introduced here. JSON export remains
a numerical snapshot in RFC 0010's existing experimental format.

## Boundaries

`openmat-opc` knows only ZIP/OPC, not Simulink or PDE semantics. It reads
content types and internal/external relationships, resolves package URIs,
retains part bytes, and never extracts paths or fetches external resources.
ZIP decompression and CRC validation use zip-rs; XML parsing uses roxmltree.

The v0 profile accepts ordinary single-volume Stored/Deflated ZIP, UTF-8 XML,
and internal package part targets. ZIP64, encryption, symlinks, multiple possible
ZIP end records, ambiguous part names, invalid content types, dangling internal
relationships, DTDs and oversized inputs fail explicitly. Signatures are not
verified. This is an OPC read profile, not a complete OPC conformance claim.

Defaults bound input to 64 MiB, 4,096 ZIP entries, 32 MiB per part, 128 MiB total
expanded bytes and 200,000 XML nodes per parsed part. The SLX layer additionally
bounds reference recursion to 32, systems to 1,024, blocks to 10,000, line branch
depth to 64 and literal/state component expansion to 262,144. Import never
allocates a large inferred state vector before checking its component budget.

`openmat-sim-slx::ImportedSlx::read` resolves the block diagram, active configuration,
release metadata and referenced systems through relationships. Inline systems
are also accepted. IDs, Unicode names, properties, port metadata, branched
lines and nested system structure are retained. Each structural node records
its package part and UTF-8 byte span. Unknown parts remain accessible as bytes.
Configuration properties retain their owning object class and source span;
repeated descriptive property names in different objects are not conflicts.

`ImportedSlx::lower` is a separate fallible operation. It validates the release,
configuration, blocks, parameters and graph before returning a runnable model.
Diagnostics include a code and, where available, source part, original SID and
parameter/port. An unsupported block is retained for inspection but never
substituted with a different block, zero signal or inferred delay.

## Executable profile

- R2022b `ModelInformation` version 1.0, normal simulation, one root system.
- `ode4` or `FixedStepDiscrete`, literal positive `FixedStep`, `StartTime=0`
  and finite nonnegative `StopTime` on the step grid.
- Finite real double scalars and fixed one-dimensional signals; literal
  scalars, row-vector or column-vector notation only. No expression evaluator.
- Constant, elementwise Gain, multi-input Sum, unreset/unlimited Integrator,
  sample-based UnitDelay and a one-input Scope observer.
- R2022b defaults for those supported blocks when parameters are omitted.
  Custom block defaults are not applied implicitly.
- UnitDelay requires one explicit positive sample period shared by all delays,
  an integer multiple of `FixedStep`. Scalar state initial values may expand
  to the signal width; non-scalar initial values must match that width.

SLX SIDs map to stable runtime IDs (`slx_` prefix); ports map to the existing
named runtime ports. Branches become fan-out connections. Width and port checks
and algebraic-loop detection use the existing compiler. Sum broadcasting,
scalar-input/vector-output Gain expansion and matrices are outside the profile.

Callbacks, workspace/dictionary dependencies, libraries, model references,
subsystems, masks, bus/complex/fixed-point signals, inherited or multiple discrete
rates, reset/saturation ports, events and unsupported solvers fail execution.
Nonempty external relationships and structured/referenced block parameters are
also rejected. Unknown block parameters, solver properties and configuration
component classes are diagnosed. Known graphical/logging/code-generation metadata
and inactive settings for other solvers are retained without applying them.
The OpenMat Scope observer collects accepted engine frames; it does not emulate
the Simulink Scope UI, logging destinations, decimation or MAT-file output.

The runtime performs RK4 and discrete scheduling as specified in RFC 0010.
LLVM compiles only the numerical kernel through the existing C ABI. Loading
SLX never starts MATLAB or executes callbacks, scripts or embedded binaries.

## Command-line behavior

- `inspect-slx`: structural document plus `runnable` and compatibility issues.
  `ok: true` means structural loading succeeded, not that execution is supported.
- `import-slx`: export a supported numerical snapshot as JSON. This export is
  intentionally lossy with respect to SLX editor/unknown metadata.
- `check`, `run`, `emit-llvm`: accept JSON or `.slx` (case-insensitive suffix).
  SLX lowering must succeed before compilation or execution.

Failures use stderr JSON and exit status 1. Output files use create-new semantics;
source packages are never modified. Inspection does not promise that every SLX
release, dependency scheme or document structure can be parsed.

## Verification and provenance

Portable tests assemble small, independently authored XML/ZIP packages in memory.
They test references, preservation, defaults, branches, widths, analytical
trajectories, malformed inputs and explicit compatibility failures. Windows CI
also executes an imported model using actual LLVM ORC machine code.

Optional local differential tests use `simulation/tools/slx_oracle.m` to create
four elementary models through public Simulink APIs: first-order feedback,
vector feedback with scalar initial-condition expansion, a delay counter and a
sampled continuous plant. Both backends compare every logged time and component
with local R2022b observations. These tests require explicit invocation and fail
if the licensed installation, generated observations or selected LLVM runtime
is absent; an omitted test is never reported as successful acceptance.

Only the project-authored generator and tests are public source. Generated SLX
files, observations, native libraries and local logs are not committed. No
MATLAB implementation, supplied models/tests, documentation or diagnostic prose
is used as a checked-in fixture. See the guide for upstream format/API references.
