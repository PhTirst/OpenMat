#Requires -Version 7.0
[CmdletBinding()]
param(
    [switch] $UsePreparedAssets,
    [switch] $SkipBuild,
    [switch] $Force,
    [ValidateRange(1, 16)] [int] $BuildJobs = 4
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$desktopResources = Join-Path $repository 'apps\desktop\src-tauri\resources'
$dist = Join-Path $repository 'apps\web\dist'
$version = (Get-Content -LiteralPath (Join-Path $repository 'apps\desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
$output = Join-Path $repository 'output\release\windows-x64'
$name = "OpenMat-$version-web-windows-x64"
$archive = Join-Path $output "$name.zip"
if ((Test-Path -LiteralPath $archive) -and -not $Force) { throw "Output already exists: $archive. Use -Force to replace it." }

Push-Location $repository
try {
    if (-not $UsePreparedAssets -and -not $SkipBuild) {
        & (Join-Path $PSScriptRoot 'Build-OpenMatWindows.ps1') -PrepareOnly -DisableCompilerCache -BuildJobs $BuildJobs
        if ($LASTEXITCODE -ne 0) { throw 'Runtime resource preparation failed.' }
        & pnpm --dir apps/web build
        if ($LASTEXITCODE -ne 0) { throw 'Web production build failed.' }
    }
    if (-not $SkipBuild) {
        & cargo build --release --locked -p openmat-server -j $BuildJobs
        if ($LASTEXITCODE -ne 0) { throw 'Rust server release build failed.' }
    }
    $metadataText = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataText | ConvertFrom-Json
    $server = Join-Path $metadata.target_directory 'release\openmat-server.exe'
    foreach ($file in @($server, (Join-Path $dist 'index.html'), (Join-Path $desktopResources 'openblas\libopenblas.dll'), (Join-Path $desktopResources 'licenses\THIRD-PARTY-MANIFEST.json'))) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required build output is missing: $file" }
    }
    if (@(Get-ChildItem -LiteralPath $dist -Recurse -File -Filter '*.map').Count -ne 0) { throw 'Production frontend contains source maps.' }

    $nodeVersion = '22.19.0'
    $nodeSha = '995a3fb3cefad590cd3f4b321532a4b9582fb9c6575320ed2e3e894caac3e362'
    $nodeLicenseSha = 'e991d81497a85bb24fc6bffae0a3637a6accd6c6bc5ce1f2c5698bd555cf9d49'
    $downloads = Join-Path $repository '.openmat\downloads'
    New-Item -ItemType Directory -Path $downloads, $output -Force | Out-Null
    $nodeRuntime = Join-Path $downloads "node-v$nodeVersion-win-x64.exe"
    $nodeLicense = Join-Path $downloads "node-v$nodeVersion-LICENSE.txt"
    $installedNode = Get-Command node.exe -ErrorAction SilentlyContinue
    if (-not (Test-Path -LiteralPath $nodeRuntime)) {
        if ($installedNode -and (Get-FileHash -LiteralPath $installedNode.Source).Hash -ieq $nodeSha) {
            Copy-Item -LiteralPath $installedNode.Source -Destination $nodeRuntime
        } else {
            Invoke-WebRequest -Uri "https://nodejs.org/dist/v$nodeVersion/win-x64/node.exe" -OutFile $nodeRuntime
        }
    }
    if (-not (Test-Path -LiteralPath $nodeLicense)) {
        Invoke-WebRequest -Uri "https://raw.githubusercontent.com/nodejs/node/v$nodeVersion/LICENSE" -OutFile $nodeLicense
    }
    if ((Get-FileHash -LiteralPath $nodeRuntime).Hash -ine $nodeSha) { throw 'Node.js executable checksum mismatch.' }
    if ((Get-FileHash -LiteralPath $nodeLicense).Hash -ine $nodeLicenseSha) { throw 'Node.js license checksum mismatch.' }

    # Each attempt gets its own staging directory; no recursive deletion is needed.
    $stage = Join-Path ([IO.Path]::GetTempPath()) ('openmat-web-release-' + [guid]::NewGuid().ToString('N'))
    $package = Join-Path $stage $name
    New-Item -ItemType Directory -Path $package, (Join-Path $package 'bin'), (Join-Path $package 'workspace') -Force | Out-Null
    Copy-Item -LiteralPath $server -Destination (Join-Path $package 'bin\openmat-server.exe')
    Copy-Item -LiteralPath (Join-Path $desktopResources 'openblas\libopenblas.dll') -Destination (Join-Path $package 'bin\libopenblas.dll')
    Copy-Item -LiteralPath $nodeRuntime -Destination (Join-Path $package 'bin\node.exe')
    Copy-Item -LiteralPath $dist -Destination (Join-Path $package 'www') -Recurse
    Copy-Item -LiteralPath (Join-Path $desktopResources 'licenses') -Destination (Join-Path $package 'licenses') -Recurse
    # The standalone server uses the root lockfile, which can differ from Tauri's.
    & (Join-Path $PSScriptRoot 'Generate-ThirdPartyLicenses.ps1') `
        -RepositoryRoot $repository -OutputDirectory (Join-Path $package 'licenses') `
        -OpenBlasLicense (Join-Path $desktopResources 'licenses\OpenBLAS-LICENSE.txt') `
        -NsisLicense (Join-Path $PSScriptRoot 'licenses\NSIS-3.08-COPYING.txt') -IncludeServer
    if ($LASTEXITCODE -ne 0) { throw 'Standalone server license generation failed.' }
    Copy-Item -LiteralPath $nodeLicense -Destination (Join-Path $package 'licenses\NODEJS-LICENSE.txt')
    foreach ($file in @('launch.mjs', 'Start-OpenMatWeb.cmd', 'openmat-web.json', 'README.md')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot "web\$file") -Destination $package
    }
    $indexPath = Join-Path $package 'www\index.html'
    $index = Get-Content -LiteralPath $indexPath -Raw
    if ($index -notmatch '<head>') { throw 'Could not inject the Web runtime endpoint.' }
    $index.Replace('<head>', "<head>`n    <script src=`"/openmat-runtime.js`"></script>") | Set-Content -LiteralPath $indexPath -Encoding utf8NoBOM
    $commit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not identify the source commit.' }
    $info = [ordered]@{
        product = 'OpenMat Web'; version = $version; platform = 'windows-x64'; sourceCommit = $commit
        nodeVersion = $nodeVersion; nodeSha256 = $nodeSha
        nodeLicenseSha256 = $nodeLicenseSha
        nodeSource = "https://nodejs.org/dist/v$nodeVersion/"
        serverSha256 = (Get-FileHash -LiteralPath $server).Hash.ToLowerInvariant()
        sourceBundleComplete = $false
    }
    $info | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $package 'BUILD-INFO.json') -Encoding utf8NoBOM
    $hashes = Get-ChildItem -LiteralPath $package -Recurse -File | Sort-Object FullName | ForEach-Object {
        $relative = [IO.Path]::GetRelativePath($package, $_.FullName).Replace('\', '/')
        '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant(), $relative
    }
    $hashes | Set-Content -LiteralPath (Join-Path $package 'SHA256SUMS.txt') -Encoding utf8NoBOM
    $pending = Join-Path $stage "$name.zip"
    Compress-Archive -LiteralPath $package -DestinationPath $pending -CompressionLevel Optimal
    Move-Item -LiteralPath $pending -Destination $archive -Force
    $sha = (Get-FileHash -LiteralPath $archive).Hash.ToLowerInvariant()
    "$sha  $name.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii
    Write-Output "Web package: $archive"
    Write-Output "SHA-256: $sha"
    Write-Output "Staged package: $package"
}
finally { Pop-Location }
