# OpenMat Desktop

The Windows desktop application is a Tauri 2 shell named **OpenMat**. It keeps
the browser development workflow intact, embeds the production Web build, and
links the native server and kernel into `OpenMat.exe`. The installed application
therefore has one application executable; its in-process server listens only on
a private loopback port and loads the bundled OpenBLAS LP64 DLL by an explicit
absolute path.

## Bundled calculation and plotting examples

The installer includes the curated scripts in `examples/numerics-and-plots`.
On launch, Desktop copies them into the Windows Documents folder under
`OpenMat/Examples/<application-version>`. New sessions start in that folder;
open `START_HERE.m` and click **Run**. Existing sessions can still restore the
last folder selected by the user. The scripts cover FFT, least-squares fitting,
formula labels, mathematical patterns and 3D surfaces; no App Designer examples
are included.

The copies are writable. Later launches fill missing files without replacing
edited scripts, and version folders preserve examples from older releases.
An explicit `OPENMAT_WORKSPACE_ROOT` continues to select that workspace without
copying examples into it. Original scripts remain under the installation's
`resources/examples` directory. `Build-OpenMatWindows.ps1 -PrepareOnly` stages
the same catalog for development and CI.

The installer build runs every bundled script through the actual desktop
kernel. To repeat that check, use
`Test-OpenMatDesktop.ps1 -Executable <path> -CheckBundledExamples`.
It also checks the FFT peak amplitudes and fitting residuals, and requires each
script to create a populated Figure. Add `-UseDefaultWorkspace` to verify the
initial examples folder with an isolated WebView2 profile.

## Persistent kernel port

Desktop and the normal Windows development launcher read the same user settings:
`%APPDATA%\org.openmat.desktop\runtime.json`. The first launch creates this file
from `runtime.default.json` without overwriting an existing configuration:

```json
{
  "schemaVersion": 1,
  "kernelPort": 42000
}
```

Change `kernelPort` to another integer from 1 through 65535 and restart OpenMat.
Every later launch uses that configured port. The host remains `127.0.0.1`.
An occupied port causes startup to fail with the endpoint, settings path and log
location; it never silently switches ports or connects to the occupying process.
Malformed settings also fail startup and are preserved for correction.

Set `OPENMAT_RUNTIME_CONFIG` to select another settings file. The development
launcher also accepts `-RuntimeConfig <file>`. Settings are user data outside the
installation directory, so replacing the application does not replace the port.

Only explicit tests use ephemeral ports: the desktop smoke script isolates its
settings and sets `OPENMAT_DESKTOP_TEST_PORT=0`, while the development launcher's
`-Smoke` mode uses port 0 unless a configuration file is explicitly supplied.
`Test-OpenMatDesktop.ps1 -KernelPort 42123` tests a fixed port; add
`-RuntimeConfig <file>` to reuse a configuration across successive test launches.
The test override never updates the stored port. Port 0 is invalid in normal
configuration files.

Desktop editor sessions use a stable application key independent of the kernel
URL, so future port changes keep the same recovery session. Web sessions remain
separate for each server address. This does not automatically migrate historical
desktop drafts stored under older ephemeral-port keys.

## Exit and recovery

The desktop close button checks both editor tabs and the mounted App Designer,
including a hidden Designer pane. **Save All**, **Don't Save**, and **Cancel**
apply to the whole workbench. An existing save finishes before this decision is
processed. Save or recovery-storage failures keep the window open; a conflicting
file must be resolved in the editor before retrying. Files recovered from an
unavailable folder are never saved over a same-named file in the current folder.

The final editor recovery transaction is awaited before destroying the window.
Autosaves are ordered, and a discarded draft is excluded from recovery without
changing its disk file. Canceling exit keeps the live drafts. App Designer also
recovers its unapplied XML text, including incomplete markup.

Normal desktop startup restores the last Current Folder, open editor tabs,
active tab, cursor and scroll positions, and whether App Designer was open or
hidden. A missing folder falls back to the startup directory while preserving
the unavailable drafts. Recovery is an editor session, not a snapshot of kernel
variables or a running application's execution state.

An explicit `OPENMAT_WORKSPACE_ROOT` takes precedence over the remembered folder
and uses a separate recovery identity for that root. This keeps development and
smoke workspaces out of the regular desktop session. Browser sessions retain
their existing server identity and browser leave-page warning.

The desktop adapter uses Tauri's
[window-close event](https://v2.tauri.app/reference/javascript/api/namespacewindow/#oncloserequested)
and the main window's `core:window:allow-destroy` capability after the workbench
has completed its saves.

## Desktop file workflow

The shared React interface selects its platform services from the runtime
configuration injected by Tauri. A browser connected to a localhost kernel still
uses the Web interface; localhost alone does not identify a desktop host.

| Operation | Desktop | Browser |
| --- | --- | --- |
| Choose Current Folder | System folder dialog | Server directory picker |
| Open File | System file dialog; changes Current Folder to the file's parent | Select a server file, or upload a local file |
| Upload/download | Hidden, including drag-and-drop upload | Available in Current Folder |
| Open/show in File Explorer | Current Folder toolbar and entry context menu | Unavailable |
| Editor Save As / conflict Save a Copy | System save dialog | Existing server-relative Save a Copy dialog for conflicts |
| Figure PNG/SVG export | System save dialog | Browser download |

Opening another folder keeps existing tabs and unsaved drafts. A selected `.omui`
file opens in App Designer; other text files open in the editor. Save As switches
the editor to the saved file and keeps edits made during the save operation dirty.
If the destination already has an unsaved editor, both drafts remain open. Normal
Save continues to use the revision-aware workspace service.

Native saves replace files atomically and are limited to 64 MiB per export.
Canceling a system dialog leaves files and editor identity unchanged. File Explorer
actions open directories or select files; they do not execute selected files.

The version 1 native file bridge uses four application commands in
`src-tauri/src/native_files.rs`. The frontend dynamically loads the Tauri core API
only for native operations. Rust calls the dialog and opener libraries directly;
no general-purpose filesystem or shell IPC permissions are granted to JavaScript.

App Designer's internal layout/source editing and M-language callbacks continue to
share the same workspace service in both versions. Its existing design path fields
remain server-relative; this change adds native file entry points to the workbench,
not a separate native rewrite of every Designer dialog.

## Build the Windows installer

For local checks or CI, prepare the ignored runtime resources without building
an installer:

```powershell
pwsh -NoProfile -File .\tools\release\Build-OpenMatWindows.ps1 -PrepareOnly -DisableCompilerCache
```

This verifies the pinned OpenBLAS download and generates the same license
inventory as the installer build. It leaves the unstripped DLL and license
resources staged, uses the current Cargo cache, and does not run tests, compile
OpenMat, or require the Tauri CLI or a strip tool. Run the separate checks in
[CONTRIBUTING.md](../../CONTRIBUTING.md) afterwards. The installer build still
strips the DLL and runs its release gates by default.

From the repository root:

```powershell
pwsh -NoProfile -File .\tools\release\Build-OpenMatWindows.ps1
```

The script verifies and caches OpenBLAS 0.3.34, runs the release gates, builds
the Rust and Web frontend in release mode, strips debug sections from the staged
OpenBLAS DLL, and creates an NSIS installer under
`output/release/windows-x64`.

Use `-SkipTests` only for a local packaging iteration. `-KeepStaging` preserves
the generated native resource inputs for troubleshooting. Generated staging
files are ignored by Git. The script defaults to `-BuildJobs 6` to keep
Windows release-link memory and page-file usage bounded; increasing it only
changes build speed, not the produced program. Cargo downloads, temporary files,
and Tauri's NSIS tools are kept under `.openmat` or the desktop target directory
on the same drive as the checkout.

An explicit `CARGO_HOME` overrides the default release dependency cache.
`CARGO_TARGET_DIR` is honored for the executable, NSIS manifest and bundle;
the release script resolves that location through Cargo metadata.

## Verify the Windows installer

Use Windows Sandbox for installation tests so the host's installed OpenMat and
user data are not changed. Enable Windows Sandbox before running:

```powershell
pwsh -NoProfile -File tools/release/Test-OpenMatInstallerSandbox.ps1 -Installer output/release/windows-x64/OpenMat-0.1.4-windows-x64-setup.exe
```

Add `-PreviousInstaller <older-setup.exe>` to test a real upgrade. The test uses
the installed executable, its bundled OpenBLAS runtime and the default Documents
workspace. It checks linear solving and FFT, first-start port 42000, repeated
startup, persistence after changing the port to 42001, same-version reinstall,
upgrade when requested, obsolete-file cleanup, and preservation of user files
and settings after uninstall. It also checks that the guest has no Rust or Node
development toolchain.

Each run creates an ignored `output/installer-tests/<run-id>` directory with
the `.wsb` configuration, transcript and JSON result. Only copied installers
and test scripts are mapped into the guest read-only; a separate result folder
is writable. Network access allows the installer to provision WebView2 when
needed. The guest shuts down after recording its result. Use `-PrepareOnly` to
create the configuration without launching it, or `-OutputDirectory` to place
the run elsewhere. The guest install script refuses execution outside the
Sandbox account. A missing `-PreviousInstaller` means upgrade testing was not run.

The setup uses Microsoft's [Windows Sandbox configuration format](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-configure-using-wsb-file)
and [NSIS silent mode](https://nsis.sourceforge.io/Which_command_line_parameters_can_be_used_to_configure_installers).
This is an automated installation and kernel acceptance check; native file-dialog
interaction and visual inspection remain separate checks.

PNG composition also needs a real browser check: DOM-only unit tests cannot
detect a canvas losing its origin-clean flag. Start a Web development server
on a test port, attach a separate Playwright CLI session to that page, and run:

```powershell
pnpm --dir apps/web dev:ready --host 127.0.0.1 --port 5178 --strictPort
# In another terminal:
npx --yes --package @playwright/cli playwright-cli -s=figure-export open http://127.0.0.1:5178
npx --yes --package @playwright/cli playwright-cli -s=figure-export run-code --filename apps/web/scripts/check-figure-png-export.js
```

This exercises the actual PNG composition function in Chromium with plain SVG,
HTML math labels, Unicode, `#` characters, and LaTeX prose mixed with formulas.
It checks spaces, decoding, dimensions, background pixels and visible label
pixels without mocking Canvas or Image.
It does not replace testing the installed application's native save dialog.

The installed application creates its default workspace in the user's
Documents directory as `OpenMat`. Set `OPENMAT_WORKSPACE_ROOT` before launch to
override that location for controlled testing. OpenBLAS defaults to at most
eight worker threads in the desktop host; set `OPENMAT_OPENBLAS_NUM_THREADS` to
an integer from 1 through 64 to override it.

For controlled native-plugin testing, set `OPENMAT_OEX_PLUGINS` to a
platform-native path list before launching the desktop application. On Windows,
separate multiple trusted OEX DLL paths with semicolons. The desktop passes the
paths to its in-process server in order; each new kernel session loads them
through the same `--oex-plugin` boundary used by the standalone server.
