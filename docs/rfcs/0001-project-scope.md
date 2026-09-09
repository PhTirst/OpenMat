# RFC 0001: Project scope and release-one boundary

Status: Accepted

## Decision

OpenMat implements a clean-room MATLAB-like language with MATLAB R2022b base
language behavior as the compatibility target. The reference
version is MATLAB R2022b 9.13, invoked only from a separately licensed local installation.

Release one includes dense numerical arrays, scripts, functions, multiple
outputs, control flow, indexing, core linear algebra, a command-line interface,
a REPL, and the core `classdef` subset specified separately.

Compatibility means matching observable language semantics: accepted syntax,
value class, shape, indexing, dispatch, normal results, and error conditions.
It does not require identical diagnostics, display text, performance, or
bit-for-bit floating-point results across numerical providers.

## Explicit non-goals for release one

- JIT compilation.
- Simulink or compatibility with proprietary MATLAB toolboxes.
- Sparse, GPU, distributed, or symbolic arrays.
- MEX compatibility.
- A pure-browser WebAssembly runtime.
- Full MATLAB graphics and desktop GUI compatibility.
- The advanced `classdef` features excluded by the class-system specification.

## Licensing and clean-room rule

Project code in this public release is licensed under GNU AGPL version 3 only
(`AGPL-3.0-only`); see the root `LICENSE`. Compatibility
tests are authored by the OpenMat project. MATLAB R2022b may be invoked as a
black-box oracle on the local licensed installation; proprietary implementation
files, tests, documentation, and diagnostic text are not copied into this
repository.
