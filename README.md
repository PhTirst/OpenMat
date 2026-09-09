# OpenMat

OpenMat is an independently developed numerical programming environment with a
MATLAB-like language, a browser IDE, and a Windows desktop application. Its
compatibility target is the MATLAB R2022b base language; compatibility is
incomplete and is defined by the checked-in specifications and tests.

The Rust server runs the compiler, bytecode interpreter, and numerical kernel.
The React client provides editing, language services, workspace inspection,
figures, and a drag-and-drop App Designer backed by M-language classes. Rust
compiled to WebAssembly handles plot rendering; the language VM runs natively.
The desktop application embeds this frontend using Tauri.

## Start developing

The first supported host is Windows x64. Install PowerShell 7, Visual Studio
Build Tools with the C++ workload and Windows SDK, CMake, Rust/rustup, Node.js
22.19.0, and pnpm 10.30.3. The repository pins Rust 1.90.0.

From the repository root:

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
pnpm --dir apps/web install --frozen-lockfile
pwsh -NoProfile -File tools/openmat-dev/Invoke-OpenMatDev.ps1
```

The launcher starts the native server and web client. MATLAB is not required
to build or run OpenMat. See the [development guide](docs/guides/windows-local-development.md)
for workspace selection, smoke tests, and build options, and
[desktop build instructions](apps/desktop/README.md) for the Windows installer.

## Repository

- `crates/`: compiler, runtime, numerical providers, server, LSP, and plotting.
- `apps/web/`: React IDE and App Designer.
- `apps/desktop/`: Tauri desktop shell.
- `sdk/`, `include/`, `plugins/`, `toolboxes/`: extension interfaces and examples.
- `spec/` and `docs/rfcs/`: language and protocol contracts.
- `tests/conformance/`: OpenMat-authored tests and normalized reference data.
- `tools/`: development, compatibility, and release tooling.

Some design documents describe earlier development milestones rather than
current completeness. Tests and accepted contracts are the source of truth.

## Checks and contributions

```powershell
node tools/release/check-public-source.mjs
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo test --locked --workspace
pnpm --dir apps/web typecheck
pnpm --dir apps/web test
pnpm --dir apps/web build
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development and source-provenance rules.
The [conformance harness](tests/conformance/README.md) compares OpenMat results
with project-owned manifests and normalized black-box observations. The
optional MATLAB oracle requires a separately licensed R2022b installation;
public CI does not invoke MATLAB.

## License and dependencies

OpenMat's first-party code, documentation, and project-authored tests are
licensed under **GNU Affero General Public License version 3 only**
(`AGPL-3.0-only`), unless a file explicitly states otherwise. See [LICENSE](LICENSE).
Contributors retain their respective copyrights.

This Git repository does not vendor third-party library source or binaries.
Cargo, pnpm, and the release scripts obtain dependencies during builds using
lockfiles and pinned checksums. Third-party components retain their own
licenses; their license texts and attribution are not relicensed as OpenMat.
See [dependency and release policy](docs/guides/public-source-and-dependencies.md).

OpenMat is an independent project and is not affiliated with or endorsed by
MathWorks. MATLAB is a trademark of The MathWorks, Inc.
