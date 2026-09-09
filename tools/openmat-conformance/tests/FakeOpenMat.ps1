#Requires -Version 7.0

[CmdletBinding()]
param(
    [Parameter(Position = 0)] [string] $Command,
    [Parameter(Position = 1)] [string] $ManifestPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Command -cne 'conformance' -or [string]::IsNullOrWhiteSpace($ManifestPath)) {
    [Console]::Error.WriteLine('unexpected fake CLI arguments')
    exit 64
}

$resolvedManifest = (Resolve-Path -LiteralPath $ManifestPath).Path
$caseId = [System.IO.Path]::GetFileNameWithoutExtension($resolvedManifest)
switch ($caseId) {
    'floating_tolerance' {
        [Console]::Error.WriteLine('fixture internal failure')
        exit 70
    }
    'matrix_shape' {
        $observation = [ordered]@{
            schema_version = 1
            case_id = $caseId
            oracle = [ordered]@{ name = 'OpenMat fixture'; release = 'test' }
            outcome = 'error'
            error = [ordered]@{ category = 'other' }
        }
        [Console]::Out.WriteLine(
            (ConvertTo-Json -InputObject $observation -Depth 10 -Compress)
        )
        exit 0
    }
    'class_constructor' {
        $observation = [ordered]@{
            schema_version = 1
            case_id = $caseId
            oracle = [ordered]@{ name = 'OpenMat fixture'; release = 'test' }
            outcome = 'error'
            error = [ordered]@{ category = 'unsupported-payload' }
        }
        [Console]::Out.WriteLine(
            (ConvertTo-Json -InputObject $observation -Depth 10 -Compress)
        )
        exit 0
    }
    'runner_v2_cli_v1_only' {
        [Console]::Error.WriteLine(
            'error[cli.manifest]: schema_version must be 1'
        )
        exit 64
    }
    'runner_v2_unrelated_unsupported' {
        [Console]::Error.WriteLine(
            'execution fault: unsupported operand corrupted the request'
        )
        exit 70
    }
    'runner_v2_capability_unavailable' {
        [Console]::Error.WriteLine(
            'error[cli.capability]: required openmat-kernel-v1 is unavailable'
        )
        exit 69
    }
    'runner_v2_mixed_observation' {
        $observation = [ordered]@{
            schema_version = 2
            case_id = $caseId
            oracle = [ordered]@{ name = 'OpenMat fixture'; release = 'test' }
            outcome = 'ok'
            value = [ordered]@{
                class = 'string'
                size = @(1, 3)
                ndims = 2
                numel = 3
                kind = 'string'
                string = @('legacy', '', '')
                missing = @($false, $false, $true)
            }
        }
        [Console]::Out.WriteLine(
            (ConvertTo-Json -InputObject $observation -Depth 10 -Compress)
        )
        exit 0
    }
    default {
        $manifestDirectory = [System.IO.Path]::GetDirectoryName($resolvedManifest)
        $caseDirectory = [System.IO.Path]::GetDirectoryName($manifestDirectory)
        $conformanceDirectory = [System.IO.Path]::GetDirectoryName($caseDirectory)
        $referencePath = Join-Path `
            $conformanceDirectory "reference/matlab-r2022b/$caseId.json"
        if ($caseId -ceq 'runner_v2_schema1_observation') {
            $observation = [System.IO.File]::ReadAllText($referencePath) |
                ConvertFrom-Json -Depth 100
            $observation.schema_version = 1
            [Console]::Out.WriteLine(
                (ConvertTo-Json -InputObject $observation -Depth 100 -Compress)
            )
        } else {
            [Console]::Out.Write(
                [System.IO.File]::ReadAllText($referencePath)
            )
        }
        exit 0
    }
}
