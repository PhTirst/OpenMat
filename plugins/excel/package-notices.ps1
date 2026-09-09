param(
    [Parameter(Mandatory = $true)][string]$Destination
)
$ErrorActionPreference = 'Stop'
$metadataText = & cargo metadata --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Cannot read dependency metadata' }
$metadata = ($metadataText -join "`n") | ConvertFrom-Json
$nodes = @{}
foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
$reachable = [System.Collections.Generic.HashSet[string]]::new()
$queue = [System.Collections.Generic.Queue[string]]::new()
$queue.Enqueue($metadata.resolve.root)
while ($queue.Count -gt 0) {
    $packageId = $queue.Dequeue()
    if (-not $reachable.Add($packageId)) { continue }
    foreach ($dependency in $nodes[$packageId].deps) {
        if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) {
            $queue.Enqueue($dependency.pkg)
        }
    }
}
$noticeRoot = Join-Path $Destination 'third-party'
New-Item -ItemType Directory -Path $noticeRoot -Force | Out-Null
$index = @('Third-party dependencies of the Excel plugin (Windows x64).', 'The OpenMat bridge is supplied separately under the OpenMat distribution terms.', '')
foreach ($package in $metadata.packages) {
    if (-not $package.source -or -not $reachable.Contains($package.id)) { continue }
    $name = "$($package.name)-$($package.version)"
    $licenses = @(Get-ChildItem -LiteralPath (Split-Path $package.manifest_path) -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT)' })
    if ($licenses.Count -eq 0) { throw "Dependency license files are missing for $name" }
    $folder = Join-Path $noticeRoot $name
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    foreach ($license in $licenses) { Copy-Item -LiteralPath $license.FullName -Destination $folder }
    $index += "$name : $($package.license) : $($package.repository)"
}
$index | Set-Content -LiteralPath (Join-Path $noticeRoot 'INDEX.txt') -Encoding utf8
