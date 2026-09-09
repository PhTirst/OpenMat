[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Invoke-CargoStep {
    param(
        [Parameter(Mandatory)]
        [string[]] $Arguments
    )

    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Invoke-CargoStep -Arguments @('fmt', '--all', '--check')
Invoke-CargoStep -Arguments @('check', '--workspace')
Invoke-CargoStep -Arguments @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
Invoke-CargoStep -Arguments @('test', '--workspace')

