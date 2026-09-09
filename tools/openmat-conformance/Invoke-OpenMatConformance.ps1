#Requires -Version 7.0

[CmdletBinding()]
param(
    [string[]] $Case = @(),
    [string] $OpenMatPath,
    [string] $ConformanceRoot,
    [string] $ResultDirectory,
    [ValidateRange(1, 3600)]
    [int] $TimeoutSeconds = 30,
    [switch] $List,
    [switch] $JsonSummary
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OpenMat.Conformance.psm1') -Force

function Test-Property {
    param(
        [Parameter(Mandatory)] [object] $InputObject,
        [Parameter(Mandatory)] [string] $Name
    )

    return $null -ne $InputObject.PSObject.Properties[$Name]
}

function Test-PathWithin {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $Candidate
    )

    $relative = [System.IO.Path]::GetRelativePath($Root, $Candidate)
    return -not [System.IO.Path]::IsPathRooted($relative) -and
        $relative -ne '..' -and
        -not $relative.StartsWith("..$([System.IO.Path]::DirectorySeparatorChar)")
}

function Test-JsonSchema {
    param(
        [Parameter(Mandatory)] [string] $Json,
        [Parameter(Mandatory)] [string] $SchemaPath
    )

    try {
        return $Json | Test-Json -SchemaFile $SchemaPath -ErrorAction Stop
    } catch {
        return $false
    }
}

function Write-Utf8Json {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [object] $Value
    )

    $encoding = [System.Text.UTF8Encoding]::new($false)
    $json = ConvertTo-Json -InputObject $Value -Depth 100
    [System.IO.File]::WriteAllText($Path, "$json`n", $encoding)
}

function New-InternalObservation {
    param(
        [Parameter(Mandatory)] [string] $CaseId,
        [Parameter(Mandatory)] [ValidateSet(1, 2)] [int] $SchemaVersion,
        [string] $Category = 'internal'
    )

    return [ordered]@{
        schema_version = $SchemaVersion
        case_id = $CaseId
        oracle = [ordered]@{
            name = 'OpenMat'
            release = 'runner-internal'
        }
        outcome = 'error'
        error = [ordered]@{ category = $Category }
    }
}

function Get-ManifestSchemaVersion {
    param(
        [Parameter(Mandatory)] [object] $Manifest,
        [Parameter(Mandatory)] [string] $FileName
    )

    $value = $Manifest.schema_version
    try {
        $version = [int] $value
        if ([decimal] $value -ne [decimal] $version) {
            throw 'not an integer'
        }
    } catch {
        throw "Manifest $FileName has an invalid schema_version."
    }
    if ($version -notin @(1, 2)) {
        throw "Manifest $FileName uses unsupported schema_version $version."
    }
    return $version
}

function Get-SelectedCases {
    param(
        [Parameter(Mandatory)] [string] $ManifestDirectory,
        [Parameter(Mandatory)] [string] $SchemaDirectory,
        [string[]] $Filters
    )

    $normalized = @(
        foreach ($filter in $Filters) {
            foreach ($part in $filter.Split(',')) {
                $trimmed = $part.Trim()
                if ($trimmed.Length -gt 0) { $trimmed }
            }
        }
    )
    $items = [System.Collections.Generic.List[object]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $ManifestDirectory -Filter '*.json' -File) {
        $manifestJson = Get-Content -LiteralPath $file.FullName -Raw
        $manifest = $manifestJson | ConvertFrom-Json -Depth 100
        foreach ($required in @('schema_version', 'id', 'source', 'tags', 'expected')) {
            if (-not (Test-Property $manifest $required)) {
                throw "Manifest $($file.Name) is missing $required."
            }
        }
        $schemaVersion = Get-ManifestSchemaVersion $manifest $file.Name
        $caseSchema = Join-Path $SchemaDirectory $(if ($schemaVersion -eq 1) {
                'case.schema.json'
            } else {
                'case-v2.schema.json'
            })
        if (-not (Test-JsonSchema $manifestJson $caseSchema)) {
            throw "Manifest $($file.Name) does not satisfy its schema version."
        }
        foreach ($problem in @(Test-OpenMatExpected `
                    $manifest.expected -SchemaVersion $schemaVersion)) {
            if ($null -ne $problem) {
                throw "Manifest $($file.Name) is semantically invalid: $problem."
            }
        }
        $id = [string] $manifest.id
        if ($id -cnotmatch '^[a-z][a-z0-9_]*$') {
            throw "Manifest $($file.Name) has an invalid id."
        }
        $selected = $normalized.Count -eq 0
        foreach ($filter in $normalized) {
            if ($id -like $filter) { $selected = $true }
        }
        if ($selected) {
            $items.Add([pscustomobject]@{
                Manifest = $manifest
                ManifestPath = $file.FullName
                SchemaVersion = $schemaVersion
            })
        }
    }
    $duplicates = @($items | Group-Object { $_.Manifest.id } | Where-Object Count -gt 1)
    if ($duplicates.Count -gt 0) {
        throw 'Selected manifests contain duplicate ids.'
    }
    return @($items | Sort-Object { $_.Manifest.id })
}

function Resolve-OpenMatExecutable {
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [string] $RequestedPath
    )

    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        $resolved = (Resolve-Path -LiteralPath $RequestedPath).Path
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            throw 'OpenMatPath must name an executable file.'
        }
        return $resolved
    }

    $buildOutput = & cargo build --quiet -p openmat-cli 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build -p openmat-cli failed: $($buildOutput -join [Environment]::NewLine)"
    }
    $metadataText = & cargo metadata --format-version=1 --no-deps 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed: $($metadataText -join [Environment]::NewLine)"
    }
    $metadata = ($metadataText -join [Environment]::NewLine) | ConvertFrom-Json -Depth 100
    $suffix = if ($IsWindows) { '.exe' } else { '' }
    $candidate = Join-Path ([string] $metadata.target_directory) "debug/openmat-cli$suffix"
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "Built OpenMat CLI was not found at $candidate."
    }
    return [System.IO.Path]::GetFullPath($candidate)
}

function Invoke-OpenMatCase {
    param(
        [Parameter(Mandatory)] [string] $Executable,
        [Parameter(Mandatory)] [string] $ManifestPath,
        [Parameter(Mandatory)] [string] $WorkingDirectory,
        [Parameter(Mandatory)] [int] $Timeout
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.ArgumentList.Add('conformance')
    $startInfo.ArgumentList.Add($ManifestPath)
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw 'OpenMat process did not start.'
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($Timeout * 1000)) {
        $process.Kill($true)
        $process.WaitForExit()
        [void] $stdoutTask.GetAwaiter().GetResult()
        [void] $stderrTask.GetAwaiter().GetResult()
        return [pscustomobject]@{
            ExitCode = 124
            Stdout = ''
            Stderr = "timed out after $Timeout seconds"
        }
    }
    return [pscustomobject]@{
        ExitCode = $process.ExitCode
        Stdout = $stdoutTask.GetAwaiter().GetResult()
        Stderr = $stderrTask.GetAwaiter().GetResult()
    }
}

function Write-CaseStatus {
    param(
        [Parameter(Mandatory)] [string] $Status,
        [Parameter(Mandatory)] [string] $CaseId,
        [switch] $AsJson
    )

    $message = '[{0}] {1}' -f $Status.ToUpperInvariant(), $CaseId
    if ($AsJson) {
        [Console]::Error.WriteLine($message)
    } else {
        Write-Host $message
    }
}

function Invoke-ConformanceRun {
    $repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
    if ([string]::IsNullOrWhiteSpace($ConformanceRoot)) {
        $conformanceRoot = Join-Path $repositoryRoot 'tests/conformance'
    } else {
        $conformanceRoot = (Resolve-Path -LiteralPath $ConformanceRoot).Path
        if (-not (Test-Path -LiteralPath $conformanceRoot -PathType Container)) {
            throw 'ConformanceRoot must name a directory.'
        }
    }
    $manifestDirectory = Join-Path $conformanceRoot 'cases/manifests'
    $referenceDirectory = Join-Path $conformanceRoot 'reference/matlab-r2022b'
    $schemaDirectory = Join-Path $conformanceRoot 'schema'
    $selected = @(Get-SelectedCases $manifestDirectory $schemaDirectory $Case)
    if ($selected.Count -eq 0) {
        throw 'No conformance cases matched -Case.'
    }
    if ($List) {
        foreach ($item in $selected) {
            [Console]::Out.WriteLine(
                ("{0}`t{1}" -f
                    $item.Manifest.id, (@($item.Manifest.tags) -join ','))
            )
        }
        return 0
    }

    if ([string]::IsNullOrWhiteSpace($ResultDirectory)) {
        $resultPath = Join-Path ([System.IO.Path]::GetTempPath()) `
            ('openmat-conformance-' + [guid]::NewGuid().ToString('N'))
    } elseif ([System.IO.Path]::IsPathRooted($ResultDirectory)) {
        $resultPath = [System.IO.Path]::GetFullPath($ResultDirectory)
    } else {
        $resultPath = [System.IO.Path]::GetFullPath(
            (Join-Path (Get-Location).Path $ResultDirectory)
        )
    }
    if (Test-PathWithin $referenceDirectory $resultPath) {
        throw 'ResultDirectory must not be inside the checked-in reference directory.'
    }
    if (Test-PathWithin (Join-Path $conformanceRoot 'cases') $resultPath) {
        throw 'ResultDirectory must not be inside the conformance case tree.'
    }
    [void] (New-Item -ItemType Directory -Path $resultPath -Force)

    Push-Location $repositoryRoot
    try {
        $executable = Resolve-OpenMatExecutable $repositoryRoot $OpenMatPath
    } finally {
        Pop-Location
    }

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $caseResults = [System.Collections.Generic.List[object]]::new()
    $counts = [ordered]@{ passed = 0; failed = 0; unsupported = 0; internal = 0 }
    foreach ($item in $selected) {
        $id = [string] $item.Manifest.id
        $schemaVersion = [int] $item.SchemaVersion
        $status = $null
        $details = [System.Collections.Generic.List[string]]::new()
        $actual = $null
        try {
            $processResult = Invoke-OpenMatCase `
                $executable $item.ManifestPath $repositoryRoot $TimeoutSeconds
            if ($processResult.ExitCode -ne 0) {
                $details.Add("CLI exited with code $($processResult.ExitCode)")
                if (-not [string]::IsNullOrWhiteSpace($processResult.Stderr)) {
                    $details.Add($processResult.Stderr.Trim())
                }
                if ($schemaVersion -eq 2) {
                    $knownV1Only = (
                        $processResult.Stderr -match
                            '(?m)^error\[cli\.manifest\][^\r\n]*:\s*schema_version must be 1\.?\r?$' -or
                        $processResult.Stderr -match
                            'protocol\.noCommonVersion|(?m)^error\[(?:cli\.capability|cli\.protocol)\][^\r\n]*openmat-kernel-v1'
                    )
                    if ($knownV1Only) {
                        $status = 'unsupported'
                        $details.Add(
                            'OpenMat CLI did not provide a schema-v2 observation over kernel-v1.'
                        )
                        $actual = New-InternalObservation `
                            $id 2 'unsupported-kernel-v1'
                    } else {
                        $status = 'failed'
                        $actual = New-InternalObservation `
                            $id 2 'conformance-cli-failure'
                    }
                } else {
                    $status = 'internal'
                    $actual = New-InternalObservation $id 1
                }
            } else {
                try {
                    $actual = $processResult.Stdout | ConvertFrom-Json -Depth 100
                } catch {
                    $details.Add('CLI stdout was not one observation JSON object')
                    $status = if ($schemaVersion -eq 2) { 'failed' } else { 'internal' }
                    $actual = New-InternalObservation `
                        $id $schemaVersion 'invalid-observation'
                }
            }

            if ($null -eq $status) {
                $actualVersion = if (Test-Property $actual 'schema_version') {
                    [string] $actual.schema_version
                } else {
                    ''
                }
                if ($schemaVersion -eq 2 -and $actualVersion -cne '2') {
                    if ($actualVersion -ceq '1') {
                        $status = 'unsupported'
                        $details.Add(
                            'OpenMat emitted a schema-v1 observation; schema-v2 requires kernel-v1.'
                        )
                    } else {
                        $status = 'failed'
                        $details.Add(
                            'OpenMat did not emit the required schema-v2 observation.'
                        )
                    }
                } else {
                    $observationSchema = Join-Path $schemaDirectory $(
                        if ($schemaVersion -eq 1) {
                            'observation.schema.json'
                        } else {
                            'observation-v2.schema.json'
                        }
                    )
                    if ($schemaVersion -eq 2 -and
                            [System.Text.Encoding]::UTF8.GetByteCount(
                                $processResult.Stdout
                            ) -gt 1048576) {
                        $status = 'failed'
                        $details.Add(
                            'OpenMat observation exceeds the 1048576-byte UTF-8 frame limit.'
                        )
                    } elseif (-not (Test-JsonSchema `
                                $processResult.Stdout $observationSchema)) {
                        $status = 'failed'
                        $details.Add(
                            "OpenMat observation does not satisfy schema version $schemaVersion."
                        )
                    }
                }
            }

            if ($null -eq $status) {
                $syntheticCategory = if (
                    [string] $actual.outcome -ceq 'error' -and
                        (Test-Property $actual 'error') -and
                        (Test-Property $actual.error 'category')
                ) {
                    [string] $actual.error.category
                } else {
                    ''
                }
                $syntheticStatus = switch ($syntheticCategory) {
                    { $_ -in @(
                            'unsupported-payload',
                            'payload-limit',
                            'cyclic-payload'
                        ) -or $_ -clike 'unsupported-*' } { 'unsupported'; break }
                    'invalid-observation' { 'internal'; break }
                    default { $null }
                }
                if ($null -ne $syntheticStatus) {
                    $status = $syntheticStatus
                    $details.Add($(switch ($syntheticCategory) {
                        'unsupported-file-loading' {
                            'RuntimeEngine cannot load a directly referenced cases/support file.'
                        }
                        'unsupported-payload' {
                            'The kernel cannot transfer this value as one complete bounded payload.'
                        }
                        'payload-limit' {
                            'The complete recursive value exceeds a conformance payload limit.'
                        }
                        'cyclic-payload' {
                            'The complete recursive value contains an active-path identity cycle.'
                        }
                        'invalid-observation' {
                            'OpenMat reported an invalid canonical observation.'
                        }
                        default {
                            'The OpenMat conformance interface does not support this case.'
                        }
                    }))
                } else {
                    foreach ($problem in @(Compare-OpenMatObservation `
                                $actual $item.Manifest.expected $id `
                                -SchemaVersion $schemaVersion)) {
                        if ($null -ne $problem) { $details.Add("manifest: $problem") }
                    }
                    $referencePath = Join-Path $referenceDirectory "$id.json"
                    if (-not (Test-Path -LiteralPath $referencePath -PathType Leaf)) {
                        $details.Add('reference observation is missing')
                        $status = 'internal'
                    } else {
                        $referenceJson = Get-Content -LiteralPath $referencePath -Raw
                        $referenceSchema = Join-Path $schemaDirectory $(
                            if ($schemaVersion -eq 1) {
                                'observation.schema.json'
                            } else {
                                'observation-v2.schema.json'
                            }
                        )
                        if ($schemaVersion -eq 2 -and
                                [System.Text.Encoding]::UTF8.GetByteCount(
                                    $referenceJson
                                ) -gt 1048576) {
                            $details.Add(
                                'reference observation exceeds the 1048576-byte UTF-8 frame limit'
                            )
                            $status = 'internal'
                        } elseif (-not (Test-JsonSchema `
                                    $referenceJson $referenceSchema)) {
                            $details.Add(
                                "reference observation does not satisfy schema version $schemaVersion"
                            )
                            $status = 'internal'
                        }
                        $reference = $referenceJson | ConvertFrom-Json -Depth 100
                        $tolerance = $null
                        if ([string] $item.Manifest.expected.outcome -ceq 'ok' -and
                                (Test-Property $item.Manifest.expected.value 'tolerance')) {
                            $tolerance = $item.Manifest.expected.value.tolerance
                        }
                        foreach ($problem in @(Compare-OpenMatObservation `
                                    $actual $reference $id $tolerance `
                                    -SchemaVersion $schemaVersion)) {
                            if ($null -ne $problem) { $details.Add("reference: $problem") }
                        }
                    }
                    if ($null -eq $status) {
                        $status = if ($details.Count -eq 0) { 'passed' } else { 'failed' }
                    }
                }
            }
        } catch {
            $status = 'internal'
            $details.Add($_.Exception.Message)
            $actual = New-InternalObservation $id $schemaVersion
        }

        Write-Utf8Json (Join-Path $resultPath "$id.json") $actual
        $counts[$status]++
        $caseResults.Add([ordered]@{
            id = $id
            status = $status
            details = $details.ToArray()
        })
        Write-CaseStatus $status $id -AsJson:$JsonSummary
    }
    $stopwatch.Stop()

    $summary = [ordered]@{
        schema_version = 1
        runner = [ordered]@{ name = 'OpenMat'; interface = 'openmat-cli conformance' }
        total = $selected.Count
        passed = $counts.passed
        failed = $counts.failed
        unsupported = $counts.unsupported
        internal = $counts.internal
        elapsed_ms = [Math]::Round($stopwatch.Elapsed.TotalMilliseconds)
        result_directory = $resultPath
        cases = $caseResults.ToArray()
    }
    Write-Utf8Json (Join-Path $resultPath 'run-summary.json') $summary
    if ($JsonSummary) {
        [Console]::Out.WriteLine(
            (ConvertTo-Json -InputObject $summary -Depth 100 -Compress)
        )
    } else {
        Write-Host (
            'Completed {0} case(s): {1} passed, {2} failed, {3} unsupported, {4} internal. Results: {5}' -f
            $summary.total, $summary.passed, $summary.failed,
            $summary.unsupported, $summary.internal, $resultPath
        )
    }

    if ($summary.internal -gt 0) { return 4 }
    if ($summary.failed -gt 0) { return 2 }
    if ($summary.unsupported -gt 0) { return 3 }
    return 0
}

try {
    exit (Invoke-ConformanceRun)
} catch {
    [Console]::Error.WriteLine("openmat-conformance: $($_.Exception.Message)")
    exit 64
}
