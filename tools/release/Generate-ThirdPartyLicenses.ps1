[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $RepositoryRoot,

    [Parameter(Mandatory)]
    [string] $OutputDirectory,

    [Parameter(Mandatory)]
    [string] $OpenBlasLicense,

    [Parameter(Mandatory)]
    [string] $NsisLicense,

    [string] $DesktopTarget = 'x86_64-pc-windows-msvc',

    [string] $WasmTarget = 'wasm32-unknown-unknown',

    [switch] $IncludeServer
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$openBlasLicensePath = (Resolve-Path -LiteralPath $OpenBlasLicense).Path
$nsisLicensePath = (Resolve-Path -LiteralPath $NsisLicense).Path
$fallbackLicenseRoot = Join-Path $repository 'tools\release\licenses'
$desktopManifest = Join-Path $repository 'apps\desktop\src-tauri\Cargo.toml'
$wasmManifest = Join-Path $repository 'crates\openmat-plot-web\Cargo.toml'
$webRoot = Join-Path $repository 'apps\web'
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null

$generatedNames = @(
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
foreach ($name in $generatedNames) {
    $candidate = Join-Path $output $name
    if ([IO.File]::Exists($candidate)) {
        [IO.File]::Delete($candidate)
    }
}

Copy-Item -LiteralPath (Join-Path $repository 'LICENSE') `
    -Destination (Join-Path $output 'OPENMAT-AGPL-3.0.txt')

$repositoryLicenseFallbacks = @{
    'https://github.com/dropbox/rust-alloc-no-stdlib' = @(
        'rust-alloc-no-stdlib-BSD-3-Clause.txt'
    )
    'https://github.com/sarah-ek/equator' = @('equator-MIT.txt')
    'https://codeberg.org/sarah-quinones/faer' = @('faer-MIT.txt')
    'https://github.com/metno/hdf5-rust' = @('hdf5-rust-Apache-2.0.txt')
    'https://github.com/libffi-rs/libffi-rs' = @('libffi-rs-Apache-2.0.txt')
    'https://github.com/sarah-ek/nano-gemm' = @('nano-gemm-MIT.txt')
    'https://github.com/aclysma/profiling' = @('profiling-Apache-2.0.txt')
    'https://github.com/sarah-quinones/pulp' = @('pulp-MIT.txt')
    'https://github.com/servo/stylo' = @('stylo-MPL-2.0.txt')
    'https://github.com/open-i18n/rust-unic' = @('rust-unic-Apache-2.0.txt')
    'https://github.com/wravery/webview2-rs' = @('webview2-rs-MIT.txt')
}

function Invoke-TextCommand {
    param(
        [Parameter(Mandatory)] [string] $FilePath,
        [Parameter(Mandatory)] [string[]] $Arguments
    )

    $lines = @(& $FilePath @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "Command '$FilePath $($Arguments -join ' ')' failed with exit code $LASTEXITCODE."
    }
    return ($lines -join [Environment]::NewLine)
}

function Get-PackageLicenseFiles {
    param(
        [Parameter(Mandatory)] [string] $PackageRoot,
        [AllowNull()] [string] $DeclaredLicenseFile,
        [Parameter(Mandatory)] [string] $PackageName,
        [AllowEmptyString()] [string] $DeclaredLicense,
        [AllowEmptyString()] [string] $Repository
    )

    $files = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    if (-not [string]::IsNullOrWhiteSpace($DeclaredLicenseFile)) {
        $declared = [IO.Path]::GetFullPath((Join-Path $PackageRoot $DeclaredLicenseFile))
        if (-not [IO.File]::Exists($declared)) {
            throw "Package '$PackageName' declares missing license file '$declared'."
        }
        $files[$declared] = [IO.Path]::GetRelativePath($PackageRoot, $declared)
    }

    foreach ($file in Get-ChildItem -LiteralPath $PackageRoot -File -Force) {
        if ($file.Name -match '(?i)(license|licence|notice|copying|copyright|unlicense)') {
            $files[$file.FullName] = $file.Name
        }
    }

    if ($PackageName -eq 'libz-sys') {
        $stockZlibLicense = Join-Path $PackageRoot 'src\zlib\LICENSE'
        if (-not [IO.File]::Exists($stockZlibLicense)) {
            throw "libz-sys does not contain the expected stock zlib license '$stockZlibLicense'."
        }
        $files[$stockZlibLicense] = 'src/zlib/LICENSE'
    }

    $hasPrimaryLicense = -not [string]::IsNullOrWhiteSpace($DeclaredLicenseFile)
    if (-not $hasPrimaryLicense) {
        $hasPrimaryLicense = @($files.Values | Where-Object {
                [IO.Path]::GetFileName($_) -match
                    '(?i)^(license|licence)([-._].*)?$|^copying$|^unlicense$'
            }).Count -gt 0
    }
    $normalizedRepository = $Repository.TrimEnd('/')
    if (
        -not $hasPrimaryLicense -and
        -not [string]::IsNullOrWhiteSpace($DeclaredLicense) -and
        $repositoryLicenseFallbacks.ContainsKey($normalizedRepository)
    ) {
        foreach ($fallbackName in $repositoryLicenseFallbacks[$normalizedRepository]) {
            $fallbackPath = Join-Path $fallbackLicenseRoot $fallbackName
            if (-not [IO.File]::Exists($fallbackPath)) {
                throw "Repository license fallback '$fallbackPath' does not exist."
            }
            $files[$fallbackPath] = "repository-license/$fallbackName"
        }
    }

    if ($files.Count -eq 0) {
        throw "Third-party package '$PackageName' contains no discoverable license or notice file."
    }

    return @(
        $files.GetEnumerator() |
            Sort-Object Value |
            ForEach-Object {
                [pscustomobject]@{
                    Path = $_.Key
                    Name = $_.Value.Replace('\', '/')
                }
            }
    )
}

function Get-CargoReleasePackages {
    param(
        [Parameter(Mandatory)] [string] $ManifestPath,
        [Parameter(Mandatory)] [string] $Target,
        [Parameter(Mandatory)] [string] $Surface
    )

    if (-not [IO.File]::Exists($ManifestPath)) {
        throw "Cargo manifest '$ManifestPath' does not exist."
    }
    $metadataText = Invoke-TextCommand -FilePath 'cargo.exe' -Arguments @(
        'metadata', '--locked', '--format-version', '1',
        '--filter-platform', $Target,
        '--manifest-path', $ManifestPath
    )
    $metadata = $metadataText | ConvertFrom-Json
    $treeText = Invoke-TextCommand -FilePath 'cargo.exe' -Arguments @(
        'tree', '--locked', '--manifest-path', $ManifestPath,
        '--target', $Target, '-e', 'normal', '--prefix', 'none',
        '--format', '{p}'
    )

    $reachable = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($line in $treeText -split '\r?\n') {
        if ($line -match '^([^ ]+) v([^ ]+)') {
            $reachable.Add("$($Matches[1])@$($Matches[2])") | Out-Null
        }
    }

    $packages = @()
    foreach ($package in $metadata.packages) {
        $key = "$($package.name)@$($package.version)"
        if ($null -eq $package.source -or -not $reachable.Contains($key)) {
            continue
        }
        if ([string]::IsNullOrWhiteSpace([string] $package.license) -and
            [string]::IsNullOrWhiteSpace([string] $package.license_file)) {
            throw "Cargo package '$key' has neither license metadata nor a license file."
        }
        $packageRoot = Split-Path -Parent ([string] $package.manifest_path)
        $licenseFiles = Get-PackageLicenseFiles `
            -PackageRoot $packageRoot `
            -DeclaredLicenseFile ([string] $package.license_file) `
            -PackageName ([string] $package.name) `
            -DeclaredLicense ([string] $package.license) `
            -Repository ([string] $package.repository)
        $packages += [pscustomobject]@{
            Ecosystem = 'Cargo'
            Surface = $Surface
            Name = [string] $package.name
            Version = [string] $package.version
            License = if ([string]::IsNullOrWhiteSpace([string] $package.license)) {
                "license-file: $($package.license_file)"
            } else {
                [string] $package.license
            }
            Authors = @($package.authors) -join '; '
            Repository = [string] $package.repository
            Homepage = [string] $package.homepage
            Source = "https://crates.io/crates/$($package.name)/$($package.version)"
            PackageRoot = $packageRoot
            LicenseFiles = $licenseFiles
        }
    }
    return $packages
}

function Get-OptionalStringProperty {
    param(
        [AllowNull()] [object] $InputObject,
        [Parameter(Mandatory)] [string] $Name
    )

    if ($null -eq $InputObject) {
        return ''
    }
    $property = $InputObject.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) {
        return ''
    }
    return [string] $property.Value
}

function Get-NpmProductionPackages {
    param([Parameter(Mandatory)] [string] $WebDirectory)

    $licenseJson = Invoke-TextCommand -FilePath 'pnpm.cmd' -Arguments @(
        '--dir', $WebDirectory, 'licenses', 'list', '--prod', '--json'
    ) | ConvertFrom-Json
    $packages = @()
    foreach ($licenseGroup in $licenseJson.PSObject.Properties) {
        foreach ($entry in @($licenseGroup.Value)) {
            foreach ($packagePath in @($entry.paths)) {
                $packageJsonPath = Join-Path ([string] $packagePath) 'package.json'
                if (-not [IO.File]::Exists($packageJsonPath)) {
                    throw "npm package metadata '$packageJsonPath' does not exist."
                }
                $packageJson = Get-Content -LiteralPath $packageJsonPath -Raw | ConvertFrom-Json
                $packageLicense = Get-OptionalStringProperty -InputObject $packageJson -Name 'license'
                $license = if ([string]::IsNullOrWhiteSpace($packageLicense)) {
                    [string] $licenseGroup.Name
                } else {
                    $packageLicense
                }
                if ([string]::IsNullOrWhiteSpace($license)) {
                    throw "npm package '$($packageJson.name)@$($packageJson.version)' has no license metadata."
                }
                $repositoryUrl = ''
                $repositoryProperty = $packageJson.PSObject.Properties['repository']
                if ($null -ne $repositoryProperty -and $repositoryProperty.Value -is [string]) {
                    $repositoryUrl = [string] $repositoryProperty.Value
                } elseif ($null -ne $repositoryProperty) {
                    $repositoryUrl = Get-OptionalStringProperty `
                        -InputObject $repositoryProperty.Value `
                        -Name 'url'
                }
                $licenseFiles = Get-PackageLicenseFiles `
                    -PackageRoot ([string] $packagePath) `
                    -DeclaredLicenseFile $null `
                    -PackageName ([string] $packageJson.name) `
                    -DeclaredLicense $license `
                    -Repository $repositoryUrl
                $encodedName = [Uri]::EscapeDataString([string] $packageJson.name)
                $packages += [pscustomobject]@{
                    Ecosystem = 'npm'
                    Surface = 'Web production bundle'
                    Name = [string] $packageJson.name
                    Version = [string] $packageJson.version
                    License = $license
                    Authors = Get-OptionalStringProperty -InputObject $entry -Name 'author'
                    Repository = $repositoryUrl
                    Homepage = Get-OptionalStringProperty -InputObject $packageJson -Name 'homepage'
                    Source = "https://www.npmjs.com/package/$encodedName/v/$($packageJson.version)"
                    PackageRoot = [string] $packagePath
                    LicenseFiles = $licenseFiles
                }
            }
        }
    }
    return $packages
}

function Write-Utf8File {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Content
    )
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
}

function Add-NoticePackage {
    param(
        [Parameter(Mandatory)] [Text.StringBuilder] $Builder,
        [Parameter(Mandatory)] [object] $Package
    )

    $separator = '=' * 79
    [void] $Builder.AppendLine($separator)
    [void] $Builder.AppendLine("$($Package.Ecosystem): $($Package.Name) $($Package.Version)")
    [void] $Builder.AppendLine("Used by: $($Package.Surface)")
    [void] $Builder.AppendLine("Declared license: $($Package.License)")
    if (-not [string]::IsNullOrWhiteSpace($Package.Authors)) {
        [void] $Builder.AppendLine("Declared authors: $($Package.Authors)")
    }
    if (-not [string]::IsNullOrWhiteSpace($Package.Repository)) {
        [void] $Builder.AppendLine("Repository: $($Package.Repository)")
    }
    if (-not [string]::IsNullOrWhiteSpace($Package.Homepage)) {
        [void] $Builder.AppendLine("Homepage: $($Package.Homepage)")
    }
    [void] $Builder.AppendLine("Exact package source: $($Package.Source)")
    foreach ($licenseFile in @($Package.LicenseFiles)) {
        [void] $Builder.AppendLine()
        [void] $Builder.AppendLine("--- BEGIN $($licenseFile.Name) ---")
        $text = [IO.File]::ReadAllText($licenseFile.Path)
        [void] $Builder.AppendLine($text.TrimEnd())
        [void] $Builder.AppendLine("--- END $($licenseFile.Name) ---")
    }
    [void] $Builder.AppendLine()
}

function New-MplSourceArchive {
    param(
        [Parameter(Mandatory)] [object[]] $Packages,
        [Parameter(Mandatory)] [string] $ArchivePath
    )

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if ([IO.File]::Exists($ArchivePath)) {
        [IO.File]::Delete($ArchivePath)
    }
    $archive = [IO.Compression.ZipFile]::Open(
        $ArchivePath,
        [IO.Compression.ZipArchiveMode]::Create
    )
    try {
        $readme = $archive.CreateEntry('README.txt', [IO.Compression.CompressionLevel]::Optimal)
        $writer = [IO.StreamWriter]::new(
            $readme.Open(),
            [Text.UTF8Encoding]::new($false)
        )
        try {
            $writer.WriteLine('This archive contains the exact unmodified source trees of MPL-2.0')
            $writer.WriteLine('components distributed in OpenMat. Each component retains its own')
            $writer.WriteLine('license and copyright notices. OpenMat itself is not licensed under MPL.')
        } finally {
            $writer.Dispose()
        }

        foreach ($package in $Packages | Sort-Object Name, Version -Unique) {
            $prefix = "$($package.Name)-$($package.Version)"
            foreach ($file in Get-ChildItem -LiteralPath $package.PackageRoot -File -Recurse -Force) {
                $relative = [IO.Path]::GetRelativePath($package.PackageRoot, $file.FullName)
                $entryName = "$prefix/$($relative.Replace('\', '/'))"
                [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $archive,
                    $file.FullName,
                    $entryName,
                    [IO.Compression.CompressionLevel]::Optimal
                ) | Out-Null
            }
        }
    } finally {
        $archive.Dispose()
    }
}

$desktopPackages = @(Get-CargoReleasePackages `
    -ManifestPath $desktopManifest `
    -Target $DesktopTarget `
    -Surface 'Windows desktop executable')
$serverPackages = @()
if ($IncludeServer) {
    $serverPackages = @(Get-CargoReleasePackages `
        -ManifestPath (Join-Path $repository 'crates\openmat-server\Cargo.toml') `
        -Target $DesktopTarget `
        -Surface 'Windows standalone Rust server')
}
$wasmPackages = @(Get-CargoReleasePackages `
    -ManifestPath $wasmManifest `
    -Target $WasmTarget `
    -Surface 'Plot Engine WebAssembly')
$npmPackages = @(Get-NpmProductionPackages -WebDirectory $webRoot)

$packageByKey = [Collections.Generic.Dictionary[string, object]]::new(
    [StringComparer]::Ordinal
)
foreach ($package in @($desktopPackages) + @($serverPackages) + @($wasmPackages) + @($npmPackages)) {
    $key = "$($package.Ecosystem):$($package.Name)@$($package.Version)"
    if ($packageByKey.ContainsKey($key)) {
        $existing = $packageByKey[$key]
        $surfaces = @($existing.Surface -split '; ') + @($package.Surface)
        $existing.Surface = ($surfaces | Sort-Object -Unique) -join '; '
    } else {
        $packageByKey[$key] = $package
    }
}
$packages = @($packageByKey.Values | Sort-Object Ecosystem, Name, Version)
if ($packages.Count -lt 100) {
    throw "License inventory unexpectedly contains only $($packages.Count) packages."
}

$notices = [Text.StringBuilder]::new()
[void] $notices.AppendLine('OPENMAT THIRD-PARTY SOFTWARE NOTICES')
[void] $notices.AppendLine()
[void] $notices.AppendLine('OpenMat is licensed under GNU AGPL version 3 only. These notices cover the')
[void] $notices.AppendLine('identified third-party components, not to OpenMat as a whole. This file')
[void] $notices.AppendLine('is generated from the locked Windows desktop, Plot Engine WebAssembly,')
[void] $notices.AppendLine('and production web dependency graphs.')
[void] $notices.AppendLine('Separate exact notices are included for OpenBLAS, HDF5, stock zlib, NSIS,')
[void] $notices.AppendLine('Monaco Editor and KaTeX, including Monaco upstream third-party notices.')
[void] $notices.AppendLine()
[void] $notices.AppendLine("Cargo desktop packages: $($desktopPackages.Count)")
if ($IncludeServer) { [void] $notices.AppendLine("Cargo standalone server packages: $(@($serverPackages).Count)") }
[void] $notices.AppendLine("Cargo Plot WASM packages: $($wasmPackages.Count)")
[void] $notices.AppendLine("npm production packages: $($npmPackages.Count)")
[void] $notices.AppendLine("Unique third-party package versions: $($packages.Count)")
[void] $notices.AppendLine()
[void] $notices.AppendLine('For packages offering a choice of licenses, this distribution relies on')
[void] $notices.AppendLine('the permissive MIT, Apache-2.0, BSD, ISC, Zlib, CC0, or Unlicense option')
[void] $notices.AppendLine('where the declared SPDX expression permits that choice. Expressions using')
[void] $notices.AppendLine('AND remain subject to every named license. MPL-2.0 components are handled')
[void] $notices.AppendLine('by the accompanying source archive and source-availability notice.')
[void] $notices.AppendLine()
foreach ($package in $packages) {
    Add-NoticePackage -Builder $notices -Package $package
}
Write-Utf8File `
    -Path (Join-Path $output 'THIRD-PARTY-NOTICES.txt') `
    -Content $notices.ToString()

$mplPackages = @($packages | Where-Object { $_.License -match '(^|\W)MPL-2\.0($|\W)' })
if ($mplPackages.Count -eq 0) {
    throw 'The release inventory unexpectedly contains no MPL-2.0 components.'
}
$mplArchivePath = Join-Path $output 'MPL-2.0-SOURCE.zip'
New-MplSourceArchive -Packages $mplPackages -ArchivePath $mplArchivePath
$mplOffer = [Text.StringBuilder]::new()
[void] $mplOffer.AppendLine('MPL-2.0 SOURCE CODE AVAILABILITY')
[void] $mplOffer.AppendLine()
[void] $mplOffer.AppendLine('OpenMat includes the following components under Mozilla Public License 2.0:')
foreach ($package in $mplPackages | Sort-Object Name, Version -Unique) {
    [void] $mplOffer.AppendLine("- $($package.Name) $($package.Version): $($package.Source)")
}
[void] $mplOffer.AppendLine()
[void] $mplOffer.AppendLine('The exact unmodified source trees distributed with this OpenMat release are')
[void] $mplOffer.AppendLine('included in MPL-2.0-SOURCE.zip in this directory. The MPL applies only to')
[void] $mplOffer.AppendLine('those components and does not license OpenMat source code.')
Write-Utf8File `
    -Path (Join-Path $output 'MPL-2.0-SOURCE-OFFER.txt') `
    -Content $mplOffer.ToString()

Copy-Item -LiteralPath $openBlasLicensePath `
    -Destination (Join-Path $output 'OpenBLAS-LICENSE.txt') -Force
Copy-Item -LiteralPath $nsisLicensePath `
    -Destination (Join-Path $output 'NSIS-COPYING.txt') -Force

$hdf5Package = $packages | Where-Object { $_.Name -eq 'hdf5-metno-src' } | Select-Object -First 1
if ($null -eq $hdf5Package) {
    throw 'The release inventory does not contain hdf5-metno-src.'
}
$hdf5License = Join-Path $hdf5Package.PackageRoot 'ext\hdf5\LICENSE'
Copy-Item -LiteralPath $hdf5License `
    -Destination (Join-Path $output 'HDF5-LICENSE.txt') -Force

$zlibPackage = $packages | Where-Object { $_.Name -eq 'libz-sys' } | Select-Object -First 1
if ($null -eq $zlibPackage) {
    throw 'The release inventory does not contain libz-sys.'
}
$zlibLicense = Join-Path $zlibPackage.PackageRoot 'src\zlib\LICENSE'
Copy-Item -LiteralPath $zlibLicense `
    -Destination (Join-Path $output 'ZLIB-LICENSE.txt') -Force

$monacoPackage = $npmPackages | Where-Object { $_.Name -eq 'monaco-editor' } | Select-Object -First 1
if ($null -eq $monacoPackage) {
    throw 'The production web inventory does not contain monaco-editor.'
}
Copy-Item -LiteralPath (Join-Path $monacoPackage.PackageRoot 'LICENSE') `
    -Destination (Join-Path $output 'MONACO-LICENSE.txt') -Force
Copy-Item -LiteralPath (Join-Path $monacoPackage.PackageRoot 'ThirdPartyNotices.txt') `
    -Destination (Join-Path $output 'MONACO-THIRD-PARTY-NOTICES.txt') -Force

$katexPackage = $npmPackages | Where-Object { $_.Name -eq 'katex' } | Select-Object -First 1
if ($null -eq $katexPackage) {
    throw 'The production web inventory does not contain katex.'
}
Copy-Item -LiteralPath (Join-Path $katexPackage.PackageRoot 'LICENSE') `
    -Destination (Join-Path $output 'KATEX-LICENSE.txt') -Force

$manifestPackages = @(
    $packages | ForEach-Object {
        [ordered]@{
            ecosystem = $_.Ecosystem
            name = $_.Name
            version = $_.Version
            license = $_.License
            surface = $_.Surface
            source = $_.Source
            repository = $_.Repository
        }
    }
)
$artifactHashes = [ordered]@{}
foreach ($name in $generatedNames | Where-Object { $_ -ne 'THIRD-PARTY-MANIFEST.json' }) {
    $path = Join-Path $output $name
    if (-not [IO.File]::Exists($path) -or (Get-Item -LiteralPath $path).Length -eq 0) {
        throw "Expected generated license artifact '$path' is missing or empty."
    }
    $artifactHashes[$name] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}
$manifest = [ordered]@{
    schemaVersion = 1
    target = $DesktopTarget
    wasmTarget = $WasmTarget
    packageCount = $packages.Count
    packages = $manifestPackages
    artifacts = $artifactHashes
}
Write-Utf8File `
    -Path (Join-Path $output 'THIRD-PARTY-MANIFEST.json') `
    -Content ($manifest | ConvertTo-Json -Depth 8)

Write-Host ((
        "Generated third-party licenses for {0} unique packages ({1} desktop Cargo, " +
        "{2} Plot WASM Cargo, {3} npm) in '{4}'."
    ) -f
        $packages.Count,
        $desktopPackages.Count,
        $wasmPackages.Count,
        $npmPackages.Count,
        $output
)
