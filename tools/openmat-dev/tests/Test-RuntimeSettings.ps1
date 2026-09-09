Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$reader = Join-Path $PSScriptRoot '../Read-OpenMatRuntimeSettings.ps1'
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$directory = Join-Path $temporaryRoot ('openmat-settings-test-' + [Guid]::NewGuid().ToString('N'))
$file = Join-Path $directory '配置/runtime.json'
try {
    $defaults = & $reader -ConfigPath $file
    if ($defaults.KernelPort -ne 42000) { throw 'Expected the persisted default port.' }
    $changed = '{"schemaVersion":1,"kernelPort":42123}'
    [IO.File]::WriteAllText($file, $changed)
    foreach ($restart in 1..2) {
        if ((& $reader -ConfigPath $file).KernelPort -ne 42123) { throw 'Configuration changed after restart.' }
        if ([IO.File]::ReadAllText($file) -ne $changed) { throw 'Existing configuration was overwritten.' }
    }
    foreach ($invalid in @(
        '{}', '{', '{"schemaVersion":2,"kernelPort":42000}',
        '{"schemaVersion":1,"kernelPort":0}', '{"schemaVersion":1,"kernelPort":65536}',
        '{"schemaVersion":1,"kernelPort":-1}', '{"schemaVersion":1,"kernelPort":"42000"}',
        '{"schemaVersion":1,"kernelPort":42.5}', '{"schemaVersion":1,"kernelPort":42000,"port":42123}'
    )) {
        [IO.File]::WriteAllText($file, $invalid)
        $rejected = $false
        try { & $reader -ConfigPath $file | Out-Null } catch { $rejected = $true }
        if (-not $rejected) { throw 'Accepted invalid settings.' }
        if ([IO.File]::ReadAllText($file) -ne $invalid) { throw 'Invalid settings were overwritten.' }
    }
    Write-Output 'Runtime settings tests passed: default, edited port across restarts, invalid configuration preservation.'
}
finally {
    $resolved = [IO.Path]::GetFullPath($directory)
    if (-not $resolved.StartsWith($temporaryRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to remove a test directory outside the temporary root.'
    }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
