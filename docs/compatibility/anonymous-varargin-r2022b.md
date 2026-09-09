# Anonymous `varargin` compatibility with MATLAB R2022b

This record covers the clean-room probes and differential cases for `varargin`
in anonymous function parameter lists. Probes were authored locally and run in
the installed MATLAB R2022b without copying MATLAB source, tests,
documentation, or diagnostic messages.

## Observed boundary

- A final parameter named exactly `varargin` is a variadic tail. `@(varargin)`
  accepts zero or many inputs, and `@(head, varargin)` requires the fixed prefix
  before packing remaining inputs.
- With no variadic inputs, `varargin` is a `0 x 0` cell array. With inputs it is
  a `1 x N` cell row vector in call order.
- `varargin{index}` selects cell contents and `varargin{:}` expands the
  comma-separated contents.
- `nargin` inside the anonymous body reports the actual call input count.
- A nonfinal parameter named `varargin`, including `@(varargin, tail)`, is an
  ordinary fixed parameter rather than a variadic tail. The spelling remains
  case-sensitive.
- Repeated anonymous parameter names are rejected. This is already represented
  by the parser's duplicate-parameter diagnostic and remains independent of the
  variadic ABI.
- Anonymous `varargin` composes with construction-time value capture, nested
  anonymous functions, parameter shadowing, and forwarding of the caller's
  requested output count from a top-level body call.

## OpenMat implementation boundary

The compiler uses the existing named-function variadic ABI: a final anonymous
`varargin` reduces `Function::parameter_count` to the fixed prefix length and
emits `LoadVariadicInputs` as the closure function's first executable
instruction. The resulting cell is stored in the ordinary local slot allocated
for the parameter. Closure construction and nested capture continue to use
`MakeClosure`; the runtime does not inspect the parameter name.

The shared `LoadVariadicInputs` runtime path now constructs MATLAB-compatible
`0 x 0` empty cells and `1 x N` populated cells, so the correction applies to
both named and anonymous variadic functions. No bytecode opcode or serialized
format change is required.

## Differential cases

| Case | Compatibility boundary |
| --- | --- |
| `function_handle_anonymous_varargin_basic` | Zero/many inputs, cell shape, cell indexing, expansion, and actual `nargin`. |
| `function_handle_anonymous_varargin_positions` | Final-name activation and nonfinal fixed-parameter behavior. |
| `function_handle_anonymous_varargin_nested_capture` | Value capture, both nesting directions, and shadowing. |
| `function_handle_anonymous_varargin_requested_outputs` | Direct and nested requested-output forwarding. |

The MATLAB oracle selection can be regenerated from the repository root with:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -Tag function-handle-anonymous-varargin-r2022b `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

The matching OpenMat selection is:

```powershell
pwsh -NoProfile -File tools/openmat-conformance/Invoke-OpenMatConformance.ps1 `
    -Case 'function_handle_anonymous_varargin_*'
```

## Remaining limitation

Like the pre-existing OpenMat function-call ABI, bytecode function-handle calls
validate the fixed prefix arity before execution. MATLAB can defer a missing
fixed anonymous parameter until the body reads it, so a handle whose unused
fixed parameter is omitted is outside this compatibility claim.

The accepted expansion case covers the common forwarding form
`target(varargin{:})`. Direct comma-separated-list splicing inside a matrix
literal, such as `[varargin{:}]`, remains outside this claim because the
existing `BuildMatrix` bytecode accepts single values rather than pack sources.
