[CmdletBinding()]
param(
    [Parameter(Mandatory)] [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string] $Installer,
    [string] $PreviousInstaller,
    [string] $OutputDirectory,
    [ValidateRange(60, 3600)] [int] $TimeoutSeconds = 900,
    [switch] $PrepareOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$sandboxCommand = Get-Command WindowsSandbox.exe -ErrorAction Stop
$version = [string] ((Get-Content -LiteralPath (Join-Path $repository 'apps\desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version)
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repository 'output\installer-tests'
}
$runDirectory = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) ([guid]::NewGuid().ToString('N'))
$inputDirectory = Join-Path $runDirectory 'input'
$resultDirectory = Join-Path $runDirectory 'result'
New-Item -ItemType Directory -Path $inputDirectory, $resultDirectory -Force | Out-Null
Copy-Item -LiteralPath $Installer -Destination (Join-Path $inputDirectory 'current-setup.exe')
foreach ($script in @('Test-OpenMatWindowsInstall.ps1', 'Test-OpenMatDesktop.ps1')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $script) -Destination $inputDirectory
}
$guestCommand = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\OpenMatAcceptance\Input\Test-OpenMatWindowsInstall.ps1 -Installer C:\OpenMatAcceptance\Input\current-setup.exe -ResultDirectory C:\OpenMatAcceptance\Result -ExpectedVersion ' + $version
if (-not [string]::IsNullOrWhiteSpace($PreviousInstaller)) {
    Copy-Item -LiteralPath $PreviousInstaller -Destination (Join-Path $inputDirectory 'previous-setup.exe')
    $guestCommand += ' -PreviousInstaller C:\OpenMatAcceptance\Input\previous-setup.exe'
}
$inputXml = [Security.SecurityElement]::Escape($inputDirectory)
$outputXml = [Security.SecurityElement]::Escape($resultDirectory)
$commandXml = [Security.SecurityElement]::Escape($guestCommand)
$configuration = @"
<Configuration>
  <vGPU>Disable</vGPU>
  <Networking>Enable</Networking>
  <AudioInput>Disable</AudioInput>
  <ClipboardRedirection>Disable</ClipboardRedirection>
  <MemoryInMB>4096</MemoryInMB>
  <MappedFolders>
    <MappedFolder><HostFolder>$inputXml</HostFolder><SandboxFolder>C:\OpenMatAcceptance\Input</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$outputXml</HostFolder><SandboxFolder>C:\OpenMatAcceptance\Result</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
  </MappedFolders>
  <LogonCommand><Command>$commandXml</Command></LogonCommand>
</Configuration>
"@
$configurationPath = Join-Path $runDirectory 'acceptance.wsb'
$configuration | Set-Content -LiteralPath $configurationPath -Encoding utf8
Write-Output "Sandbox configuration: $configurationPath"
Write-Output "Sandbox results: $resultDirectory"
if ($PrepareOnly) { return }

$sandbox = Start-Process -FilePath $sandboxCommand.Source -ArgumentList ('"' + $configurationPath + '"') -WindowStyle Hidden -PassThru
$resultPath = Join-Path $resultDirectory 'result.json'
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while (-not (Test-Path -LiteralPath $resultPath)) {
    if ($sandbox.HasExited -and $sandbox.ExitCode -ne 0) {
        throw "Windows Sandbox could not start (exit $($sandbox.ExitCode)). Configuration: $configurationPath"
    }
    if ((Get-Date) -ge $deadline) {
        throw "Sandbox acceptance timed out. Inspect $resultDirectory and the Sandbox window."
    }
    Start-Sleep -Milliseconds 500
}
# The guest publishes its result before shutting down its disposable session.
$result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
Write-Output "Sandbox status: $($result.status); completed checks: $(@($result.checks).Count)"
if ($result.status -ne 'passed') { throw "Sandbox acceptance failed: $($result.error). See $resultDirectory" }
