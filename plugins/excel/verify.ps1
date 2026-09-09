param([string]$BuildRoot)
$ErrorActionPreference = 'Stop'
$pluginRoot = $PSScriptRoot
$manifest = Join-Path $pluginRoot 'Cargo.toml'
$hostManifest = Join-Path $pluginRoot 'tests/host/Cargo.toml'
if (-not $BuildRoot) { $BuildRoot = Join-Path $pluginRoot 'target' }
$pluginTarget = Join-Path $BuildRoot 'plugin'
$hostTarget = Join-Path $BuildRoot 'host'

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo failed with exit code $LASTEXITCODE" }
}

Invoke-Cargo fmt --manifest-path $manifest --check
Invoke-Cargo fmt --manifest-path $hostManifest --check
Invoke-Cargo clippy --manifest-path $manifest --all-targets --locked --target-dir $pluginTarget '--' -D warnings
Invoke-Cargo test --manifest-path $manifest --locked --target-dir $pluginTarget
# This separate test executable uses the existing kernel without editing it.
Invoke-Cargo build --manifest-path $hostManifest --locked --target-dir $hostTarget
# Explicitly build the existing bridge as a top-level DLL in this same cache.
Invoke-Cargo build --manifest-path $hostManifest -p openmat-oex --locked --target-dir $hostTarget
& (Join-Path $pluginRoot 'build.ps1') -BridgePath (Join-Path $hostTarget 'debug/openmat_oex.dll') -TargetDirectory $pluginTarget
Invoke-Cargo run --manifest-path $hostManifest --locked --target-dir $hostTarget '--' `
    (Join-Path $pluginRoot 'dist/openmat_excel.dll') `
    (Join-Path $pluginRoot 'tests/integration.m')
