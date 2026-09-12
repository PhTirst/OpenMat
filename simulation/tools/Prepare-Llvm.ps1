#Requires -Version 7.0
[CmdletBinding()]
param(
    [string] $Destination = (Join-Path $PSScriptRoot '../.openmat/llvm-22.1.8'),
    [string] $ArchiveDirectory,
    [switch] $Offline
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') {
    throw 'This dependency manifest is for Windows x64. Other hosts must supply an LLVM 22 library explicitly.'
}

$runtimeDirectory = [System.IO.Path]::GetFullPath($Destination)
if (-not $ArchiveDirectory) { $ArchiveDirectory = Join-Path $runtimeDirectory 'archives' }
$archiveCache = [System.IO.Path]::GetFullPath($ArchiveDirectory)
New-Item -ItemType Directory -Path $runtimeDirectory, $archiveCache -Force | Out-Null
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'llvm-windows.lock.json') -Raw | ConvertFrom-Json
$archiveTool = (Get-Command tar.exe -ErrorAction Stop).Source

foreach ($package in $manifest.packages) {
    $archivePath = Join-Path $archiveCache $package.file
    if (-not (Test-Path -LiteralPath $archivePath)) {
        if ($Offline) { throw "Offline dependency archive is missing: $($package.file)" }
        Write-Host "Downloading $($package.file)"
        Invoke-WebRequest -Uri $package.url -OutFile $archivePath -TimeoutSec 180
    }
    $actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
    if ($actualHash -ne $package.sha256) {
        throw "SHA-256 mismatch for $($package.file). The archive was not extracted."
    }
    $entries = @(& $archiveTool -tf $archivePath)
    if ($LASTEXITCODE -ne 0) { throw "Cannot read archive $($package.file)" }
    foreach ($entry in $entries) {
        if ($entry -notmatch '^(?:ucrt64(?:/|$)|\.(?:BUILDINFO|MTREE|PKGINFO|INSTALL)$)' -or $entry -match '(?:^|/)\.\.(?:/|$)' -or $entry.Contains('\')) {
            throw "Unexpected archive path in $($package.file): $entry"
        }
    }
    # Versioned upstream archives are verified before extraction; no global
    # installation, PATH modification, checkout or downloaded install script runs.
    & $archiveTool -xf $archivePath -C $runtimeDirectory ucrt64
    if ($LASTEXITCODE -ne 0) { throw "Cannot extract archive $($package.file)" }
    Write-Host "Verified $($package.file)"
}

$libraryPath = Join-Path $runtimeDirectory 'ucrt64/bin/libLLVM-22.dll'
if (-not (Test-Path -LiteralPath $libraryPath)) { throw 'The prepared LLVM shared library is missing.' }
return $libraryPath
