[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $ChildPidPath,

    [string] $FirstLine
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = (Get-Process -Id $PID).Path
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
[void] $startInfo.ArgumentList.Add('-NoProfile')
[void] $startInfo.ArgumentList.Add('-NonInteractive')
[void] $startInfo.ArgumentList.Add('-Command')
[void] $startInfo.ArgumentList.Add('Start-Sleep -Seconds 300')

$child = [System.Diagnostics.Process]::new()
$child.StartInfo = $startInfo
if (-not $child.Start()) {
    throw 'Could not start fixture child process'
}
[System.IO.File]::WriteAllText($ChildPidPath, [string] $child.Id)

if (-not [string]::IsNullOrEmpty($FirstLine)) {
    [Console]::Out.WriteLine($FirstLine)
    [Console]::Out.Flush()
}

try {
    while (-not $child.HasExited) {
        Start-Sleep -Milliseconds 100
    }
}
finally {
    if (-not $child.HasExited) {
        $child.Kill($true)
        [void] $child.WaitForExit(5000)
    }
    $child.Dispose()
}
