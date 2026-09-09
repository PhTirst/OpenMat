# Public source and dependency policy

The public repository contains first-party source, tests, documentation,
dependency manifests and lockfiles, build scripts, and necessary license texts.
It starts a new development history. Earlier private development records are
not part of this repository.

## Dependencies are obtained during the build

| Dependency surface | Source of truth | Generated location |
| --- | --- | --- |
| Rust workspace and Plot WASM | `Cargo.lock` | Cargo cache and `target/` |
| Tauri desktop | `apps/desktop/src-tauri/Cargo.lock` | Cargo cache and desktop `target/` |
| React frontend | `apps/web/pnpm-lock.yaml` | pnpm store and `node_modules/` |
| Windows OpenBLAS LP64 runtime | version, URL, and SHA-256 in `Build-OpenMatWindows.ps1` | `.openmat/` and ignored Tauri resources |
| HDF5 and zlib | locked Cargo packages and their build features | Cargo cache and build output |

`crates/openmat-openblas` is OpenMat's ABI/provider integration code, not a copy
of OpenBLAS. Keep it in the source repository. OpenBLAS's actual library is
downloaded and verified when preparing a Windows installer.

Use locked Cargo commands and `pnpm install --frozen-lockfile`. Dependency source
may be downloaded into local caches to compile the project; those caches must
never be committed. This also applies to plugin and toolbox build directories.

## License materials and binary releases

The first-party license is the root `LICENSE` (`AGPL-3.0-only`). Upstream license
texts in `tools/release/licenses/` retain their original terms; they are
attribution material, not vendored implementation code.

`Generate-ThirdPartyLicenses.ps1` resolves the actual desktop, WASM, and web
dependency graphs. It generates notices, a dependency manifest, exact upstream
license texts, an MPL source archive/availability notice, and a copy of OpenMat's
AGPL license for the installer. These outputs belong in ignored build staging
and release artifacts, not the Git source tree.

A source-only Git policy does not remove obligations when distributing binaries
or operating modified network services. Before publishing a binary release,
provide the applicable Corresponding Source and build/install material for that
exact version, including required dependency source. Keep these release source
archives separate from Git if needed. Preserve the generated third-party
artifacts, and provide users an appropriate source-access route where AGPL
requires it. A GitHub URL alone is not a substitute for checking that available
source corresponds to the binary. The current Windows packaging script
generates notices and MPL sources; it does not yet assemble a complete AGPL
Corresponding Source release bundle.

See the [AGPL text](../../LICENSE), particularly sections 1, 6, and 13, for the
actual terms. A project's own license does not relicense its dependencies.

## Public-source guard

To export a reviewed commit without development history or ignored build caches,
run this from a clean checkout (choose an output path outside the repository):

```powershell
pwsh -NoProfile -File tools/release/Export-PublicSnapshot.ps1 -OutputPath ../OpenMat-source.zip
```

The script checks the committed source and archives only the latest Git tree.
For an initial public release, extract it into a new directory and initialize a
fresh Git repository there. Do not push a private staging repository's history.
The archive contains no `.git` directory, parent commits, or commit identifier in
the ZIP comment. `-Force` explicitly replaces an existing export.

`node tools/release/check-public-source.mjs` inspects tracked file contents.
Use `--staged` to inspect the index before a commit, or `--snapshot` to inspect
a source export without Git metadata. The check rejects generated/vendor paths,
common binary payloads and credential patterns, machine-specific private paths,
stale proprietary declarations, and missing package license metadata.
It reports file names and rule names without printing credential values.

Keep personal audit reports, migration logs, and initial-commit helper scripts
outside the public repository.
