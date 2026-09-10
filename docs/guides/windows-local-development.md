# Windows local development launcher

`tools/openmat-dev/Invoke-OpenMatDev.ps1` starts the native Rust kernel server
and the Vite web client as one owned local process group. It builds
`openmat-server` and the Plot Engine WebAssembly package, asks the server to
listen on the configured loopback port (initially `127.0.0.1:42000`), reads the
WebSocket URL from the server's first stdout line, and supplies that URL only to
the Vite child as `VITE_OPENMAT_WS_URL`. The same server exposes the independent
`openmat-workspace-v1` API at `/workspace/v1`, rooted in an explicitly selected
existing directory. The repository root is the local-development default.

## Prerequisites

- Windows 10 or Windows 11.
- PowerShell 7 (`pwsh`). Windows PowerShell 5.1 is not supported.
- The repository's pinned Rust toolchain and `cargo` on `PATH`.
- The `wasm32-unknown-unknown` Rust target and the lock-compatible
  `wasm-bindgen` CLI:

  ```powershell
  rustup target add wasm32-unknown-unknown
  cargo install wasm-bindgen-cli --version 0.2.127 --locked
  ```

- Node.js 20.19 or newer and `pnpm` on `PATH`.
- Existing `apps/web` dependencies. The launcher does not run `pnpm install`. Cargo and the compiler-cache
  initializer may download build dependencies. Prepare the checkout separately if
  `apps/web/node_modules` is absent.

MATLAB is not needed by the launcher or its smoke test.

## Start the server and web client

From the repository root:

```powershell
pwsh -NoProfile -File .\tools\openmat-dev\Invoke-OpenMatDev.ps1
```

Select another existing workspace root with:

```powershell
pwsh -NoProfile -File .\tools\openmat-dev\Invoke-OpenMatDev.ps1 `
  -WorkspaceRoot C:\work\my-openmat-project
```

The normal launcher shares `%APPDATA%\org.openmat.desktop\runtime.json` with
Desktop. Edit its `kernelPort` and restart to persistently change the port.
Use `-RuntimeConfig C:\work\openmat-runtime.json` or `OPENMAT_RUNTIME_CONFIG`
to select another file. Missing configuration files are created once from the
[default settings](../../apps/desktop/runtime.default.json); invalid settings
or occupied ports cause an error instead of selecting another port.

The command prints the settings file, log directory, actual kernel URL, and the
Vite local URL. It waits for both the server URL and Vite's ready line with
bounded timeouts, then continues monitoring both processes.

Both this launcher and `pnpm --dir apps/web dev` build the Plot Engine WASM in
optimized Release mode, matching Desktop's renderer. The React frontend still
uses Vite's development server and hot reload. Unoptimized WASM makes dense 3D
plots noticeably slower to rotate, so Debug mode is reserved for debugging the
renderer itself:

```powershell
pnpm --dir apps/web build:plot-wasm:debug
# Start Vite with the existing WASM and a separately running native server.
pnpm --dir apps/web dev:ready
```

Use `pnpm --dir apps/web build:plot-wasm` to restore the optimized renderer. The
next normal launcher run also restores it automatically. `build:plot-wasm:release`
remains an explicit alias for the optimized build.

To reuse a server already built by CI or a previous local build:

```powershell
pwsh -NoProfile -File .\tools\openmat-dev\Invoke-OpenMatDev.ps1 `
  -ServerPath .\target\debug\openmat-server.exe
```

Server and frontend output are kept separate:

- `server.stdout.log` and `server.stderr.log`
- `frontend.stdout.log` and `frontend.stderr.log`
- `cargo-build.stdout.log` and `cargo-build.stderr.log` when a build runs
- `plot-wasm-build.stdout.log` and `plot-wasm-build.stderr.log` when the web
  client is started

The development launcher enables lightweight execution timing lines in
`server.stderr.log`. Lines beginning with `OPENMAT_PERF` separate kernel
source-to-bytecode compilation, VM execution, and graphics publication time.

By default the command creates a unique directory below
`$env:TEMP\openmat-dev` and prints its full path. Use `-LogDirectory` to select
another location.

## Real WebSocket smoke test

The non-interactive mode starts a real server but does not start Vite:

```powershell
pwsh -NoProfile -File .\tools\openmat-dev\Invoke-OpenMatDev.ps1 -Smoke
```

This explicit test mode uses an ephemeral port and leaves user settings untouched.
Pass `-RuntimeConfig <file>` with `-Smoke` to test a configured fixed port instead.
It uses .NET `ClientWebSocket` and one bounded smoke lifecycle. The kernel client
requires the `openmat-kernel-v0` startup event, then performs `initialize`, scalar
`while`/`disp` execution, `listWorkspace`, `inspect`, `shutdown`, and the normal
WebSocket close handshake. It checks `sessionId`, unique server `messageId`
values, every response `replyTo`, result types, workspace summaries, the
inspected scalar value `40`, close code 1000, UTF-8 text frames, and a 1 MiB
per-message limit. A second client connects to `/workspace/v1`, lists the
configured root, creates a uniquely named file and directory without overwrite,
lists again, verifies both objects on disk, and removes only those exact smoke
objects locally. The smoke test does not use array features or MATLAB.

Reuse a built binary to avoid rebuilding:

```powershell
pwsh -NoProfile -File .\tools\openmat-dev\Invoke-OpenMatDev.ps1 `
  -Smoke `
  -ServerPath .\target\debug\openmat-server.exe
```

Run the helper and lifecycle self-tests with:

```powershell
pwsh -NoProfile -File .\tools\openmat-dev\tests\Test-OpenMatDev.ps1
```

The self-test covers repository/server path guards, announced-URL validation,
startup timeout cleanup, invalid-startup cleanup, descendant process cleanup,
frontend child environment/readiness helpers, and the real server WebSocket
lifecycle. It intentionally does not automate the indefinitely running Vite UI
lifecycle; the production launcher separately checks Vite's ready output and
monitors its process after startup.

## Exit and cleanup

Press Ctrl-C in the launcher's terminal. Normal exit, Ctrl-C, a startup timeout,
and a child failure all run the same cleanup path.

Cleanup uses .NET `System.Diagnostics.Process.Kill(true)` only on process objects
recorded by this launcher. That closes each recorded process and its descendant
tree (for example, the Node process below `pnpm.cmd`). It does not enumerate or
kill unrelated `cargo`, `node`, server, or Vite processes.

Logs are retained after cleanup. No port is fixed or searched by process name.

Stable nonzero exit codes are:

| Code | Meaning |
| ---: | --- |
| 2 | invalid repository, executable, log path, or protected command argument |
| 3 | required native command is unavailable |
| 10 | Cargo launch or build failure |
| 11 | Cargo build timeout |
| 12 | Plot Engine WASM launch, build, or artifact failure |
| 13 | Plot Engine WASM build timeout |
| 20 | server launch or early-exit failure |
| 21 | server URL announcement timeout |
| 22 | invalid server URL announcement |
| 23 | server exited after successful startup |
| 30 | WebSocket or kernel protocol mismatch |
| 31 | overall smoke timeout |
| 40 | pnpm/Vite launch or early-exit failure |
| 41 | Vite readiness timeout |
| 42 | Vite exited after successful startup |
| 130 | Ctrl-C cancellation when PowerShell surfaces it to the script |

## Parameters and environment

Command-line parameters take precedence over the corresponding environment
variables.

| Purpose | Parameter | Environment variable |
| --- | --- | --- |
| reuse a built server | `-ServerPath` | `OPENMAT_SERVER_PATH` |
| select an existing workspace root | `-WorkspaceRoot` | `OPENMAT_WORKSPACE_ROOT` |
| select Cargo | `-CargoPath` | `OPENMAT_CARGO` |
| select pnpm (`.cmd` or `.exe`) | `-PnpmPath` | `OPENMAT_PNPM` |
| select the log directory | `-LogDirectory` | `OPENMAT_DEV_LOG_DIR` |

`CARGO_TARGET_DIR` is honored when locating the binary produced by the default
debug build. `VITE_OPENMAT_WS_URL` is set or replaced only in the new Vite
process environment; the parent PowerShell environment is unchanged.

The build, server startup, frontend startup, and smoke timeout parameters can be
adjusted independently with `-BuildTimeoutSeconds`,
`-ServerStartupTimeoutSeconds`, `-FrontendStartupTimeoutSeconds`, and
`-SmokeTimeoutSeconds`.

## Local security boundary

This development transport binds only to `127.0.0.1` and exposes the `/kernel`
and `/workspace/v1` WebSocket endpoints. It uses plain `ws://`: there is no TLS
and no authentication or authorization. Any local process able to connect can
submit kernel requests or create files and directories directly below the
configured workspace root.

Treat it as a single-user development service, not as a network service. Do not
forward the port, bind it to a non-loopback interface, or expose it through a
proxy. Origin handling is not an authentication boundary, and this guide makes
no security claim for non-browser clients or for browser Origin behavior beyond
the tested local development flow.
