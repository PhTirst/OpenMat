# Generates only OpenMat-authored models using public MATLAB/Simulink APIs.
# The generated packages and observations are local artifacts, never fixtures
# to check into public source. Run explicit Rust oracle tests after generation.
[CmdletBinding()]
param(
    [string]$MatlabPath = 'matlab',
    [string]$OutputDirectory,
    [ValidateSet('legacy', 'control', 'multirate', 'hybrid')][string]$Profile = 'legacy',
    [ValidateRange(30, 3600)][int]$TimeoutSeconds = 600
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or -not $IsWindows) {
    throw 'This MATLAB launcher requires PowerShell 7 on Windows. Other hosts can call slx_oracle directly from MATLAB R2022b.'
}
$matlab = (Get-Command $MatlabPath -CommandType Application -ErrorAction Stop).Source
$toolDirectory = $PSScriptRoot
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path (Split-Path $PSScriptRoot -Parent) ('.openmat/slx-oracle-' + [guid]::NewGuid().ToString('N'))
}
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputPath) {
    throw 'OutputDirectory must be new, to prevent stale observations or overwriting existing files.'
}
New-Item -ItemType Directory -Path $outputPath | Out-Null
$oldDirectory = $env:OPENMAT_SLX_ORACLE_DIR
$oldTools = $env:OPENMAT_SLX_TOOL_DIR
try {
    $env:OPENMAT_SLX_ORACLE_DIR = $outputPath
    $env:OPENMAT_SLX_TOOL_DIR = $toolDirectory
    $oracleEntry = switch ($Profile) {
        'control' { 'slx_control_oracle' }
        'multirate' { 'slx_multirate_oracle' }
        'hybrid' { 'slx_hybrid_oracle' }
        default { 'slx_oracle' }
    }
    $oracleCommand = '"addpath(getenv(''OPENMAT_SLX_TOOL_DIR'')); ' + $oracleEntry + '(getenv(''OPENMAT_SLX_ORACLE_DIR''))"'
    $process = Start-Process -FilePath $matlab -ArgumentList @(
        '-wait', '-batch', $oracleCommand
    ) -WindowStyle Hidden -WorkingDirectory $outputPath -PassThru `
      -RedirectStandardOutput (Join-Path $outputPath 'matlab.stdout.txt') `
      -RedirectStandardError (Join-Path $outputPath 'matlab.stderr.txt')
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        # Only terminate the process tree started by this invocation.
        $process.Kill($true)
        throw 'OpenMat SLX oracle timed out.'
    }
    $process.Refresh()
    $statusPath = Join-Path $outputPath 'oracle-status.json'
    if (-not (Test-Path -LiteralPath $statusPath)) {
        throw 'MATLAB exited without completing the OpenMat oracle; inspect the local launcher logs.'
    }
    $status = Get-Content -LiteralPath $statusPath -Raw | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $status.ok -or $status.release -ne '2022b') {
        throw "OpenMat SLX oracle failed; inspect normalized status at $statusPath."
    }
    Write-Output $outputPath
} finally {
    $env:OPENMAT_SLX_ORACLE_DIR = $oldDirectory
    $env:OPENMAT_SLX_TOOL_DIR = $oldTools
}
