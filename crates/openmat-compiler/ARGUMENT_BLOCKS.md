# `arguments` and `Name=Value` (MATLAB R2022b baseline)

## Implemented

```matlab
function y = scaled(x, options)
    arguments
        x (1,:) double {mustBeReal, mustBeFinite}
        options.Scale (1,1) double {mustBePositive} = 2
    end
    arguments (Output)
        y (1,:) double
    end
    y = x * options.Scale;
end

y = scaled([1,2,3], Scale=3);
```

- Parenthesized calls accept `Name=Value`, including ordinary functions without
  argument blocks. The lossless CST retains a dedicated node; HIR lowers it to
  a string name and the value. Expressions and side effects are evaluated once.
- Legacy name/value pairs may precede this syntax; positional arguments may not
  follow it. Brace indexing, non-identifier names and parenthesized assignments
  are rejected. Call/index ambiguity remains unresolved by the parser.
- Leading input blocks bind required and optional positional arguments and one
  or more option structures. All signature inputs must be declared in order.
- Defaults are lazy, can reference preceding arguments and captured outer
  variables, and undergo conversion/validation too. Omitted options without
  defaults remain absent fields. Option fields finish in declaration order.
- Exact option names take precedence; unambiguous case-insensitive names and
  prefixes work. Repeated options use the final value. Ambiguous/unknown names,
  missing required inputs and incomplete pairs fail before the body executes.
- `nargin` counts supplied positional inputs, excluding option pairs and inserted
  defaults. Functions without blocks keep their original raw input-count rules.
- Class constraints retain matching classes/subclasses and otherwise invoke the
  class conversion through the existing runtime. Size declarations support
  nonnegative integral numeric literals and `:`. Dense numeric scalar expansion,
  row/column vector orientation and empty-vector shape conversion are supported;
  general matrix broadcasting/reshaping is not performed.
- Validators are ordinary source-resolved calls, including user-authored m
  functions. Both `{validator}` and `{validator(x, extra)}` work. Errors preserve
  the existing catchable exception path and custom identifiers.
- `(Output)` blocks run at a common function exit, outside body exception-handler
  ranges, on both explicit and implicit returns. Other outputs cannot be used by
  output validators; output defaults are rejected.
- The paths above cover source-loaded m functions, class methods, nested captures,
  function handles, `feval` and runtime-sized multi-output assignments.

Added source-callable validators: `mustBeNumeric`, `mustBeFloat`, `mustBeReal`,
`mustBeFinite`, `mustBeNonNan`, `mustBeNonempty`, `mustBePositive`,
`mustBeNonnegative`, `mustBeNegative`, `mustBeNonpositive`, `mustBeNonzero`,
`mustBeInteger`, `mustBeMember`. `isfield` supplies scalar and array name queries.

### Repeating parameters and range validators (2026-09-06)

```matlab
function results = pairs(x, y, options)
    arguments (Repeating)
        x (1,:) double {mustBeFinite}
        y (1,:) double {mustBeGreaterThan(y, 0)}
    end
    arguments
        options.Scale (1,1) double = 1
    end
    arguments (Output,Repeating)
        results (1,:) double {mustBeFinite}
    end
    results = cell(1, numel(x));
    for k = 1:numel(x)
        results{k} = (x{k} + y{k}) * options.Scale;
    end
end

[a,b] = pairs(1, 2, 3, 4, Scale=2); % a=6, b=14
```

- Input `Repeating` blocks bind complete groups into one row cell array per
  declaration; zero groups produce 1-by-0 cells. Fixed/defaulted positional
  inputs precede the repeating block, and separate name-value blocks follow it.
  `varargin` can be the sole declaration of the repeating block.
- Group arity and option binding fail before validators/body effects. Option
  matching only starts at group boundaries, not at a name-looking element inside
  a group. Unknown names remain positional data until the option tail starts;
  ambiguous option names at group boundaries are rejected.
- Validation/conversion runs in call order: group first, then declaration.
  Validators see the current element and converted preceding input elements;
  forward references to later input declarations are rejected. Body code
  and its nested captures see the complete converted cell arrays. `nargin`
  counts all supplied fixed and repeated positional arguments, not option pairs.
- Output `Repeating` declares one final output, either `varargout` or an ordinary
  name. Fixed outputs may precede it. The cell is expanded through the existing
  variadic return path, including function handles and runtime-sized assignments.
  Every populated element is validated, even if not requested by the caller.
  An unassigned tail is empty; an assigned non-cell is invalid even for zero
  requested outputs. Column/matrix cells retain column-major element order.
  Validation remains outside body exception handlers on early and normal exits.
- Added `mustBeGreaterThan`, `mustBeGreaterThanOrEqual`, `mustBeLessThan`,
  `mustBeLessThanOrEqual`, and `mustBeInRange`. These accept real numeric/logical
  values with scalar bounds, preserve exact 64-bit integer comparisons, and
  inspect sparse stored values plus implicit zeros without densifying arrays.
  Complex storage, text and nonscalar bounds are rejected. Empty numeric inputs
  pass valid scalar-bound checks, including NaN or reversed bounds.
- `mustBeInRange` implements inclusive/exclusive/exclude-lower/exclude-upper,
  case-insensitive unique flag prefixes, and the permitted pair of lower/upper
  exclusions. Conflicting or repeated flags are rejected. Standalone calls and
  calls from validation blocks share the same zero-output builtin implementation.

Semantics were checked using OpenMat-authored programs in installed MATLAB
R2022b. Current public references are [repeating parameters](https://www.mathworks.com/help/matlab/matlab_prog/validate-repeating-arguments.html)
and [range validation](https://www.mathworks.com/help/matlab/ref/mustbeinrange.html);
the implementation targets R2022b, not later documentation-only changes.

## Runtime and shared interface

Bytecode version 28 introduced optional `Function.argument_layout`: positional
and required counts plus the owning local slot and field name of each option.
Version 29 adds `repeating_count`, the number of repeated signature slots between
fixed positional inputs and option owners. Verification checks version, checked
slot arithmetic, complete/disjoint slot coverage and duplicate fields. A layout
cannot coexist with the old variadic-input marker. Old v24–27 modules without
this metadata and v28 layouts with zero repeated slots remain accepted.

The runtime binds inputs after the existing language-copy boundary, retaining
the original copied inputs as GC roots. Defaults, type/size conversions and
validators are executable bytecode calls, not serialized source or native
callbacks. Validators continue to use the heap-managed m call stack.
Repeated validation uses bytecode loops and the same calls; output expansion
reuses `ReturnVariadic`. No new instruction or OEX C ABI entry is required.

The module linker preserves function-owned metadata by cloning it; field names
contain no relocatable constant or function IDs. Validated functions cannot be
inlined as script entries. Existing bytecode fixtures (including OEX fixtures)
explicitly initialize the new optional field. The OEX C ABI is unchanged.
Constructor-handle dependencies now load classdef sources as well as function
sources, so class conversion does not require a prior manual construction.

## Remaining boundaries

- `options.?ClassName` property imports are not implemented; unsupported
  declarations fail explicitly.
- This is not the complete MATLAB validator library. Missing validators must be
  supplied as m functions or implemented separately.
- Automatic shape changes reuse existing numeric `reshape`/`repmat` support.
  Shape-changing conversions for objects, strings, cells and structs are not yet
  complete. Already conforming shapes still pass without conversion.
- Some built-in numeric validators do not yet accept sparse storage or custom
  overloaded numeric classes. User validators can handle additional domains.
  The five new scalar-bound range validators support real/logical sparse arrays,
  but do not dispatch user-defined relational overloads.
- Custom class constraints under wildcard imports require a fully qualified
  class name in this stage; ambiguous dynamic type-name resolution is rejected.
  Argument blocks must be the leading function statements (imports can follow
  the blocks because imports are collected lexically).
- OpenMat-owned diagnostics are not copies of MATLAB messages or exact MATLAB
  identifier compatibility. Property validation syntax is outside this change.

## Verification

### Repeating/range extension, 2026-09-06

Final full regression: **1029 tests passed across 50 test/doc-test targets**.
This includes 17 new tests (12 kernel, four builtin, one bytecode), and all
29 argument-related kernel tests pass. OEX/server builds, scoped formatting and
whitespace checks pass. The new documentation example is also an executable
kernel regression test.

Changed paths for this extension are limited to `crates/openmat-bytecode`,
`crates/openmat-compiler`, `crates/openmat-runtime/src/arguments.rs`,
`crates/openmat-builtins`, and `crates/openmat-kernel/src/argument_blocks_tests.rs`.
Syntax/HIR already retained the required block attributes, so no parser or HIR
schema change was needed. Root manifests, accepted specs/RFCs and OEX ABI are
unchanged.

Strict Clippy passes for bytecode/compiler. The broader strict run is blocked by
pre-existing warnings in the checkout (including `unnested_or_patterns` from
the single-storage refactor, long functions, duplicate match arms and missing
error docs in unrelated runtime/builtin files). The broader check passes with
only these four lint categories allowed; this is not a claim of an unqualified
strict-Clippy pass. Unrelated implementation files were not changed to silence
these warnings.

Initial high-parallelism debug builds exhausted the Windows commit/pagefile
limit and C-drive space. Twelve just-generated test PDBs in the task-owned
temporary target were removed (2,979,590,013 bytes, reproducible by rebuilding).
Final full verification uses a separate E-drive target, two compiler jobs,
no incremental compilation and no debug symbols. No system settings, user
sources or live server processes are changed by this verification.

```powershell
$env:CARGO_PROFILE_TEST_DEBUG='0'
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_INCREMENTAL='0'
cargo build -q -j 2 -p openmat-oex -p openmat-server --target-dir C:/work/OpenMat/tmp/repeating-check-20260906
cargo test -q -j 2 --no-fail-fast -p openmat-syntax -p openmat-parser -p openmat-hir -p openmat-bytecode -p openmat-compiler -p openmat-runtime -p openmat-builtins -p openmat-kernel -p openmat-oex --target-dir C:/work/OpenMat/tmp/repeating-check-20260906
cargo clippy -q -j 2 -p openmat-bytecode -p openmat-compiler --all-targets --no-deps --target-dir C:/work/OpenMat/tmp/repeating-check-20260906 -- -D warnings
cargo clippy -q -j 2 -p openmat-bytecode -p openmat-compiler -p openmat-runtime -p openmat-builtins -p openmat-kernel --all-targets --no-deps --target-dir C:/work/OpenMat/tmp/repeating-check-20260906 -- -D warnings -A clippy::unnested_or_patterns -A clippy::too_many_lines -A clippy::match_same_arms -A clippy::missing_errors_doc
git diff --check
```

### Initial implementation, 2026-09-05

Previous verification on 2026-09-05: **1007 tests passed across 50 test/doc-test
targets**. Strict Clippy (`-D warnings`), scoped formatting, whitespace checks,
and the OEX/server build passed. Builds used the isolated target directory below.

OpenMat-authored probes ran in the installed MATLAB R2022b. Observations retained
only program-produced values/booleans, not MATLAB source or diagnostic messages.
They established string names, positional-only `nargin`, lazy/validated defaults,
case/prefix/duplicate option binding, scalar/vector shape conversion, rejection
of matrix broadcasting, and output validation outside the body catch scope.

Tests reside in `openmat-kernel/src/argument_blocks_tests.rs`,
`openmat-builtins/tests/argument_validation.rs` and
`openmat-bytecode/tests/argument_layout.rs`. Existing bytecode, parser, HIR,
compiler, runtime, builtins, kernel and OEX regressions are also run.

```text
cargo test -q --no-fail-fast -p openmat-syntax -p openmat-parser -p openmat-hir -p openmat-bytecode -p openmat-compiler -p openmat-runtime -p openmat-builtins -p openmat-kernel -p openmat-oex --target-dir C:/work/openmat-heap-frames-20260905
cargo clippy -q -p openmat-syntax -p openmat-parser -p openmat-hir -p openmat-bytecode -p openmat-compiler -p openmat-runtime -p openmat-builtins -p openmat-kernel -p openmat-oex --all-targets --no-deps --target-dir C:/work/openmat-heap-frames-20260905 -- -D warnings
cargo build -q -p openmat-oex -p openmat-server --target-dir C:/work/openmat-heap-frames-20260905
```

Scoped Rust files are formatted with `rustfmt --edition 2024` and checked with
`git diff --check`. No application server is restarted as part of these checks.

Changed paths are confined to `crates/openmat-syntax`, `openmat-parser`,
`openmat-hir`, `openmat-bytecode`, `openmat-compiler`, `openmat-runtime`,
`openmat-builtins`, `openmat-kernel`, and the bytecode fixtures in
`crates/openmat-oex/tests/c_plugin.rs`. Root manifests, accepted specs, the C ABI,
frontend files and unrelated user scratch files are unchanged.
