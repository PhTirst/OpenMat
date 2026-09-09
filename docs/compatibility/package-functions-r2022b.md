# Package functions, imports, and classes in MATLAB R2022b

This tranche implements MATLAB package functions and classes stored beneath
directories whose names begin with `+`, together with lexical `import`
resolution. The parent of the outer package directory is placed on the MATLAB
search path; the `+name` directory itself is not a path entry.

The behavior was measured against the locally installed MATLAB R2022b on
2026-09-02 using only OpenMat-authored black-box probe files. No MATLAB source,
tests, documentation, or diagnostic messages were copied.

## R2022b observations

- `+alpha/twice.m` is called as `alpha.twice(x)`.
- Nested directories such as `+alpha/+nested/inc.m` are called as
  `alpha.nested.inc(x)`.
- Qualified names work through direct calls, `feval`, named function handles,
  and `eval`.
- `which('alpha.twice')` returns the package function file, while
  `exist('alpha.twice', 'file')` returns zero in R2022b.
- A workspace value named `alpha` takes precedence over the package. Therefore
  `alpha.twice(x)` remains an ordinary field or method call when that value
  exists.
- A package function does not see an ordinary sibling package function by its
  unqualified leaf name. It must use the qualified name or an import.
- `import alpha.*` and `import alpha.Box` apply to the whole function scope,
  including expressions that textually precede the declaration. Base-workspace
  imports remain available to later evaluated commands. Imported function
  leaves also resolve through `feval('leaf', ...)` and `@leaf`.
- `clear import` is a command-workspace operation. It removes the complete
  import list without removing ordinary variables. Plain `clear` removes value
  bindings but leaves imports intact. R2022b rejects `clear import` inside a
  script or function file.
- When multiple imports provide the same leaf name, declaration order is used.
- A package class is constructed as `alpha.Box(...)`, reports the qualified
  class name, and exposes static methods and constant properties through that
  name. Imported class leaves are constructible in the same way.
- A bare expression such as `alpha.zero` invokes a zero-input package function.
  If `alpha` is a workspace value, ordinary field access still takes priority.
- Supplying a `+name` directory directly to `addpath` does not make it an
  effective search-path entry.

## OpenMat implementation

`MatlabSourceResolver` validates a dot-separated source name and maps every
leading component to a `+component` directory. Nested packages therefore use
one filesystem rule in the kernel linker, runtime module loader, `which`,
`feval`, handles, and evaluated code.

The compiler collects imports lexically per function, emits their ordered
qualified candidates, and preserves unresolved root ambiguity with dedicated
bytecode. The runtime resolves the root before evaluating call arguments. If
the root is a workspace value, class, or registered namespace, normal member
dispatch is used. Only a missing root falls back to package lookup. This also
preserves object methods and namespaces such as `ffi.*` across REPL requests.

The kernel linker resolves the longest callable prefix of a qualified name. It
normalizes a source file's leaf declaration (`Box`) to the package-visible name
(`alpha.Box`) while retaining leaf method names inside class metadata. This
allows the same instruction path to represent a package function, a package
class constructor, a static member, or a workspace-root field chain.

Package directories are filtered from direct search-path mutations, matching
the existing `genpath` exclusion rule.

The bytecode generation is version 26. Version 24 remains the compatibility
floor. Version-24 import and generalized package-member instructions remain
valid; version-25 adds mutable-import and external-class-method metadata, and
version-26 adds argument-sensitive bare-call targets for old-style method
dispatch. A module is rejected when it uses an instruction introduced after
its declared version.
