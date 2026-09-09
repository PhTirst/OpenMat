[CmdletBinding()]
param(
    [switch] $PrepareOnly,
    [switch] $SkipTests,
    [switch] $KeepStaging,
    [switch] $DisableCompilerCache,
    [ValidateRange(1, 16)]
    [int] $BuildJobs = 6
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$releaseScriptRoot = Split-Path -Parent $PSCommandPath
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $releaseScriptRoot '..\..')).Path
$desktopRoot = Join-Path $repositoryRoot 'apps\desktop'
$tauriRoot = Join-Path $desktopRoot 'src-tauri'
$webRoot = Join-Path $repositoryRoot 'apps\web'
$tauriConfig = Get-Content -LiteralPath (Join-Path $tauriRoot 'tauri.conf.json') -Raw | ConvertFrom-Json
$appVersion = [string] $tauriConfig.version
if ([string]::IsNullOrWhiteSpace($appVersion)) {
    throw 'The Tauri configuration must provide a non-empty application version.'
}
$openBlasVersion = '0.3.34'
$openBlasArchiveName = "OpenBLAS-$openBlasVersion-x64.zip"
$openBlasArchiveSha256 = 'e9cb6134541f36c27346d5fc5995652f060fba227cebbbabcbda5a5a44d7c76b'
$openBlasLicenseSha256 = '190b5a9c8d9723fe958ad33916bd7346d96fab3c5ea90832bb02d854f620fcff'
$openBlasDownloadUrl = "https://github.com/OpenMathLib/OpenBLAS/releases/download/v$openBlasVersion/$openBlasArchiveName"
$openBlasLicenseUrl = "https://raw.githubusercontent.com/OpenMathLib/OpenBLAS/v$openBlasVersion/LICENSE"
$downloadRoot = Join-Path $repositoryRoot '.openmat\downloads'
$dependencyRoot = Join-Path $repositoryRoot ".openmat\dependencies\openblas\v$openBlasVersion\windows-x86_64-lp64"
$archivePath = Join-Path $downloadRoot $openBlasArchiveName
$dependencyDll = Join-Path $dependencyRoot 'bin\libopenblas.dll'
$dependencyLicense = Join-Path $dependencyRoot 'LICENSE'
$licenseGenerator = Join-Path $releaseScriptRoot 'Generate-ThirdPartyLicenses.ps1'
$buildEnvironmentInitializer = Join-Path $repositoryRoot 'tools\build\Initialize-OpenMatBuildEnvironment.ps1'
$nsisLicense = Join-Path $releaseScriptRoot 'licenses\NSIS-3.08-COPYING.txt'
$resourceRoot = Join-Path $tauriRoot 'resources'
$resourceOpenBlasRoot = Join-Path $resourceRoot 'openblas'
$resourceLicenseRoot = Join-Path $resourceRoot 'licenses'
$exampleSourceRoot = Join-Path $repositoryRoot 'examples\numerics-and-plots'
$resourceExampleRoot = Join-Path $resourceRoot 'examples'
$exampleCatalog = Get-Content -LiteralPath (Join-Path $exampleSourceRoot 'catalog.json') -Raw | ConvertFrom-Json
$requiredExampleArtifacts = @('README.txt', 'catalog.json') + @($exampleCatalog.examples | ForEach-Object { [string] $_.file })
$stagedOpenBlas = Join-Path $resourceOpenBlasRoot 'libopenblas.dll'
$requiredLicenseArtifacts = @(
    'OPENMAT-AGPL-3.0.txt',
    'THIRD-PARTY-NOTICES.txt',
    'THIRD-PARTY-MANIFEST.json',
    'MPL-2.0-SOURCE.zip',
    'MPL-2.0-SOURCE-OFFER.txt',
    'OpenBLAS-LICENSE.txt',
    'NSIS-COPYING.txt',
    'HDF5-LICENSE.txt',
    'ZLIB-LICENSE.txt',
    'MONACO-LICENSE.txt',
    'MONACO-THIRD-PARTY-NOTICES.txt',
    'KATEX-LICENSE.txt'
)
$releaseOutputRoot = Join-Path $repositoryRoot 'output\release\windows-x64'
$releaseCargoHome = if ([string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
    Join-Path $repositoryRoot '.openmat\cargo-home'
} else {
    [IO.Path]::GetFullPath($env:CARGO_HOME)
}
$releaseTempRoot = Join-Path $repositoryRoot '.openmat\temp'
$previousCargoBuildJobs = $env:CARGO_BUILD_JOBS
$previousCargoIncremental = $env:CARGO_INCREMENTAL
$previousCargoHome = $env:CARGO_HOME
$previousRustcWrapper = $env:RUSTC_WRAPPER
$previousSccacheDirectory = $env:SCCACHE_DIR
$previousSccacheCacheSize = $env:SCCACHE_CACHE_SIZE
$previousSccacheBaseDirectories = $env:SCCACHE_BASEDIRS
$previousSccacheClientSide = $env:SCCACHE_CLIENT_SIDE
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
New-Item -ItemType Directory -Force -Path $releaseTempRoot | Out-Null
# CI resource preparation uses the same Cargo cache as the checks that follow.
# Full installer builds use an explicit CARGO_HOME or the isolated release cache.
if (-not $PrepareOnly) {
    New-Item -ItemType Directory -Force -Path $releaseCargoHome | Out-Null
    $env:CARGO_HOME = $releaseCargoHome
}
$env:TEMP = $releaseTempRoot
$env:TMP = $releaseTempRoot

function Write-ReleaseStep {
    param([Parameter(Mandatory)] [string] $Message)
    Write-Host "`n==> $Message" -ForegroundColor Cyan
}

function Resolve-RequiredCommand {
    param([Parameter(Mandatory)] [string] $Name)
    $resolved = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -eq $resolved) {
        throw "Required command '$Name' was not found."
    }
    return $resolved.Source
}

function Assert-Sha256 {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Expected
    )
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected) {
        throw "SHA-256 mismatch for '$Path'. Expected $Expected, got $actual."
    }
}

function Initialize-OpenBlasDependency {
    New-Item -ItemType Directory -Force -Path $downloadRoot | Out-Null
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        $partialPath = "$archivePath.partial"
        Invoke-WebRequest -Headers @{ 'User-Agent' = 'OpenMat-release-builder' } `
            -Uri $openBlasDownloadUrl `
            -OutFile $partialPath
        Assert-Sha256 -Path $partialPath -Expected $openBlasArchiveSha256
        Move-Item -LiteralPath $partialPath -Destination $archivePath
    }
    Assert-Sha256 -Path $archivePath -Expected $openBlasArchiveSha256

    if (-not (Test-Path -LiteralPath $dependencyDll -PathType Leaf)) {
        New-Item -ItemType Directory -Force -Path $dependencyRoot | Out-Null
        Expand-Archive -LiteralPath $archivePath -DestinationPath $dependencyRoot -Force
    }
    if (-not (Test-Path -LiteralPath $dependencyLicense -PathType Leaf)) {
        Invoke-WebRequest -Headers @{ 'User-Agent' = 'OpenMat-release-builder' } `
            -Uri $openBlasLicenseUrl `
            -OutFile $dependencyLicense
    }
    Assert-Sha256 -Path $dependencyLicense -Expected $openBlasLicenseSha256
}

function Resolve-StripCommand {
    foreach ($name in @('llvm-strip.exe', 'strip.exe')) {
        $resolved = Get-Command $name -ErrorAction SilentlyContinue
        if ($null -ne $resolved) {
            return $resolved.Source
        }
    }
    $msysStrip = 'C:\msys64\ucrt64\bin\strip.exe'
    if (Test-Path -LiteralPath $msysStrip -PathType Leaf) {
        return $msysStrip
    }
    throw 'A strip tool is required to remove the official OpenBLAS DLL debug sections. Install llvm-strip or MSYS2 strip.'
}

function Stage-ReleaseResources {
    param([string] $StripCommand)

    New-Item -ItemType Directory -Force -Path $resourceOpenBlasRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $resourceLicenseRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $resourceExampleRoot | Out-Null
    foreach ($artifact in $requiredExampleArtifacts) {
        if ($artifact -notmatch '^[A-Za-z0-9_-]+\.(m|txt|json)$') {
            throw "Invalid bundled example filename '$artifact'."
        }
        Copy-Item -LiteralPath (Join-Path $exampleSourceRoot $artifact) -Destination (Join-Path $resourceExampleRoot $artifact) -Force
    }
    $unexpectedExamples = @(Get-ChildItem -LiteralPath $resourceExampleRoot -Force |
        Where-Object { $_.PSIsContainer -or $_.Name -notin $requiredExampleArtifacts })
    if ($unexpectedExamples.Count -ne 0) {
        throw 'Example staging contains files outside the curated calculation/plot catalog.'
    }

    foreach ($obsoleteProjectLicense in @(
        'OpenMat-LICENSE-MIT.txt',
        'OpenMat-LICENSE-APACHE.txt'
    )) {
        $obsoletePath = Join-Path $resourceLicenseRoot $obsoleteProjectLicense
        if (Test-Path -LiteralPath $obsoletePath -PathType Leaf) {
            Remove-Item -LiteralPath $obsoletePath -Force
        }
    }

    Copy-Item -LiteralPath $dependencyDll -Destination $stagedOpenBlas -Force
    if (-not [string]::IsNullOrWhiteSpace($StripCommand)) {
        & $StripCommand '--strip-debug' $stagedOpenBlas
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to strip OpenBLAS release DLL with '$StripCommand'."
        }
    }

    & $licenseGenerator `
        -RepositoryRoot $repositoryRoot `
        -OutputDirectory $resourceLicenseRoot `
        -OpenBlasLicense $dependencyLicense `
        -NsisLicense $nsisLicense
    if ($LASTEXITCODE -ne 0) {
        throw 'Third-party license generation failed.'
    }

    foreach ($artifact in $requiredLicenseArtifacts) {
        $artifactPath = Join-Path $resourceLicenseRoot $artifact
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf) -or
            (Get-Item -LiteralPath $artifactPath).Length -eq 0) {
            throw "Required third-party license artifact '$artifactPath' is missing or empty."
        }
    }
    $licenseManifest = Get-Content `
        -LiteralPath (Join-Path $resourceLicenseRoot 'THIRD-PARTY-MANIFEST.json') `
        -Raw |
        ConvertFrom-Json
    if ([int] $licenseManifest.packageCount -lt 300) {
        throw "Third-party manifest contains only $($licenseManifest.packageCount) packages."
    }
    foreach ($requiredPackage in @(
        'tauri', 'wgpu', 'faer', 'hdf5-metno', 'libffi',
        'react', 'monaco-editor', 'katex'
    )) {
        if (@($licenseManifest.packages | Where-Object name -eq $requiredPackage).Count -eq 0) {
            throw "Third-party manifest does not contain required package '$requiredPackage'."
        }
    }
}

$null = Resolve-RequiredCommand -Name 'cargo.exe'
$null = Resolve-RequiredCommand -Name 'rustc.exe'
$null = Resolve-RequiredCommand -Name 'pnpm.cmd'
$hostTuple = (& rustc --print host-tuple).Trim()
if ($hostTuple -ne 'x86_64-pc-windows-msvc') {
    throw "This release script requires x86_64-pc-windows-msvc; current host is '$hostTuple'."
}
$buildEnvironment = & $buildEnvironmentInitializer `
    -RepositoryRoot $repositoryRoot `
    -BuildJobs $BuildJobs `
    -DisableCompilerCache:$DisableCompilerCache
Write-Host "Build jobs: $($buildEnvironment.BuildJobs); compiler cache: $($buildEnvironment.CompilerCacheReason)"

Push-Location $repositoryRoot
try {
    Write-ReleaseStep 'Preparing verified OpenBLAS 0.3.34 LP64 runtime'
    Initialize-OpenBlasDependency

    if (-not $SkipTests -and -not $PrepareOnly) {
        Write-ReleaseStep 'Running Rust and Web release gates'
        & cargo test --workspace --locked
        if ($LASTEXITCODE -ne 0) { throw 'Workspace Rust tests failed.' }
        $previousTestDll = $env:OPENMAT_TEST_OPENBLAS_DLL
        $previousOpenBlasThreads = $env:OPENBLAS_NUM_THREADS
        try {
            $env:OPENMAT_TEST_OPENBLAS_DLL = $dependencyDll
            $env:OPENBLAS_NUM_THREADS = '1'
            & cargo test --locked -p openmat-openblas --test provider -- --test-threads=1
            if ($LASTEXITCODE -ne 0) { throw 'Real OpenBLAS provider tests failed.' }
        }
        finally {
            $env:OPENMAT_TEST_OPENBLAS_DLL = $previousTestDll
            $env:OPENBLAS_NUM_THREADS = $previousOpenBlasThreads
        }
        & pnpm --dir $webRoot test
        if ($LASTEXITCODE -ne 0) { throw 'Web tests failed.' }
    }

    Write-ReleaseStep 'Staging OpenBLAS and complete third-party license inventory'
    $stripCommand = if ($PrepareOnly) { $null } else { Resolve-StripCommand }
    Stage-ReleaseResources -StripCommand $stripCommand

    if ($PrepareOnly) {
        Write-Host "Desktop build resources prepared: $resourceRoot"
        return
    }

    if (-not $SkipTests) {
        Write-ReleaseStep 'Running desktop tests with prepared native resources'
        & cargo test --manifest-path (Join-Path $tauriRoot 'Cargo.toml') --locked
        if ($LASTEXITCODE -ne 0) { throw 'Desktop Rust tests failed.' }
    }

    Write-ReleaseStep 'Building OpenMat NSIS installer'
    $metadataJson = & cargo metadata --manifest-path (Join-Path $tauriRoot 'Cargo.toml') `
        --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the Desktop Cargo target directory.' }
    $targetDirectory = [string] (($metadataJson | ConvertFrom-Json).target_directory)
    if ([string]::IsNullOrWhiteSpace($targetDirectory)) {
        throw 'Desktop Cargo metadata did not provide a target directory.'
    }
    $releaseTarget = Join-Path $targetDirectory 'release'
    # Release builds must not inherit an opt-in incremental setting. The Rust
    # release profile is cacheable by sccache and remains reproducible.
    $env:CARGO_INCREMENTAL = '0'
    Push-Location $desktopRoot
    try {
        & cargo tauri build --ci --bundles nsis -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Tauri release bundle failed.' }
    }
    finally {
        Pop-Location
    }

    $sourceMap = @(Get-ChildItem -LiteralPath (Join-Path $webRoot 'dist') -Recurse -File -Filter '*.map')
    if ($sourceMap.Count -ne 0) {
        throw 'Production Web output contains source maps.'
    }
    $nsisScript = Join-Path $releaseTarget 'nsis\x64\installer.nsi'
    if (-not (Test-Path -LiteralPath $nsisScript -PathType Leaf)) {
        throw "Tauri did not produce the expected NSIS manifest '$nsisScript'."
    }
    $nsisSource = Get-Content -LiteralPath $nsisScript -Raw
    if ($nsisSource -match '(?im)^\s*File\b.*\.(?:pdb|lib|exp|map)"') {
        throw 'The NSIS bundle manifest contains a debug or development-only file.'
    }
    if ($nsisSource -match '(?im)^\s*File\b.*openmat-server\.exe"') {
        throw 'The NSIS bundle unexpectedly contains the legacy server sidecar.'
    }
    foreach ($artifact in $requiredLicenseArtifacts) {
        if ($nsisSource -notmatch [regex]::Escape($artifact)) {
            throw "The NSIS bundle manifest does not include '$artifact'."
        }
    }
    foreach ($artifact in $requiredExampleArtifacts) {
        if ($nsisSource -notmatch [regex]::Escape("resources\examples\$artifact")) {
            throw "The NSIS bundle manifest does not include example '$artifact'."
        }
    }
    $installerHooks = Join-Path $tauriRoot 'installer-hooks.nsh'
    $installerHookSource = Get-Content -LiteralPath $installerHooks -Raw
    if ($nsisSource -notmatch '(?im)^!include .*installer-hooks\.nsh"') {
        throw 'The NSIS bundle does not include the installer cleanup hooks.'
    }
    foreach ($obsoleteFile in @(
        'openmat-server.exe',
        'resources\licenses\OpenMat-LICENSE-MIT.txt',
        'resources\licenses\OpenMat-LICENSE-APACHE.txt'
    )) {
        $expectedDelete = 'Delete /REBOOTOK "$INSTDIR\' + $obsoleteFile + '"'
        if ($installerHookSource -notmatch [regex]::Escape($expectedDelete)) {
            throw "The NSIS hooks do not clean obsolete installed file '$obsoleteFile'."
        }
    }
    $installerRoot = Join-Path $releaseTarget 'bundle\nsis'
    $installer = Get-ChildItem -LiteralPath $installerRoot -File -Filter '*.exe' |
        Sort-Object LastWriteTimeUtc -Descending |
        Select-Object -First 1
    if ($null -eq $installer) {
        throw "No NSIS installer was produced under '$installerRoot'."
    }

    Write-ReleaseStep 'Running single-process desktop protocol smoke'
    $desktopExecutable = Join-Path $releaseTarget 'OpenMat.exe'
    & pwsh -NoProfile -ExecutionPolicy Bypass -File `
        (Join-Path $releaseScriptRoot 'Test-OpenMatDesktop.ps1') `
        -Executable $desktopExecutable -CheckBundledExamples -UseDefaultWorkspace
    if ($LASTEXITCODE -ne 0) { throw 'Single-process desktop protocol smoke failed.' }

    New-Item -ItemType Directory -Force -Path $releaseOutputRoot | Out-Null
    $publishedFileName = "OpenMat-$appVersion-windows-x64-setup.exe"
    $publishedInstaller = Join-Path $releaseOutputRoot $publishedFileName
    Copy-Item -LiteralPath $installer.FullName -Destination $publishedInstaller -Force
    $publishedHash = (Get-FileHash -LiteralPath $publishedInstaller -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumPath = "$publishedInstaller.sha256"
    Set-Content -LiteralPath $checksumPath -Value "$publishedHash  $publishedFileName" -Encoding ascii

    Write-ReleaseStep 'OpenMat release is ready'
    Write-Host "Installer: $publishedInstaller" -ForegroundColor Green
    Write-Host "SHA-256:  $publishedHash" -ForegroundColor Green
}
finally {
    Pop-Location
    $env:CARGO_BUILD_JOBS = $previousCargoBuildJobs
    $env:CARGO_INCREMENTAL = $previousCargoIncremental
    $env:CARGO_HOME = $previousCargoHome
    $env:RUSTC_WRAPPER = $previousRustcWrapper
    $env:SCCACHE_DIR = $previousSccacheDirectory
    $env:SCCACHE_CACHE_SIZE = $previousSccacheCacheSize
    $env:SCCACHE_BASEDIRS = $previousSccacheBaseDirectories
    $env:SCCACHE_CLIENT_SIDE = $previousSccacheClientSide
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
    if (-not $KeepStaging -and -not $PrepareOnly) {
        foreach ($path in @($stagedOpenBlas)) {
            if (Test-Path -LiteralPath $path -PathType Leaf) {
                Remove-Item -LiteralPath $path -Force
            }
        }
    }
}
