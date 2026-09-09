# OpenMat contributor and agent guidance

Read `CONTRIBUTING.md` before making changes. Treat accepted specifications in
`spec/` and accepted RFCs in `docs/rfcs/` as contracts.

- Keep changes within the requested scope and preserve unrelated work.
- Propose shared language/protocol changes explicitly before implementing them.
- Preserve MATLAB R2022b compatibility decisions; do not infer them from Octave alone.
- Differential tests may execute OpenMat-authored programs in a separately
  licensed MATLAB installation. Do not copy MATLAB source, tests, documentation,
  or diagnostic prose.
- Do not commit dependency source, binaries, credentials, generated outputs, or
  local development records. Run the public-source check before committing.
- Keep Windows-specific behavior behind a narrow platform boundary.
- Run relevant formatting, tests, and checks. Report commands, outcomes, and
  remaining limitations without representing unrun checks as passing.
- Make focused commits when the task calls for commits.

## Architectural invariants

- Handwritten lexer and parser with a lossless, error-tolerant syntax layer.
- AST/HIR preserves unresolved call-versus-index ambiguity where required.
- No JIT in the first release; source lowers to register-oriented bytecode.
- Numerical arrays are column-major, one-based at the language boundary, and
  copy-on-write.
- The web client talks to a native server/kernel; the VM does not execute in
  WebAssembly in the first release.
- Rust ABI is never a plugin or process boundary. Stable boundaries use C ABI
  or versioned serialized protocols.
