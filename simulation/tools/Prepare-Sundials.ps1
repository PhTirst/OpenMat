#Requires -Version 7.0
[CmdletBinding()]
param(
    [string] $Destination = (Join-Path $PSScriptRoot '../.openmat/sundials-7.5.0'),
    [string] $ArchiveDirectory,
    [switch] $Offline
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') {
    throw 'The current CVODE adapter requires the pinned Windows x64 serial runtime.'
}
$runtimeDirectory = [System.IO.Path]::GetFullPath($Destination)
if (-not $ArchiveDirectory) { $ArchiveDirectory = Join-Path $runtimeDirectory 'archives' }
$archiveCache = [System.IO.Path]::GetFullPath($ArchiveDirectory)
New-Item -ItemType Directory -Path $runtimeDirectory, $archiveCache -Force | Out-Null
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'sundials-windows.lock.json') -Raw | ConvertFrom-Json
$archiveTool = (Get-Command tar.exe -ErrorAction Stop).Source
foreach ($package in $manifest.packages) {
    $archivePath = Join-Path $archiveCache $package.file
    if (-not (Test-Path -LiteralPath $archivePath)) {
        if ($Offline) { throw "Offline archive is missing: $($package.file)" }
        Write-Host "Downloading $($package.file)"
        Invoke-WebRequest -Uri $package.url -OutFile $archivePath -TimeoutSec 180
    }
    if ((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ne $package.sha256) {
        throw "SHA-256 mismatch: $($package.file). Nothing was extracted."
    }
    $entries = @(& $archiveTool -tf $archivePath)
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect $($package.file)" }
    foreach ($entry in $entries) {
        if ($entry -notmatch '^(?:ucrt64(?:/|$)|\.(?:BUILDINFO|MTREE|PKGINFO|INSTALL)$)' -or $entry -match '(?:^|/)\.\.(?:/|$)' -or $entry.Contains('\')) {
            throw "Unexpected archive path: $entry"
        }
    }
    # Only the serial CVODE closure, its ABI configuration and licenses are
    # extracted. No Fortran, OpenBLAS, MPI, solver source or global installation.
    $extractPaths = @($package.paths)
    & $archiveTool -xf $archivePath -C $runtimeDirectory @extractPaths
    if ($LASTEXITCODE -ne 0) { throw "Cannot extract $($package.file)" }
    Write-Host "Verified $($package.file)"
}
$directory = Join-Path $runtimeDirectory 'ucrt64/bin'
foreach ($name in @('libsundials_core-7.dll', 'libsundials_cvode-7.dll', 'libsundials_nvecserial-7.dll', 'libsundials_sunmatrixdense-5.dll', 'libsundials_sunlinsoldense-5.dll', 'libwinpthread-1.dll')) {
    if (-not (Test-Path -LiteralPath (Join-Path $directory $name))) { throw "Missing runtime library: $name" }
}
return $directory
