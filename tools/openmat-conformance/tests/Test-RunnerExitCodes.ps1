#Requires -Version 7.0

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Equal {
    param(
        [AllowNull()] [object] $Actual,
        [AllowNull()] [object] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if ([string] $Actual -cne [string] $Expected) {
        throw "$Label`: expected '$Expected', got '$Actual'."
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory)] [string] $Text,
        [Parameter(Mandatory)] [string] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if (-not $Text.Contains($Expected, [StringComparison]::Ordinal)) {
        throw "$Label`: output does not contain '$Expected'."
    }
}

function Write-TestJson {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [object] $Value
    )

    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText(
        $Path,
        (ConvertTo-Json -InputObject $Value -Depth 100),
        $encoding
    )
}

function Initialize-RunnerFixtures {
    $script:ConformanceRoot = Join-Path $script:TestRoot 'conformance'
    $manifestDirectory = Join-Path $script:ConformanceRoot 'cases/manifests'
    $referenceDirectory = Join-Path $script:ConformanceRoot `
        'reference/matlab-r2022b'
    $schemaDirectory = Join-Path $script:ConformanceRoot 'schema'
    foreach ($directory in @(
            $manifestDirectory, $referenceDirectory, $schemaDirectory
        )) {
        [void] (New-Item -ItemType Directory -Path $directory -Force)
    }

    Copy-Item -Path (
        Join-Path $script:RepositoryRoot 'tests/conformance/schema/*'
    ) -Destination $schemaDirectory
    foreach ($caseId in @(
            'scalar_shape', 'matrix_shape',
            'class_constructor', 'floating_tolerance'
        )) {
        Copy-Item -LiteralPath (
            Join-Path $script:RepositoryRoot `
                "tests/conformance/cases/manifests/$caseId.json"
        ) -Destination $manifestDirectory
        Copy-Item -LiteralPath (
            Join-Path $script:RepositoryRoot `
                "tests/conformance/reference/matlab-r2022b/$caseId.json"
        ) -Destination $referenceDirectory
    }

    foreach ($caseId in @(
            'runner_v2_pass',
            'runner_v2_mixed_observation',
            'runner_v2_schema1_observation',
            'runner_v2_cli_v1_only',
            'runner_v2_unrelated_unsupported',
            'runner_v2_capability_unavailable',
            'runner_v2_payload_limit',
            'runner_v2_cyclic_payload',
            'runner_v2_invalid_observation'
        )) {
        $value = [ordered]@{
            class = 'char'
            size = [object[]] @(1, 1)
            ndims = 2
            numel = 1
            complex = $false
            kind = 'char'
            code_units = [object[]] @(65)
        }
        $manifest = [ordered]@{
            schema_version = 2
            id = $caseId
            description = 'Self-contained schema-v2 runner fixture.'
            source = 'programs/runner_fixture.m'
            tags = [object[]] @('runner-fixture')
            expected = [ordered]@{ outcome = 'ok'; value = $value }
        }
        $syntheticCategory = switch ($caseId) {
            'runner_v2_payload_limit' { 'payload-limit' }
            'runner_v2_cyclic_payload' { 'cyclic-payload' }
            'runner_v2_invalid_observation' { 'invalid-observation' }
            default { $null }
        }
        $reference = if ($null -eq $syntheticCategory) {
            [ordered]@{
                schema_version = 2
                case_id = $caseId
                oracle = [ordered]@{ name = 'MATLAB fixture'; release = 'R2022b' }
                outcome = 'ok'
                value = $value
            }
        } else {
            [ordered]@{
                schema_version = 2
                case_id = $caseId
                oracle = [ordered]@{ name = 'OpenMat fixture'; release = 'test' }
                outcome = 'error'
                error = [ordered]@{ category = $syntheticCategory }
            }
        }
        Write-TestJson (Join-Path $manifestDirectory "$caseId.json") $manifest
        Write-TestJson (Join-Path $referenceDirectory "$caseId.json") $reference
    }
}

function Invoke-RunnerFixture {
    param(
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $Cases,
        [Parameter(Mandatory)] [int] $ExpectedExitCode,
        [Parameter(Mandatory)] [ValidateSet('Json', 'Normal')] [string] $Mode,
        [int] $Passed = 0,
        [int] $Failed = 0,
        [int] $Unsupported = 0,
        [int] $Internal = 0,
        [string[]] $DetailContains = @()
    )

    $resultDirectory = Join-Path $script:TestRoot "results-$Name"
    $stdoutPath = Join-Path $script:TestRoot "$Name.stdout"
    $stderrPath = Join-Path $script:TestRoot "$Name.stderr"
    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($argument in @(
            '-NoProfile', '-File', $script:RunnerPath,
            '-Case', $Cases,
            '-OpenMatPath', $script:FakeOpenMatPath,
            '-ConformanceRoot', $script:ConformanceRoot,
            '-ResultDirectory', $resultDirectory
        )) {
        $arguments.Add($argument)
    }
    if ($Mode -ceq 'Json') {
        $arguments.Add('-JsonSummary')
    }

    $process = Start-Process `
        -FilePath $script:PowerShellPath `
        -ArgumentList $arguments `
        -WorkingDirectory $script:RepositoryRoot `
        -Wait `
        -PassThru `
        -WindowStyle Hidden `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath
    $stdout = Get-Content -LiteralPath $stdoutPath -Raw
    $stderr = Get-Content -LiteralPath $stderrPath -Raw
    Assert-Equal $process.ExitCode $ExpectedExitCode "$Name exit code"

    $summaryPath = Join-Path $resultDirectory 'run-summary.json'
    if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf)) {
        throw "$Name did not write run-summary.json."
    }
    $summary = Get-Content -LiteralPath $summaryPath -Raw |
        ConvertFrom-Json -Depth 100
    Assert-Equal $summary.passed $Passed "$Name passed count"
    Assert-Equal $summary.failed $Failed "$Name failed count"
    Assert-Equal $summary.unsupported $Unsupported "$Name unsupported count"
    Assert-Equal $summary.internal $Internal "$Name internal count"
    $detailsText = @($summary.cases | ForEach-Object { @($_.details) }) -join "`n"
    foreach ($expectedDetail in $DetailContains) {
        Assert-Contains $detailsText $expectedDetail "$Name case details"
    }

    if ($Mode -ceq 'Json') {
        $stdoutLines = @($stdout -split '\r?\n' | Where-Object Length -gt 0)
        Assert-Equal $stdoutLines.Count 1 "$Name JSON stdout line count"
        $stdoutSummary = $stdoutLines[0] | ConvertFrom-Json -Depth 100
        Assert-Equal $stdoutSummary.failed $Failed "$Name JSON failed count"
        if ($stdout.Contains('[', [StringComparison]::Ordinal)) {
            # JSON itself contains brackets, so check the status prefixes specifically.
            foreach ($prefix in @('[PASSED]', '[FAILED]', '[UNSUPPORTED]', '[INTERNAL]')) {
                if ($stdout.Contains($prefix, [StringComparison]::Ordinal)) {
                    throw "$Name JSON stdout contains status output '$prefix'."
                }
            }
        }
        Assert-Contains $stderr '[' "$Name JSON stderr status"
    } else {
        Assert-Contains $stdout 'Completed ' "$Name normal summary"
        if (-not [string]::IsNullOrWhiteSpace($stderr)) {
            throw "$Name normal mode unexpectedly wrote stderr: $stderr"
        }
    }
}

$script:RepositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$script:RunnerPath = Join-Path $script:RepositoryRoot `
    'tools/openmat-conformance/Invoke-OpenMatConformance.ps1'
$script:PowerShellPath = (Get-Command pwsh -ErrorAction Stop).Source
$temporaryRoot = [System.IO.Path]::GetFullPath(
    [System.IO.Path]::GetTempPath()
).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
$script:TestRoot = Join-Path $temporaryRoot `
    ('openmat-conformance-exit-tests-' + [guid]::NewGuid().ToString('N'))
$script:FakeOpenMatPath = Join-Path $PSScriptRoot 'FakeOpenMat.cmd'

try {
    [void] (New-Item -ItemType Directory -Path $script:TestRoot)
    Initialize-RunnerFixtures
    Invoke-RunnerFixture `
        -Name 'pass-json' -Cases 'scalar_shape' -ExpectedExitCode 0 `
        -Mode Json -Passed 1
    Invoke-RunnerFixture `
        -Name 'failed-normal' -Cases 'matrix_shape' -ExpectedExitCode 2 `
        -Mode Normal -Failed 1
    Invoke-RunnerFixture `
        -Name 'unsupported-json' -Cases 'class_constructor' -ExpectedExitCode 3 `
        -Mode Json -Unsupported 1
    Invoke-RunnerFixture `
        -Name 'internal-normal' -Cases 'floating_tolerance' -ExpectedExitCode 4 `
        -Mode Normal -Internal 1
    Invoke-RunnerFixture `
        -Name 'failed-over-unsupported-json' `
        -Cases 'matrix_shape,class_constructor' -ExpectedExitCode 2 `
        -Mode Json -Failed 1 -Unsupported 1
    Invoke-RunnerFixture `
        -Name 'internal-over-mixed-normal' `
        -Cases 'scalar_shape,matrix_shape,class_constructor,floating_tolerance' `
        -ExpectedExitCode 4 -Mode Normal `
        -Passed 1 -Failed 1 -Unsupported 1 -Internal 1
    Invoke-RunnerFixture `
        -Name 'v2-pass-json' `
        -Cases 'runner_v2_pass' `
        -ExpectedExitCode 0 -Mode Json -Passed 1
    Invoke-RunnerFixture `
        -Name 'v2-value-failure-normal' `
        -Cases 'runner_v2_mixed_observation' `
        -ExpectedExitCode 2 -Mode Normal -Failed 1 `
        -DetailContains 'does not satisfy schema version 2'
    Invoke-RunnerFixture `
        -Name 'v2-reject-v1-observation-json' `
        -Cases 'runner_v2_schema1_observation' `
        -ExpectedExitCode 3 -Mode Json -Unsupported 1 `
        -DetailContains 'schema-v1 observation; schema-v2 requires kernel-v1'
    Invoke-RunnerFixture `
        -Name 'v2-cli-v1-only-json' `
        -Cases 'runner_v2_cli_v1_only' `
        -ExpectedExitCode 3 -Mode Json -Unsupported 1 `
        -DetailContains 'schema-v2 observation over kernel-v1'
    Invoke-RunnerFixture `
        -Name 'v2-unrelated-unsupported-failed' `
        -Cases 'runner_v2_unrelated_unsupported' `
        -ExpectedExitCode 2 -Mode Normal -Failed 1 `
        -DetailContains 'unsupported operand corrupted the request'
    Invoke-RunnerFixture `
        -Name 'v2-capability-unavailable' `
        -Cases 'runner_v2_capability_unavailable' `
        -ExpectedExitCode 3 -Mode Json -Unsupported 1 `
        -DetailContains 'schema-v2 observation over kernel-v1'
    Invoke-RunnerFixture `
        -Name 'v2-payload-limit' `
        -Cases 'runner_v2_payload_limit' `
        -ExpectedExitCode 3 -Mode Json -Unsupported 1 `
        -DetailContains 'exceeds a conformance payload limit'
    Invoke-RunnerFixture `
        -Name 'v2-cyclic-payload' `
        -Cases 'runner_v2_cyclic_payload' `
        -ExpectedExitCode 3 -Mode Json -Unsupported 1 `
        -DetailContains 'active-path identity cycle'
    Invoke-RunnerFixture `
        -Name 'v2-invalid-observation' `
        -Cases 'runner_v2_invalid_observation' `
        -ExpectedExitCode 4 -Mode Normal -Internal 1 `
        -DetailContains 'invalid canonical observation'
    Invoke-RunnerFixture `
        -Name 'v1-v2-routing-normal' `
        -Cases 'scalar_shape,runner_v2_pass' `
        -ExpectedExitCode 0 -Mode Normal -Passed 2

    $setupStdout = Join-Path $script:TestRoot 'setup.stdout'
    $setupStderr = Join-Path $script:TestRoot 'setup.stderr'
    $setupProcess = Start-Process `
        -FilePath $script:PowerShellPath `
        -ArgumentList @(
            '-NoProfile', '-File', $script:RunnerPath,
            '-Case', 'case_that_does_not_exist',
            '-OpenMatPath', $script:FakeOpenMatPath,
            '-ConformanceRoot', $script:ConformanceRoot,
            '-JsonSummary'
        ) `
        -WorkingDirectory $script:RepositoryRoot `
        -Wait `
        -PassThru `
        -WindowStyle Hidden `
        -RedirectStandardOutput $setupStdout `
        -RedirectStandardError $setupStderr
    Assert-Equal $setupProcess.ExitCode 64 'setup exit code'
    Assert-Equal (Get-Content -LiteralPath $setupStdout -Raw) '' 'setup stdout'
    Assert-Contains `
        (Get-Content -LiteralPath $setupStderr -Raw) `
        'No conformance cases matched' `
        'setup stderr'

    Write-Output 'Runner exit-code tests passed.'
} finally {
    if (Test-Path -LiteralPath $script:TestRoot -PathType Container) {
        $resolved = (Resolve-Path -LiteralPath $script:TestRoot).Path
        $parent = [System.IO.Path]::GetDirectoryName($resolved)
        $leaf = [System.IO.Path]::GetFileName($resolved)
        if ($parent -cne $temporaryRoot -or
                $leaf -cnotmatch '^openmat-conformance-exit-tests-[0-9a-f]{32}$') {
            throw 'Refusing to clean an unexpected runner test path.'
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
