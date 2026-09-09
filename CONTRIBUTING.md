# Contributing to OpenMat

OpenMat targets Windows x64 first and MATLAB R2022b base-language semantics.
Read the relevant specification and accepted RFC before changing observable
language or protocol behavior. For interface changes, explain the proposed
contract and compatibility impact in an issue or pull request before coding.

## Development

Follow the root README and [local development guide](docs/guides/windows-local-development.md).
Keep pull requests focused and include the problem, resulting behavior, checks
run, and known limitations. Add regression coverage for behavior changes.

Run the relevant subset locally; CI checks the main Rust workspace, Web client,
and independent Desktop workspace:

```powershell
node --test tools/release/check-public-source.test.mjs
node tools/release/check-public-source.mjs
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo test --locked --workspace
pwsh -NoProfile -File tools/openmat-conformance/tests/Test-Comparator.ps1
pwsh -NoProfile -File tools/openmat-conformance/tests/Test-RunnerExitCodes.ps1
cargo build --locked -p openmat-cli -p openmat-server
pwsh -NoProfile -File tools/openmat-conformance/Invoke-OpenMatConformance.ps1 -OpenMatPath .\target\debug\openmat-cli.exe -JsonSummary
pwsh -NoProfile -File tools/openmat-dev/tests/Test-RuntimeSettings.ps1
pwsh -NoProfile -File tools/openmat-dev/tests/Test-OpenMatDev.ps1 -ServerPath .\target\debug\openmat-server.exe
pnpm --dir apps/web typecheck
pnpm --dir apps/web test
pnpm --dir apps/web build
pwsh -NoProfile -File tools/release/Build-OpenMatWindows.ps1 -PrepareOnly -DisableCompilerCache
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --all-targets -- -D warnings -D clippy::pedantic
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features tauri/custom-protocol
pwsh -NoProfile -File tools/release/Test-OpenMatDesktop.ps1 -Executable .\apps\desktop\src-tauri\target\debug\openmat-desktop.exe
```

Run dependency installation from the README before these commands. Desktop
checks need the prepared native resources, and its embedded frontend build
needs `pnpm --dir apps/web build` first. The smoke command above uses Cargo's
default Desktop target path; when `CARGO_TARGET_DIR` is set, use
`<CARGO_TARGET_DIR>/debug/openmat-desktop.exe` instead. CI shares one target
directory across both workspaces and disables incremental compilation and
debug symbols in development/test artifacts to keep disk use bounded.
CI also reruns the OpenBLAS provider tests with the prepared DLL selected by
`OPENMAT_TEST_OPENBLAS_DLL` and `OPENBLAS_NUM_THREADS=1`, so those checks exercise
the actual downloaded runtime instead of the optional no-DLL path.
The Windows job requires GCC for the real C plugin fixtures. Before launching
the desktop smoke test, it checks for the WebView2 Runtime and, if absent, uses
Microsoft's [documented Evergreen installation procedure](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#online-only-deployment).

## Source and data provenance

Submit work you have the right to contribute under `AGPL-3.0-only`, the project's
license. Retain existing copyright and license notices. Contributors retain
copyright in their contributions.

Do not commit third-party library source, downloaded archives, native binaries,
package caches, generated bundles, personal workspace files, or credentials.
Use manifests and lockfiles for dependencies. Preserve upstream notices and
document new native dependency downloads with their version and SHA-256.

Write compatibility tests independently. The optional MATLAB harness runs
project-authored programs against a separately licensed installation and
retains normalized values and error categories, not proprietary implementation
files, documentation, tests, or diagnostic prose. Explain the origin of new
fixtures; generated local logs are not reference fixtures.

Before committing, inspect the exact staged changes and run:

```powershell
node tools/release/check-public-source.mjs --staged
git diff --cached --check
git diff --cached
```

The automated check finds known risky paths and credential patterns. It does
not replace reviewing new files and their provenance.
