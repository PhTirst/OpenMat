# OpenMat Web for Windows x64

Extract the entire ZIP into a writable directory, then double-click
`Start-OpenMatWeb.cmd`. The launcher opens your browser at
`http://127.0.0.1:42001/`. Keep the launcher running; press Ctrl+C there to stop.
Use a current Edge or Chrome browser with WebGPU support for Figures.

No MATLAB, Rust, Node.js installation, or package-manager command is needed.
The package contains the compiled Rust kernel, OpenBLAS DLL, production React
and Plot WASM assets, and a pinned Node.js runtime used only to serve static
web files and supervise startup. M-language execution remains in Rust.

## Ports and workspace

Edit `openmat-web.json` while OpenMat Web is stopped. Each later launch reads
these same settings; there is no automatic port switching:

- `webPort`: browser HTTP port, initially 42001.
- `kernelPort`: Rust kernel port, initially 42000.
- `workspaceRoot`: absolute path or a path relative to the configuration file,
  initially the `workspace` directory next to the launcher.

Both ports must be different integers from 1 to 65535. If a port is occupied,
startup fails. Desktop also defaults to kernel port 42000; stop it first or
choose another kernel port in this package's configuration.

For a separate configuration or manual browser startup:

```powershell
.\bin\node.exe .\launch.mjs --config C:\OpenMatSettings\web.json --no-open
```

The current server is a local, single-user service. Both listeners bind only
to 127.0.0.1. This package does not add LAN/public hosting, authentication, or
TLS; it must not be exposed through port forwarding or a public reverse proxy.
Workspace uploads/downloads and browser PNG/SVG downloads use the Web UI;
native desktop file dialogs are provided by the separate Desktop installer.

## Files and updates

`www/` contains only static application assets. Your documents are in the
configured workspace and runtime diagnostics are in `logs/`. Extract an update
into a new directory, preserve your configuration, and point it at your existing
workspace. Do not overwrite a working directory that contains user documents.

`BUILD-INFO.json` identifies the source commit and release version.
`SHA256SUMS.txt` lists the shipped files. `licenses/` contains OpenMat's AGPL
license, third-party notices, MPL source material, and the Node.js runtime
license (including its bundled dependencies). These materials do not claim to
be a complete AGPL Corresponding Source release bundle.
