$ErrorActionPreference = 'Stop'
$sdkRoot = $PSScriptRoot
$repositoryRoot = (Resolve-Path (Join-Path $sdkRoot '../..')).Path
$sdkManifest = Join-Path $sdkRoot 'Cargo.toml'
$hostManifest = Join-Path $sdkRoot 'tests/host/Cargo.toml'
$sdkTarget = Join-Path $sdkRoot 'target'
$hostTarget = Join-Path $repositoryRoot 'target'

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo failed with exit code $LASTEXITCODE" }
}

Push-Location $repositoryRoot
$previousRustdocFlags = $env:RUSTDOCFLAGS
try {
    Invoke-Cargo fmt --manifest-path $sdkManifest --all --check
    Invoke-Cargo fmt --manifest-path $hostManifest --check
    Invoke-Cargo clippy --manifest-path $sdkManifest --workspace --all-targets --locked --target-dir $sdkTarget '--' -D warnings
    Invoke-Cargo test --manifest-path $sdkManifest --locked --target-dir $sdkTarget
    $env:RUSTDOCFLAGS = '-D warnings'
    Invoke-Cargo doc --manifest-path $sdkManifest --no-deps --locked --target-dir $sdkTarget
    Invoke-Cargo build -p openmat-oex --locked --target-dir $hostTarget
    Invoke-Cargo build --manifest-path $sdkManifest --workspace --locked --target-dir $sdkTarget
    Copy-Item -LiteralPath (Join-Path $hostTarget 'debug/openmat_oex.dll') -Destination (Join-Path $sdkTarget 'debug/openmat_oex.dll')
    # The standalone host resolves a different dependency graph. Its un-hashed
    # openmat_oex rlib/cdylib must not overwrite the root workspace's artifacts.
    Invoke-Cargo run --manifest-path $hostManifest --locked --target-dir $sdkTarget '--' `
        (Join-Path $sdkRoot 'tests/integration.m') `
        (Join-Path $sdkTarget 'debug/oex_rust_example.dll') `
        (Join-Path $sdkTarget 'debug/oex_rust_test_plugin.dll')
}
finally {
    $env:RUSTDOCFLAGS = $previousRustdocFlags
    Pop-Location
}
