# Bytecode v27: optional inputs and dynamic output packs

The compiler emits v27. The verifier still accepts v24-v26 modules using their
existing instruction surfaces and rejects v27 instructions in older modules.
No C/OEX ABI, kernel wire protocol, or accepted RFC is changed.

## Inputs

`Function::parameter_count` counts declared fixed parameter slots, not required
inputs. Calls may omit any trailing fixed inputs, including the fixed prefix of
a function declaring `varargin`. Actual arguments alone determine `nargin`.
An omitted parameter is `Nothing` in its local slot, not an empty MATLAB array.
The variadic tail is an empty 0-by-0 cell when no extra inputs were supplied.

`RequireDefined` checks an actual language read after loading a local/capture.
Raw loads remain useful for `exist`, initializing an indexed variable, capturing
a missing binding, and collecting unrequested outputs. Supplying too many inputs
to a non-variadic function is still rejected before entering its body.

## Outputs and assignment

- `CountPlaceOutputs` counts final cell-content/struct-field destinations from
  evaluated selectors without reading the final contents or field. It consumes
  the compiler-owned shallow root snapshot used for counting.
- `ApplyOutputPack` takes the requested count from a scalar register. Its target
  distinguishes ordinary application, member application, and qualified calls.
  Results are stored directly in a pack, without temporary cell repacking.
- `SlicePack` distributes zero-based ranges to individual LHS destinations.
- `RequireSinglePack` rejects a non-scalar intermediate assignment prefix when
  resolving a nested `end`; it never silently chooses the first child.

Ordinary name-only multi-assignment retains its existing fixed-register path.
For bracketed aggregate destinations, all-fixed parenthesized targets evaluate
the RHS before evaluating LHS selectors. If any destination can expand, all LHS
selectors are evaluated before the RHS, once, to compute the requested output
count. Writes then proceed left-to-right using freshly loaded roots, preserving
RHS mutations and earlier writes to the same root.

These order observations were checked with locally authored probes in installed
MATLAB R2022b: `[a(mark(1)),a(mark(2))]=pair()` traces RHS,1,2; adding a brace
destination moves all selectors before the RHS. The committed runtime tests
assert these traces, repeated roots, empty selections, growth, column-major
selection order, missing inputs, exceptions, and heap-backed recursive calls.

Module linking relocates all new ordinary registers, pack registers, and member
name constants, including nested path operands.

Implementation paths: `openmat-bytecode/src/{model,verify,lib}.rs`,
`openmat-compiler/src/lowering.rs`, `openmat-runtime/src/{interpreter,array_ops}.rs`,
and the narrow integration point `openmat-kernel/src/module_linker.rs`.
Regression tests live in `openmat-runtime/tests/language_arguments_assignment.rs`,
`openmat-bytecode/tests/dynamic_outputs.rs`, the compiler integration suite,
and the linker's unit tests.

## Remaining boundaries

This change does not implement `arguments`, `Name=Value`, custom `subsref` or
`subsasgn`, heterogeneous object assignment, table brace assignment, or general
numeric deletion. Existing matrix-literal comma-list concatenation limitations
are also separate from distributing RHS packs to LHS destinations.

```text
cargo build -p openmat-oex -p openmat-server --target-dir <isolated-target>
cargo test -p openmat-bytecode -p openmat-compiler -p openmat-runtime -p openmat-kernel --target-dir <isolated-target>
cargo clippy -p openmat-bytecode -p openmat-compiler -p openmat-runtime --all-targets --no-deps --target-dir <isolated-target> -- -D warnings
```
