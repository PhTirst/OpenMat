# Dynamic workspace compatibility with MATLAB R2022b

OpenMat's first `eval` tranche parses, lowers, compiles, and executes source text
inside the caller's current language workspace. The implementation accepts a
character row vector or string scalar and is available through direct calls,
`feval('eval', ...)`, and `@eval` handles.

`evalin` and `assignin` use one explicit internal target model:

```rust
enum ScopeTarget {
    Current,
    Caller,
    Base,
}
```

Each ordinary language invocation receives a stable scope identity and a
snapshot of its immediate caller. This lets a write remain attached to the
correct suspended frame across nested calls and error handling. Dynamically
evaluated bytecode reuses the selected scope identity rather than introducing a
language-visible caller level.

## Accepted behavior

- A zero-output call executes statement text and writes assignments, newly
  introduced bindings, removals from `clear`, and the implicit `ans` binding
  back to the calling script or function workspace.
- A call with outputs treats the complete source as one right-hand-side
  expression. It suppresses an implicit `ans` side effect and forwards the
  requested output count.
- Local functions visible in the caller's linked source module remain callable
  from evaluated text.
- If execution fails, assignments completed before the failing operation remain
  visible to a surrounding `try`/`catch`.
- `eval(source, catch_source)` and
  `evalin(workspace, source, catch_source)` execute the catch source only when
  the primary source fails. `eval` runs both sources in the current workspace;
  `evalin` runs its primary source in the selected workspace but its catch source
  in the invoking function's current workspace. The catch source supplies the
  requested outputs.
- Catch-source text validation and compilation are lazy: a successful primary
  source does not inspect the unused catch value.
- `evalc(source[, catch_source])` captures command-window display and direct
  command text into its first char-row output. Additional requested outputs are
  returned from the selected primary or catch expression. Nested `evalc` calls
  capture independently.
- Captured display follows the active numeric and compact/loose format. Graphics
  notices, command-window clearing, and display-format changes remain session
  side effects instead of becoming text.
- `evalin('caller', source)` reads and writes the immediate caller function or
  script; at the entry boundary, `caller` resolves to the base workspace.
- `evalin('base', source)` addresses the session workspace even from inside
  nested `eval` execution.
- `assignin` accepts `base` and `caller`, validates the destination identifier,
  and performs the same copy-on-write/value-object language copy as ordinary
  assignment.
- `evalin` and `assignin` remain callable through `feval` and named function
  handles.
- `lasterr` and `lasterror` share one session-owned state. Setter calls return
  the previous state, `lasterr` resets the stack, and successful execution does
  not clear an earlier error.
- Catchable runtime failures update the state before a `catch` body or dynamic
  catch source starts. A newer failure replaces the older one, including a
  failure raised by the catch source itself.
- `lasterror` exposes the R2022b `message`, `identifier`, and column-structured
  `stack` fields. Registered source metadata supplies stack file names and
  one-based line numbers.

The clean-room `eval_current_workspace`, `scope_metaprogramming`,
`eval_catch_forms`, `evalc_capture`, and `last_error_state` conformance cases
cover these behaviors and pass both OpenMat and the locally installed MATLAB
R2022b oracle. The oracle observations contain only normalized results, not
MATLAB source, tests, documentation, or diagnostic text.

## Deferred boundary

Top-level parse and compile failures occur before the runtime invocation and do
not currently replace the session's last-error state. Function and class
definitions inside evaluated statement text are rejected; ordinary expressions
may still create anonymous function handles. Named functions used only inside
dynamic text are resolved lazily from the session current folder and MATLAB
search path. Evaluation inside a real source file preserves that file's
caller-private visibility. Dynamic script and class loading remains outside
this tranche.

Run the focused differential checks from the repository root:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -Tag eval `
    -ResultDirectory tests/conformance/reference/matlab-r2022b

pwsh -NoProfile -File tools/openmat-conformance/Invoke-OpenMatConformance.ps1 `
    -Case eval_current_workspace,scope_metaprogramming,eval_catch_forms,evalc_capture,last_error_state
```
