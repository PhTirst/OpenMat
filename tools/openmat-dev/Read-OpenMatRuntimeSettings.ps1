[CmdletBinding()]
param([string] $ConfigPath = $env:OPENMAT_RUNTIME_CONFIG)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ConfigPath)) {
    $ConfigPath = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'org.openmat.desktop/runtime.json'
}
$resolved = [IO.Path]::GetFullPath($ConfigPath)
if (-not [IO.File]::Exists($resolved)) {
    $parent = [IO.Path]::GetDirectoryName($resolved)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = Join-Path $parent ('.runtime-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        $defaults = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../../apps/desktop/runtime.default.json'))
        [IO.File]::WriteAllText($temporary, $defaults, [Text.UTF8Encoding]::new($false))
        try { [IO.File]::Move($temporary, $resolved, $false) }
        catch { if (-not [IO.File]::Exists($resolved)) { throw } }
    }
    finally {
        if ([IO.File]::Exists($temporary)) { Remove-Item -LiteralPath $temporary }
    }
}
try {
    $settings = [IO.File]::ReadAllText($resolved) | ConvertFrom-Json -AsHashtable
    if ($settings -isnot [System.Collections.IDictionary] -or
        $settings.Count -ne 2 -or
        $settings.Keys -cnotcontains 'schemaVersion' -or
        $settings.Keys -cnotcontains 'kernelPort') {
        throw 'Expected schemaVersion and kernelPort, with no unknown properties.'
    }
    if ($settings.schemaVersion -isnot [long] -and $settings.schemaVersion -isnot [int] -or $settings.schemaVersion -ne 1) {
        throw 'Unsupported runtime settings schemaVersion; expected 1.'
    }
    if (($settings.kernelPort -isnot [long] -and $settings.kernelPort -isnot [int]) -or
        $settings.kernelPort -lt 1 -or $settings.kernelPort -gt 65535) {
        throw 'kernelPort must be an integer between 1 and 65535.'
    }
    [pscustomobject] @{ ConfigPath = $resolved; KernelPort = [int] $settings.kernelPort }
}
catch {
    throw "Could not load runtime settings ${resolved}: $($_.Exception.Message)"
}
