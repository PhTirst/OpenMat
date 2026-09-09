[CmdletBinding()]
param(
    [string] $ServerPath = (Join-Path $PSScriptRoot '.build/debug/openmat-server.exe'),
    [string] $Entry = 'run_tests.m',
    [ValidateRange(1, 3600)] [int] $TimeoutSeconds = 180
)
$ErrorActionPreference = 'Stop'
$server = (Resolve-Path -LiteralPath $ServerPath).Path
$entryPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot $Entry)).Path
$rootLiteral = $PSScriptRoot.Replace('\', '/').Replace("'", "''")
$entryCode = Get-Content -LiteralPath $entryPath -Raw
$session = 'fem-tests-' + [guid]::NewGuid().ToString('N')
function New-Request([string] $Id, [string] $Type, [hashtable] $Params) {
    return @{
        protocol = 'openmat-kernel-v0'; sessionId = $session; messageId = $Id
        kind = 'request'; request = @{ type = $Type; params = $Params }
    } | ConvertTo-Json -Depth 12 -Compress
}
$frames = @(
    (New-Request 'initialize' 'initialize' @{
        client = @{ name = 'fem-m-tests'; version = '1' }
        supportedProtocols = @('openmat-kernel-v0')
        capabilities = @{
            executionModes = @('repl', 'file'); displayMimeTypes = @('text/plain')
            maxPreviewElements = 16; interrupt = $false; workspaceDelta = $false
        }
    }),
    (New-Request 'path' 'execute' @{
        code = "addpath('$rootLiteral');"
        sourceName = 'fem-verification-path.m'; mode = 'repl'
    }),
    (New-Request 'execute' 'execute' @{
        code = $entryCode
        sourceName = $entryPath; mode = 'file'
    }),
    (New-Request 'shutdown' 'shutdown' @{})
)
$info = [System.Diagnostics.ProcessStartInfo]::new()
$info.FileName = $server
$info.ArgumentList.Add('--stdio')
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.WorkingDirectory = $PSScriptRoot
$info.RedirectStandardInput = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $info
$started = $false
try {
    $started = $process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    foreach ($frame in $frames) { $process.StandardInput.WriteLine($frame) }
    $process.StandardInput.Close()
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        throw "FEM execution exceeded $TimeoutSeconds seconds."
    }
    $outputText = $stdout.GetAwaiter().GetResult()
    $errorText = $stderr.GetAwaiter().GetResult()
    if ($errorText) { Write-Host $errorText }
    if ($process.ExitCode -ne 0) { throw "Server exited with code $($process.ExitCode): $outputText" }
    $executed = $false
    foreach ($line in ($outputText -split '\r?\n')) {
        if (-not $line.Trim()) { continue }
        $message = $line | ConvertFrom-Json
        if ($message.kind -eq 'event' -and $message.event.type -eq 'stream') {
            Write-Host -NoNewline $message.event.data.text
        }
        if ($message.kind -eq 'event' -and $message.event.type -eq 'display') {
            Write-Host $message.event.data.representations.'text/plain'
        }
        if ($message.kind -eq 'response') {
            if (-not $message.ok) { throw ($message.error | ConvertTo-Json -Depth 16) }
            if ($message.replyTo -eq 'execute') {
                if ($message.result.data.interrupted) { throw 'FEM execution was interrupted.' }
                $executed = $true
            }
        }
    }
    if (-not $executed) { throw 'No successful execute response received.' }
    Write-Host 'OpenMat FEM verification passed.'
} finally {
    if ($started -and -not $process.HasExited) { $process.Kill($true) }
    $process.Dispose()
}
