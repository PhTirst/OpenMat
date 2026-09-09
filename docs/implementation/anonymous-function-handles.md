# Anonymous function handles

Milestone 6 supports MATLAB-style anonymous function expressions of the form
`@(parameters) expression`. This tranche extends the named-handle baseline; it
does not add a JIT or change unresolved parenthesized call-versus-index syntax.

## Frontend and HIR

The lossless CST uses `AnonymousFunctionExpr` with an explicit `ParameterList`
child and an expression-body child. The HIR retains the ordered parameter
names, expression body, and complete source span as
`ExprKind::AnonymousFunction`. Missing bodies and closing parentheses recover
to a tree with stable parser diagnostics. Duplicate parameters are diagnosed
both by the source parser and by checked HIR compilation.

The compiler rejects anonymous `varargin` parameters with a structured
unsupported-feature diagnostic. Variadic anonymous invocation requires cell
and comma-separated-list semantics that are outside this tranche; treating the
name as one ordinary input would be silently incompatible.

## Capture timing

Construction captures the current lexical or workspace value. Parameters
shadow free variables. Every captured value passes through the interpreter's
existing `language_copy` boundary, so arrays preserve copy-on-write behavior,
value-class objects are copied, and handle-class objects retain identity.

Two locally authored MATLAB R2022b probes established the timing boundary with
custom result markers only:

- a free value or callable that does not exist at construction remains
  unavailable even if a same-named workspace binding is created before call;
- a value or callable that exists at construction retains its construction-time
  value after the original workspace binding changes.

Runtime capture entries therefore have three states:

- `Value`: an owned construction-time language copy;
- `MissingValue`: a value reference that was absent at construction and must
  remain absent;
- `ResolverOnly`: an absent call target that skips future workspace bindings
  but may use a linked function, registered class, intrinsic, or built-in.

This prevents built-ins, local functions, and source-path functions from being
mistaken for ordinary variable values while preserving workspace-variable
precedence when such a value actually exists at construction.

## Bytecode and verification

Bytecode version 9.0 adds `MakeClosure`. It names the closure-body function and
encodes ordered capture tuples containing the free-variable string constant,
an optional statically resolved enclosing register, and the missing-value
policy. The verifier checks the destination, closure function, every name
constant, every present register, and duplicate capture names.

Version 9.0 also adds `ReturnApply`. When an anonymous body is a top-level
ordinary unresolved application, this instruction forwards the closure call's
dynamic requested-output count and returns the callee's outputs unchanged.
This covers calls such as `@(x) size(x)` and source functions with multiple
outputs without fixing the body call to one output.

## Runtime ownership and linking

Each runtime bytecode handle owns an `Arc<BytecodeModule>`, a `FunctionId`, and
an owned capture environment. Handles remain callable after a kernel request
installs another module and while they flow through arguments, returns, and the
persistent workspace. Session reset clears the workspace and retained function
code.

Registry-issued handle IDs use the high half of the `u32` space. The legacy
direct-function-index fallback is restricted to the low half, so a registered
closure cannot collide with a direct index.

The kernel linker remaps `MakeClosure` function IDs and capture constants when
merging support modules. Anonymous bodies use the existing resolver and source
origins: local functions, the calling file's directory, configured search
paths, classes, intrinsics, and built-ins retain their established precedence.
Linking constructs no handle target and executes no target body.

## Deliberate boundaries

An anonymous function still has one expression body. Statement bodies,
assignments in the body, and variadic `varargin` forms are rejected by stable
parser or compiler diagnostics. Dynamic multi-output forwarding applies when
the top-level body is an ordinary parenthesized application. A top-level member
application currently follows the ordinary one-result expression lowering;
requesting additional outputs produces the existing structured missing-output
runtime error rather than silently fabricating values.
