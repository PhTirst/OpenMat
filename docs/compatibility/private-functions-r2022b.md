# Caller-private function loading in MATLAB R2022b

OpenMat measured the relevant lookup behavior against the locally installed
MATLAB R2022b on 2026-09-02 using only OpenMat-authored probe files. The probe
placed an ordinary function and a same-named function under `private` and
observed which implementation supplied a distinct numeric result.

## R2022b observations

- The command window resolves the ordinary function in the current folder; it
  cannot use the current folder's `private` directory directly.
- A function or script stored in the directory immediately above `private`
  resolves the private function before an ordinary same-named sibling.
- Direct calls, `feval`, named function handles, and `eval` retain this caller
  visibility. `which` reports the private file and `exist(name, 'file')`
  reports a MATLAB file.
- A function inside `private` can call another function in that same private
  directory.
- A caller in a child directory does not inherit the parent directory's
  private visibility.
- R2022b warns and does not add a `private` directory supplied directly to
  `addpath`; OpenMat likewise omits such a directory from effective path state.

## OpenMat architecture

`MatlabSourceResolver` is the single filesystem rule for caller-private,
caller-directory, current-folder, and search-path precedence. Both the kernel
pre-linker and runtime services use it, so compile-time and lazy lookup cannot
silently disagree about private visibility.

The runtime `ModuleLoader` owns lazy function-file loading. It resolves the
source, parses and lowers it, compiles verified bytecode, registers source
metadata, and caches the result by canonical path plus exact source contents.
A changed file at the same path is compiled again; a path-order mutation is
observed because resolution occurs before the cache is consulted.

Script instructions retain their own source location after inlining, so a
script uses the private directory associated with the script file rather than
the source file that happened to invoke it.

## Deferred boundary

Classdef files in `@Class` directories and their declared sibling method files
use the same caller-contained resolver. Pre-classdef objects created through
`class(structValue, 'ClassName')` remain separate future work. Package functions
and package `classdef` files use the package resolver described in
`package-functions-r2022b.md`. Dynamically discovered scripts are still handled
by the kernel's script linker rather than the function-only `ModuleLoader`.
