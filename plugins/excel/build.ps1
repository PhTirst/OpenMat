param(
    [string]$BridgePath,
    [ValidateSet('debug', 'release')][string]$Profile = 'release',
    [string]$TargetDirectory
)
$ErrorActionPreference = 'Stop'
$pluginRoot = $PSScriptRoot
$repositoryRoot = (Resolve-Path (Join-Path $pluginRoot '../..')).Path
if (-not $BridgePath) {
    $BridgePath = Join-Path $repositoryRoot 'target/debug/openmat_oex.dll'
}
# A distributed bridge can be supplied without compiling the OpenMat application.
$bridge = (Resolve-Path -LiteralPath $BridgePath).Path
$target = if ($TargetDirectory) { $TargetDirectory } else { Join-Path $pluginRoot 'target' }
$cargoArgs = @('build', '--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'), '--locked', '--target-dir', $target)
if ($Profile -eq 'release') { $cargoArgs += '--release' }
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed: $LASTEXITCODE" }
$distribution = Join-Path $pluginRoot 'dist'
New-Item -ItemType Directory -Force -Path $distribution | Out-Null
Copy-Item -LiteralPath (Join-Path $target "$Profile/openmat_excel.dll") -Destination $distribution
Copy-Item -LiteralPath $bridge -Destination $distribution
Copy-Item -LiteralPath (Join-Path $pluginRoot 'README.md') -Destination $distribution
Copy-Item -LiteralPath (Join-Path $pluginRoot 'examples/excel_demo.m') -Destination $distribution
& (Join-Path $pluginRoot 'package-notices.ps1') -Destination $distribution
Write-Output "Excel plugin: $(Join-Path $distribution 'openmat_excel.dll')"
