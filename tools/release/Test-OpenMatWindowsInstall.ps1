[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Installer,
    [Parameter(Mandatory)] [string] $ExpectedVersion,
    [Parameter(Mandatory)] [string] $ResultDirectory,
    [string] $PreviousInstaller
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# This script installs and uninstalls the real product. Never run it on the host.
if ($env:USERNAME -ne 'WDAGUtilityAccount') {
    throw 'Run this acceptance script only inside Windows Sandbox through Test-OpenMatInstallerSandbox.ps1.'
}

$registryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenMat'
$installDirectory = Join-Path $env:LOCALAPPDATA 'OpenMat'
$configurationDirectory = Join-Path $env:APPDATA 'org.openmat.desktop'
$runtimeConfig = Join-Path $configurationDirectory 'runtime.json'
$documentsDirectory = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'OpenMat'
$checks = New-Object 'System.Collections.Generic.List[string]'
$smokeScript = Join-Path $PSScriptRoot 'Test-OpenMatDesktop.ps1'
$result = [ordered]@{
    status = 'running'
    windows = [Environment]::OSVersion.VersionString
    version = $ExpectedVersion
    installerSha256 = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash.ToLowerInvariant()
    previousInstallerSha256 = $null
    webView2Before = $null
    webView2After = $null
    checks = @()
    error = $null
}
New-Item -ItemType Directory -Path $ResultDirectory -Force | Out-Null
Start-Transcript -LiteralPath (Join-Path $ResultDirectory 'install-transcript.txt') | Out-Null

function Confirm-Check {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
    $checks.Add($Message)
    Write-Host "PASS $Message"
    [IO.File]::AppendAllText((Join-Path $ResultDirectory 'progress.log'), "PASS $Message`n")
}

function Get-WebView2Version {
    foreach ($key in @(
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    )) {
        $version = Get-ItemPropertyValue -LiteralPath $key -Name pv -ErrorAction SilentlyContinue
        if ($version -and [version] $version -gt [version] '0.0.0.0') { return $version }
    }
    return $null
}

function Invoke-InstallerProcess {
    param([string] $Path)
    $process = Start-Process -FilePath $Path -ArgumentList '/S' -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(300000)) {
        Stop-Process -Id $process.Id -Force
        throw "Installer process timed out: $Path"
    }
    if ($process.ExitCode -ne 0) { throw "Installer process exited with code $($process.ExitCode): $Path" }
}

function Confirm-InstalledResources {
    $entry = Get-ItemProperty -LiteralPath $registryPath
    Confirm-Check ($entry.DisplayVersion -eq $ExpectedVersion) "Installed version is $ExpectedVersion"
    foreach ($relative in @(
        'OpenMat.exe', 'uninstall.exe',
        'resources\openblas\libopenblas.dll',
        'resources\licenses\OPENMAT-AGPL-3.0.txt',
        'resources\licenses\THIRD-PARTY-NOTICES.txt',
        'resources\licenses\THIRD-PARTY-MANIFEST.json',
        'resources\licenses\MPL-2.0-SOURCE.zip'
    )) {
        $file = Join-Path $installDirectory $relative
        Confirm-Check ((Test-Path -LiteralPath $file -PathType Leaf) -and (Get-Item -LiteralPath $file).Length -gt 0) "Installed resource exists: $relative"
    }
    foreach ($obsolete in @(
        'openmat-server.exe',
        'resources\licenses\OpenMat-LICENSE-MIT.txt',
        'resources\licenses\OpenMat-LICENSE-APACHE.txt'
    )) {
        Confirm-Check (-not (Test-Path -LiteralPath (Join-Path $installDirectory $obsolete))) "No obsolete installed file: $obsolete"
    }
    $developmentFiles = @(Get-ChildItem -LiteralPath $installDirectory -Recurse -File |
        Where-Object { $_.Extension -in @('.pdb', '.lib', '.exp', '.map') })
    Confirm-Check ($developmentFiles.Count -eq 0) 'No debug or development files in installation'
}

function Invoke-InstalledSmoke {
    param([int] $Port)
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $smokeScript `
        -Executable (Join-Path $installDirectory 'OpenMat.exe') `
        -KernelPort $Port -RuntimeConfig $runtimeConfig -UseDefaultWorkspace
    if ($LASTEXITCODE -ne 0) { throw "Installed runtime smoke failed on configured port $Port." }
    Confirm-Check $true "Installed runtime starts and computes on configured port $Port"
}

function Uninstall-OpenMat {
    Invoke-InstallerProcess (Join-Path $installDirectory 'uninstall.exe')
    # NSIS can relaunch its uninstaller from TEMP before the parent exits.
    $deadline = (Get-Date).AddSeconds(30)
    while (((Test-Path -LiteralPath $registryPath) -or
        (Test-Path -LiteralPath (Join-Path $installDirectory 'OpenMat.exe'))) -and
        (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 200
    }
    Confirm-Check (-not (Test-Path -LiteralPath $registryPath)) 'Uninstall removes its registry entry'
    Confirm-Check (-not (Test-Path -LiteralPath (Join-Path $installDirectory 'OpenMat.exe'))) 'Uninstall removes the application executable'
}

try {
    Confirm-Check (-not (Test-Path -LiteralPath $registryPath)) 'Sandbox starts without an existing OpenMat installation'
    Confirm-Check (-not (Test-Path -LiteralPath $installDirectory)) 'Sandbox starts without an OpenMat install directory'
    $developmentTools = @(Get-Command cargo, rustc, node, pnpm -ErrorAction SilentlyContinue)
    Confirm-Check ($developmentTools.Count -eq 0) 'Sandbox has no Rust or Node development toolchain'
    $result.webView2Before = Get-WebView2Version
    Invoke-InstallerProcess $Installer
    $result.webView2After = Get-WebView2Version
    Confirm-Check (-not [string]::IsNullOrWhiteSpace($result.webView2After)) 'WebView2 Runtime is available after installation'
    Confirm-InstalledResources
    Invoke-InstalledSmoke 42000
    $settings = Get-Content -LiteralPath $runtimeConfig -Raw | ConvertFrom-Json
    Confirm-Check ($settings.schemaVersion -eq 1 -and $settings.kernelPort -eq 42000) 'First startup creates the default fixed-port configuration'
    Confirm-Check (Test-Path -LiteralPath $documentsDirectory -PathType Container) 'First startup creates the Documents workspace'
    Invoke-InstalledSmoke 42000

    $settings.kernelPort = 42001
    $settings | ConvertTo-Json | Set-Content -LiteralPath $runtimeConfig -Encoding utf8
    $configurationHash = (Get-FileHash -LiteralPath $runtimeConfig).Hash
    $documentMarker = Join-Path $documentsDirectory 'installer-acceptance.m'
    $dataMarker = Join-Path $configurationDirectory 'installer-acceptance.txt'
    Set-Content -LiteralPath $documentMarker -Value 'answer = 42;' -Encoding utf8
    Set-Content -LiteralPath $dataMarker -Value 'Preserve user data across install and uninstall.' -Encoding utf8
    $documentHash = (Get-FileHash -LiteralPath $documentMarker).Hash
    $dataHash = (Get-FileHash -LiteralPath $dataMarker).Hash
    Invoke-InstalledSmoke 42001

    Invoke-InstallerProcess $Installer
    Confirm-InstalledResources
    Confirm-Check ((Get-FileHash -LiteralPath $runtimeConfig).Hash -eq $configurationHash) 'Reinstall preserves the edited port configuration'
    Invoke-InstalledSmoke 42001
    Uninstall-OpenMat

    if (-not [string]::IsNullOrWhiteSpace($PreviousInstaller)) {
        $result.previousInstallerSha256 = (Get-FileHash -LiteralPath $PreviousInstaller).Hash.ToLowerInvariant()
        Invoke-InstallerProcess $PreviousInstaller
        $oldEntry = Get-ItemProperty -LiteralPath $registryPath
        Confirm-Check ([version] $oldEntry.DisplayVersion -lt [version] $ExpectedVersion) 'Upgrade starts from an older installed version'
        Invoke-InstallerProcess $Installer
        Confirm-InstalledResources
        Confirm-Check ((Get-FileHash -LiteralPath $runtimeConfig).Hash -eq $configurationHash) 'Upgrade preserves the edited port configuration'
        Invoke-InstalledSmoke 42001
        Uninstall-OpenMat
    }

    Confirm-Check ((Get-FileHash -LiteralPath $runtimeConfig).Hash -eq $configurationHash) 'Uninstall preserves the user port configuration'
    Confirm-Check ((Get-FileHash -LiteralPath $documentMarker).Hash -eq $documentHash) 'Install, upgrade and uninstall preserve user documents'
    Confirm-Check ((Get-FileHash -LiteralPath $dataMarker).Hash -eq $dataHash) 'Install, upgrade and uninstall preserve application user data'
    $result.status = 'passed'
}
catch {
    $result.status = 'failed'
    $result.error = $_.Exception.Message
    Write-Host "FAIL $($result.error)"
}
finally {
    $result.checks = @($checks.ToArray())
    foreach ($dataRoot in @($configurationDirectory, (Join-Path $env:LOCALAPPDATA 'org.openmat.desktop'))) {
        $serverLog = Join-Path $dataRoot 'logs\openmat-server.log'
        if (Test-Path -LiteralPath $serverLog) { Copy-Item -LiteralPath $serverLog -Destination $ResultDirectory -Force -ErrorAction Continue }
    }
    Stop-Transcript | Out-Null
    $pendingResult = Join-Path $ResultDirectory 'result.pending.json'
    $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $pendingResult -Encoding utf8
    Move-Item -LiteralPath $pendingResult -Destination (Join-Path $ResultDirectory 'result.json')
    # The guard at script entry restricts this shutdown to the disposable guest.
    & shutdown.exe /s /t 3
}
if ($result.status -ne 'passed') { exit 1 }
