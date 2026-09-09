[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $Binary
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $PSCommandPath
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $scriptRoot '..\..')).Path
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$lockPath = Join-Path $repositoryRoot 'Cargo.lock'

$expectedPackages = [ordered]@{
    'hdf5-metno' = @{
        Version = '0.14.1'
        Sha256 = 'f72d6ab4f6d6d79bd350c23f6fe17c99d86d8dd05c0e86f6291a366d47e1fece'
    }
    'hdf5-metno-src' = @{
        Version = '0.10.4'
        Sha256 = '69c883c565498492954e344c482f8622525e58c7a71100dd9498c4940da82885'
    }
    'hdf5-metno-sys' = @{
        Version = '0.12.3'
        Sha256 = '8139abe2218e47a40bdc7822a4365b620e23dce17e6b4735bbe6629410cdfedb'
    }
    'libz-sys' = @{
        Version = '1.1.29'
        Sha256 = '85bc9657773828b90eeb625adff10eeac83cc21bbfd8e23a03eaa8a33c9e28d9'
    }
}

$lockText = Get-Content -LiteralPath $lockPath -Raw
foreach ($entry in $expectedPackages.GetEnumerator()) {
    $name = [regex]::Escape([string] $entry.Key)
    $version = [regex]::Escape([string] $entry.Value.Version)
    $sha256 = [regex]::Escape([string] $entry.Value.Sha256)
    $pattern = "(?ms)^\[\[package\]\]\r?\nname = `"$name`"\r?\nversion = `"$version`"\r?\nsource = .*?\r?\nchecksum = `"$sha256`""
    if ($lockText -notmatch $pattern) {
        throw "Cargo.lock does not contain the approved $($entry.Key) $($entry.Value.Version) SHA-256."
    }
}

Push-Location $repositoryRoot
try {
    $featureTree = (& cargo tree --locked -p openmat-mat -e features 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tree failed:`n$featureTree"
    }
}
finally {
    Pop-Location
}

foreach ($required in @(
    'hdf5-metno feature "static"',
    'hdf5-metno feature "zlib"',
    'hdf5-metno-src feature "zlib"',
    'libz-sys feature "static"'
)) {
    if (-not $featureTree.Contains($required, [StringComparison]::Ordinal)) {
        throw "The resolved feature tree is missing '$required'."
    }
}

$objdump = Get-Command 'objdump.exe' -ErrorAction Stop
$peHeaders = (& $objdump.Source -p $binaryPath 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "objdump failed for '$binaryPath':`n$peHeaders"
}
$imports = [regex]::Matches($peHeaders, '(?im)^\s*DLL Name:\s*(\S+)\s*$') |
    ForEach-Object { $_.Groups[1].Value }
$forbiddenImports = @($imports | Where-Object { $_ -match '(?i)hdf5|zlib|(^|[^a-z])z\.dll$' })
if ($forbiddenImports.Count -ne 0) {
    throw "The release binary dynamically imports a forbidden HDF5/zlib library: $($forbiddenImports -join ', ')."
}

$binarySha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host "Verified locked HDF5 2.2.0 and zlib 1.3.2 static feature graph."
Write-Host "Binary: $binaryPath"
Write-Host "Binary SHA-256: $binarySha256"
Write-Host "PE imports: $($imports -join ', ')"
Write-Host 'No HDF5 or zlib DLL import was found.'
