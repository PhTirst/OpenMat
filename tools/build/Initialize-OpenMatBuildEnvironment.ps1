[CmdletBinding()]
param(
    [string] $RepositoryRoot,

    [ValidateRange(1, 32)]
    [int] $BuildJobs = 6,

    [ValidateRange(1, 256)]
    [int] $CompilerCacheSizeGiB = 20,

    [switch] $DisableCompilerCache,

    [switch] $RequireCompilerCache
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$sccacheVersion = '0.17.0'
$sccacheArchiveName = "sccache-v$sccacheVersion-x86_64-pc-windows-msvc.zip"
$sccacheArchiveSha256 = 'e94cfc5b58cbe439302f586c1d1bd7980c2cd371d47bdf385ade657411e6f3ac'
$sccacheDownloadUrl = "https://github.com/mozilla/sccache/releases/download/v$sccacheVersion/$sccacheArchiveName"

function Resolve-OpenMatBuildRoot {
    param([string] $Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        $Path = Join-Path $PSScriptRoot '..\..'
    }
    $resolved = (Resolve-Path -LiteralPath $Path).Path
    if (-not (Test-Path -LiteralPath (Join-Path $resolved 'Cargo.toml') -PathType Leaf)) {
        throw "OpenMat repository root does not contain Cargo.toml: $resolved"
    }
    return $resolved
}

function Assert-OpenMatSha256 {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Expected
    )

    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected) {
        throw "SHA-256 mismatch for '$Path'. Expected $Expected, got $actual."
    }
}

function Install-OpenMatSccache {
    param([Parameter(Mandatory)] [string] $Root)

    $downloadRoot = Join-Path $Root '.openmat\downloads'
    $toolRoot = Join-Path $Root ".openmat\tools\sccache\v$sccacheVersion\x86_64-pc-windows-msvc"
    $archivePath = Join-Path $downloadRoot $sccacheArchiveName
    $executablePath = Join-Path $toolRoot 'sccache.exe'
    New-Item -ItemType Directory -Force -Path $downloadRoot, $toolRoot | Out-Null

    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        $partialPath = "$archivePath.partial"
        if (Test-Path -LiteralPath $partialPath -PathType Leaf) {
            Remove-Item -LiteralPath $partialPath -Force
        }
        Invoke-WebRequest `
            -Headers @{ 'User-Agent' = 'OpenMat-build-bootstrap' } `
            -Uri $sccacheDownloadUrl `
            -OutFile $partialPath
        Assert-OpenMatSha256 -Path $partialPath -Expected $sccacheArchiveSha256
        Move-Item -LiteralPath $partialPath -Destination $archivePath
    }
    Assert-OpenMatSha256 -Path $archivePath -Expected $sccacheArchiveSha256

    if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
        $extractRoot = Join-Path $Root '.openmat\temp\sccache-extract'
        if (Test-Path -LiteralPath $extractRoot -PathType Container) {
            Remove-Item -LiteralPath $extractRoot -Recurse -Force
        }
        New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
        try {
            Expand-Archive -LiteralPath $archivePath -DestinationPath $extractRoot -Force
            $candidate = Get-ChildItem -LiteralPath $extractRoot -Recurse -File -Filter 'sccache.exe' |
                Select-Object -First 1
            if ($null -eq $candidate) {
                throw "Archive '$archivePath' does not contain sccache.exe."
            }
            Copy-Item -LiteralPath $candidate.FullName -Destination $executablePath -Force
        }
        finally {
            if (Test-Path -LiteralPath $extractRoot -PathType Container) {
                Remove-Item -LiteralPath $extractRoot -Recurse -Force
            }
        }
    }
    return $executablePath
}

$root = Resolve-OpenMatBuildRoot -Path $RepositoryRoot
$env:CARGO_BUILD_JOBS = $BuildJobs.ToString([Globalization.CultureInfo]::InvariantCulture)

if ($DisableCompilerCache) {
    $env:RUSTC_WRAPPER = $null
    return [pscustomobject]@{
        BuildJobs = $BuildJobs
        CompilerCacheEnabled = $false
        CompilerCachePath = $null
        CompilerCacheReason = 'disabled'
    }
}

$hostTuple = (& rustc --print host-tuple).Trim()
if (-not $IsWindows -or $hostTuple -ne 'x86_64-pc-windows-msvc') {
    $message = "OpenMat's pinned sccache bootstrap currently supports x86_64-pc-windows-msvc; current host is '$hostTuple'."
    if ($RequireCompilerCache) {
        throw $message
    }
    Write-Warning $message
    return [pscustomobject]@{
        BuildJobs = $BuildJobs
        CompilerCacheEnabled = $false
        CompilerCachePath = $null
        CompilerCacheReason = 'unsupported-host'
    }
}

try {
    $sccache = Install-OpenMatSccache -Root $root
    $env:RUSTC_WRAPPER = $sccache
    $env:SCCACHE_DIR = Join-Path $root '.openmat\sccache'
    $env:SCCACHE_CACHE_SIZE = "${CompilerCacheSizeGiB}G"
    $env:SCCACHE_BASEDIRS = $root
    # Client-side mode is intentionally not enabled. On the OpenMat Windows
    # benchmark it was slower than the default daemon mode.
    $env:SCCACHE_CLIENT_SIDE = $null
    New-Item -ItemType Directory -Force -Path $env:SCCACHE_DIR | Out-Null

    return [pscustomobject]@{
        BuildJobs = $BuildJobs
        CompilerCacheEnabled = $true
        CompilerCachePath = $sccache
        CompilerCacheReason = 'pinned-sccache'
    }
}
catch {
    $env:RUSTC_WRAPPER = $null
    if ($RequireCompilerCache) {
        throw
    }
    Write-Warning "Unable to initialize the optional OpenMat compiler cache: $($_.Exception.Message)"
    return [pscustomobject]@{
        BuildJobs = $BuildJobs
        CompilerCacheEnabled = $false
        CompilerCachePath = $null
        CompilerCacheReason = 'bootstrap-failed'
    }
}
