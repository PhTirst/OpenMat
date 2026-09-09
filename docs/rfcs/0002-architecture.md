# RFC 0002: Compiler, runtime, and product architecture

Status: Accepted

## Compiler pipeline

```text
UTF-8 source
  -> lossless lexer
  -> error-tolerant concrete syntax tree
  -> abstract syntax tree
  -> name and scope analysis
  -> high-level IR
  -> register-oriented bytecode
  -> interpreter
```

The parser is handwritten. Expression parsing uses Pratt-style precedence;
declarations and statements use recursive descent. Source spans are byte ranges
and every diagnostic carries a source identifier and range.

The syntax tree does not prematurely decide whether `f(x)` is a function call or
an array/object indexing operation. That distinction can require name resolution
or runtime state and is represented as an unresolved parenthesized application
until lowering or execution.

## Runtime

The language workspace executes user code sequentially. Server I/O and control
messages may be asynchronous. Interrupt requests use a separate control path and
a cooperative cancellation flag; the server may terminate and restart a kernel
that is stuck inside native code.

All numerical values follow MATLAB's column-major and one-based language
semantics. Internal offsets are zero-based and checked. Runtime indices use 64
bits. LP64 BLAS calls reject dimensions that do not fit their integer ABI.

## Product processes

```text
Browser or Tauri webview
        <-> HTTP/WebSocket
Session server
        <-> versioned kernel protocol
Per-session OpenMat kernel
        -> compiler/runtime
        -> numerical provider
```

The browser never receives whole large matrices by default; it requests bounded
previews. Desktop packaging reuses the web UI and starts local server/kernel
sidecars.

## Crate dependency direction

Low-level compiler and array crates must not depend on server or UI crates.
`openmat-array` is independent of dynamic runtime values. `openmat-value` may
wrap arrays. The object system may depend on values, and the runtime composes
bytecode, values, arrays, and objects. Kernel and application crates sit at the
top of the graph.

