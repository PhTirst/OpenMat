#Requires -Version 7.0

[CmdletBinding()]
param(
    [string] $MatlabPath = $env:MATLAB_EXE,
    [string[]] $Tag = @(),
    [ValidateRange(1, 86400)]
    [int] $TimeoutSeconds = 180,
    [string] $ResultDirectory,
    [switch] $List,
    [switch] $KeepTemporary
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (
    Join-Path $PSScriptRoot '../openmat-conformance/OpenMat.Conformance.psm1'
) -Force

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

function Test-Property {
    param(
        [Parameter(Mandatory)] [object] $InputObject,
        [Parameter(Mandatory)] [string] $Name
    )

    return $null -ne $InputObject.PSObject.Properties[$Name]
}

function Get-OracleNormalizationCategory {
    param([Parameter(Mandatory)] [object] $Observation)

    if (-not (Test-Property $Observation 'outcome') -or
            [string] $Observation.outcome -cne 'error' -or
            -not (Test-Property $Observation 'error') -or
            $null -eq $Observation.error -or
            -not (Test-Property $Observation.error 'category')) {
        return $null
    }
    $category = [string] $Observation.error.category
    if ($category -cin @(
            'unsupported-payload',
            'payload-limit',
            'cyclic-payload',
            'invalid-observation'
        )) {
        return $category
    }
    return $null
}

function Assert-OracleNormalizationObservation {
    param(
        [Parameter(Mandatory)] [object] $Observation,
        [Parameter(Mandatory)] [string] $CaseId,
        [Parameter(Mandatory)] [ValidateSet(1, 2)] [int] $SchemaVersion,
        [Parameter(Mandatory)] [string] $Label
    )

    foreach ($required in @('schema_version', 'case_id', 'oracle', 'outcome', 'error')) {
        if (-not (Test-Property $Observation $required)) {
            throw "$Label is missing $required."
        }
    }
    $properties = @($Observation.PSObject.Properties.Name)
    if ($properties.Count -ne 5 -or
            @($properties | Where-Object {
                    $_ -cnotin @('schema_version', 'case_id', 'oracle', 'outcome', 'error')
                }).Count -ne 0) {
        throw "$Label has a non-canonical normalization envelope."
    }
    if ([int] $Observation.schema_version -ne $SchemaVersion -or
            [string] $Observation.case_id -cne $CaseId -or
            [string] $Observation.outcome -cne 'error') {
        throw "$Label has inconsistent normalization metadata."
    }
    if ($null -eq $Observation.oracle -or
            -not (Test-Property $Observation.oracle 'name') -or
            -not (Test-Property $Observation.oracle 'release') -or
            [string] $Observation.oracle.name -cne 'MATLAB') {
        throw "$Label has an invalid oracle identity."
    }
    $errorProperties = @($Observation.error.PSObject.Properties.Name)
    if ($errorProperties.Count -ne 1 -or $errorProperties[0] -cne 'category') {
        throw "$Label has a non-canonical normalization error."
    }
}

function Assert-JsonSchema {
    param(
        [Parameter(Mandatory)] [string] $Json,
        [Parameter(Mandatory)] [string] $SchemaPath,
        [Parameter(Mandatory)] [string] $Label
    )

    try {
        if (-not ($Json | Test-Json -SchemaFile $SchemaPath -ErrorAction Stop)) {
            throw 'schema validation returned false'
        }
    } catch {
        throw "$Label does not satisfy $([System.IO.Path]::GetFileName($SchemaPath))."
    }
}

function ConvertTo-CompactJson {
    param([AllowNull()] [object] $InputObject)

    return ConvertTo-Json -InputObject @($InputObject) -Depth 100 -Compress
}

function Compare-ExactSequence {
    param(
        [AllowNull()] [object] $Actual,
        [AllowNull()] [object] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if ((ConvertTo-CompactJson $Actual) -ne (ConvertTo-CompactJson $Expected)) {
        return "$Label differs"
    }
    return $null
}

function ConvertTo-FiniteDouble {
    param([Parameter(Mandatory)] [string] $Text)

    return [double]::Parse(
        $Text,
        [System.Globalization.NumberStyles]::Float,
        [System.Globalization.CultureInfo]::InvariantCulture
    )
}

function Compare-NumericSequence {
    param(
        [AllowNull()] [object] $Actual,
        [AllowNull()] [object] $Expected,
        [AllowNull()] [object] $Tolerance,
        [Parameter(Mandatory)] [string] $Label
    )

    $actualItems = @($Actual)
    $expectedItems = @($Expected)
    if ($actualItems.Count -ne $expectedItems.Count) {
        return "$Label count differs"
    }

    if ($null -eq $Tolerance) {
        return Compare-ExactSequence $actualItems $expectedItems $Label
    }

    $absolute = if (Test-Property $Tolerance 'absolute') {
        [double] $Tolerance.absolute
    } else {
        0.0
    }
    $relative = if (Test-Property $Tolerance 'relative') {
        [double] $Tolerance.relative
    } else {
        0.0
    }
    $special = @('NaN', '+Inf', '-Inf')

    for ($index = 0; $index -lt $actualItems.Count; $index++) {
        $actualText = [string] $actualItems[$index]
        $expectedText = [string] $expectedItems[$index]
        if ($special -contains $actualText -or $special -contains $expectedText) {
            if ($actualText -cne $expectedText) {
                return "$Label element $index differs"
            }
            continue
        }

        $actualNumber = ConvertTo-FiniteDouble $actualText
        $expectedNumber = ConvertTo-FiniteDouble $expectedText
        $limit = $absolute + $relative * [Math]::Max(
            [Math]::Abs($actualNumber),
            [Math]::Abs($expectedNumber)
        )
        if ([Math]::Abs($actualNumber - $expectedNumber) -gt $limit) {
            return "$Label element $index exceeds tolerance"
        }
    }
    return $null
}

function Compare-Value {
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($name in @('class', 'ndims', 'numel', 'kind')) {
        if ([string] $Actual.$name -cne [string] $Expected.$name) {
            $problems.Add("value.$name differs")
        }
    }
    $sizeProblem = Compare-ExactSequence $Actual.size $Expected.size 'value.size'
    if ($null -ne $sizeProblem) {
        $problems.Add($sizeProblem)
    }

    $tolerance = if (Test-Property $Expected 'tolerance') {
        $Expected.tolerance
    } else {
        $null
    }

    switch ([string] $Expected.kind) {
        'numeric' {
            foreach ($component in @('real', 'imag')) {
                $problem = Compare-NumericSequence $Actual.$component `
                    $Expected.$component $tolerance "value.$component"
                if ($null -ne $problem) {
                    $problems.Add($problem)
                }
            }
        }
        'integer' {
            $problem = Compare-ExactSequence $Actual.integer $Expected.integer 'value.integer'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'logical' {
            $problem = Compare-ExactSequence $Actual.logical $Expected.logical 'value.logical'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'char' {
            $problem = Compare-ExactSequence $Actual.code_units $Expected.code_units 'value.code_units'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'string' {
            foreach ($payload in @('string', 'missing')) {
                $problem = Compare-ExactSequence $Actual.$payload `
                    $Expected.$payload "value.$payload"
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
        'cell' {
            $problem = Compare-ExactSequence $Actual.items $Expected.items 'value.items'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
    }

    return $problems.ToArray()
}

function Compare-Observation {
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected,
        [Parameter(Mandatory)] [string] $CaseId,
        [Parameter(Mandatory)] [ValidateSet(1, 2)] [int] $SchemaVersion
    )

    if ($SchemaVersion -eq 2) {
        return @(Compare-OpenMatObservation `
            $Actual $Expected $CaseId -SchemaVersion 2)
    }

    $problems = [System.Collections.Generic.List[string]]::new()
    if ([int] $Actual.schema_version -ne 1) {
        $problems.Add('schema_version is not 1')
    }
    if ([string] $Actual.case_id -cne $CaseId) {
        $problems.Add('case_id differs')
    }
    if ([string] $Actual.outcome -cne [string] $Expected.outcome) {
        $problems.Add('outcome differs')
        return $problems.ToArray()
    }

    if ([string] $Expected.outcome -ceq 'ok') {
        foreach ($problem in @(Compare-Value $Actual.value $Expected.value)) {
            if ($null -ne $problem) { $problems.Add($problem) }
        }
    } elseif ([string] $Actual.error.category -cne [string] $Expected.error.category) {
        $problems.Add('error.category differs')
    }
    return $problems.ToArray()
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

function ConvertTo-MatlabStringLiteral {
    param([Parameter(Mandatory)] [string] $Value)

    return "'" + $Value.Replace("'", "''") + "'"
}

function Get-ConformanceCases {
    param(
        [Parameter(Mandatory)] [string] $ManifestDirectory,
        [Parameter(Mandatory)] [string] $CaseDirectory,
        [Parameter(Mandatory)] [string] $SchemaDirectory,
        [string[]] $Tags
    )

    $items = [System.Collections.Generic.List[object]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $ManifestDirectory -Filter '*.json' -File) {
        $manifestJson = Get-Content -LiteralPath $file.FullName -Raw
        $manifest = $manifestJson | ConvertFrom-Json -Depth 100
        foreach ($required in @('schema_version', 'id', 'description', 'source', 'tags', 'expected')) {
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
        Assert-JsonSchema $manifestJson $caseSchema "Manifest $($file.Name)"
        foreach ($problem in @(Test-OpenMatExpected `
                    $manifest.expected -SchemaVersion $schemaVersion)) {
            if ($null -ne $problem) {
                throw "Manifest $($file.Name) is semantically invalid: $problem."
            }
        }
        if ([string] $manifest.id -cnotmatch '^[a-z][a-z0-9_]*$') {
            throw "Manifest $($file.Name) has an invalid id."
        }

        $sourcePath = [System.IO.Path]::GetFullPath(
            (Join-Path $CaseDirectory ([string] $manifest.source))
        )
        if (-not (Test-PathWithin $CaseDirectory $sourcePath) -or
                -not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            throw "Manifest $($file.Name) references an invalid source path."
        }

        $selected = $Tags.Count -eq 0
        foreach ($wantedTag in $Tags) {
            if (@($manifest.tags) -icontains $wantedTag) {
                $selected = $true
            }
        }
        if ($selected) {
            $items.Add([pscustomobject]@{
                Manifest = $manifest
                SchemaVersion = $schemaVersion
                SourcePath = $sourcePath
            })
        }
    }

    $duplicates = @($items | Group-Object { $_.Manifest.id } | Where-Object Count -gt 1)
    if ($duplicates.Count -gt 0) {
        throw 'Selected manifests contain duplicate case ids.'
    }
    return @($items | Sort-Object { $_.Manifest.id })
}

function Invoke-Oracle {
    $scriptDirectory = [System.IO.Path]::GetFullPath($PSScriptRoot)
    $repositoryRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $scriptDirectory '..\..')
    )
    $caseDirectory = [System.IO.Path]::GetFullPath(
        (Join-Path $repositoryRoot 'tests\conformance\cases')
    )
    $manifestDirectory = Join-Path $caseDirectory 'manifests'
    $schemaDirectory = Join-Path $repositoryRoot 'tests\conformance\schema'
    $wrapperPath = Join-Path $scriptDirectory 'openmat_oracle_run.m'
    $temporaryDirectory = $null

    try {
        $normalizedTags = @(
            foreach ($tagArgument in $Tag) {
                foreach ($part in $tagArgument.Split(',')) {
                    $trimmed = $part.Trim()
                    if ($trimmed.Length -gt 0) { $trimmed }
                }
            }
        )
        $cases = @(Get-ConformanceCases `
            $manifestDirectory $caseDirectory $schemaDirectory $normalizedTags)
        if ($cases.Count -eq 0) {
            throw 'No conformance cases matched the requested tags.'
        }

        if ($List) {
            foreach ($case in $cases) {
                Write-Host ("{0}`t{1}" -f
                    $case.Manifest.id, (@($case.Manifest.tags) -join ','))
            }
            return 0
        }

        if ([string]::IsNullOrWhiteSpace($MatlabPath)) {
            throw 'Specify -MatlabPath or set MATLAB_EXE to your licensed MATLAB R2022b executable.'
        }
        $resolvedMatlab = (Resolve-Path -LiteralPath $MatlabPath).Path
        if (-not (Test-Path -LiteralPath $resolvedMatlab -PathType Leaf)) {
            throw 'The MATLAB executable path is not a file.'
        }
        if (-not (Test-Path -LiteralPath $wrapperPath -PathType Leaf)) {
            throw 'The MATLAB wrapper is missing.'
        }

        if ([string]::IsNullOrWhiteSpace($ResultDirectory)) {
            $resultPath = Join-Path $scriptDirectory 'results'
        } elseif ([System.IO.Path]::IsPathRooted($ResultDirectory)) {
            $resultPath = [System.IO.Path]::GetFullPath($ResultDirectory)
        } else {
            $resultPath = [System.IO.Path]::GetFullPath(
                (Join-Path (Get-Location).Path $ResultDirectory)
            )
        }
        if (Test-PathWithin $caseDirectory $resultPath) {
            throw 'The result directory must not be inside the source case tree.'
        }

        $temporaryParent = [System.IO.Path]::GetFullPath(
            [System.IO.Path]::GetTempPath()
        ).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
        $temporaryDirectory = Join-Path $temporaryParent `
            ('openmat-oracle-' + [guid]::NewGuid().ToString('N'))
        [void] (New-Item -ItemType Directory -Path $temporaryDirectory)
        Copy-Item -LiteralPath $caseDirectory -Destination $temporaryDirectory -Recurse
        Copy-Item -LiteralPath $wrapperPath -Destination $temporaryDirectory

        $stagedCaseDirectory = Join-Path $temporaryDirectory 'cases'
        $stagedOutput = Join-Path $temporaryDirectory 'observations'
        [void] (New-Item -ItemType Directory -Path $stagedOutput)
        $configCases = foreach ($case in $cases) {
            $relativeSource = [System.IO.Path]::GetRelativePath(
                $caseDirectory,
                $case.SourcePath
            )
            [ordered]@{
                id = [string] $case.Manifest.id
                schema_version = $case.SchemaVersion
                source_path = Join-Path $stagedCaseDirectory $relativeSource
            }
        }
        $config = [ordered]@{
            output_directory = $stagedOutput
            program_directory = Join-Path $stagedCaseDirectory 'programs'
            support_directory = Join-Path $stagedCaseDirectory 'support'
            cases = @($configCases)
        }
        $configPath = Join-Path $temporaryDirectory 'oracle-config.json'
        $utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
        [System.IO.File]::WriteAllText(
            $configPath,
            (ConvertTo-Json $config -Depth 20),
            $utf8WithoutBom
        )

        $wrapperLiteral = ConvertTo-MatlabStringLiteral $temporaryDirectory
        $configLiteral = ConvertTo-MatlabStringLiteral $configPath
        $batchExpression = "try, addpath($wrapperLiteral); " +
            "openmat_oracle_run($configLiteral); catch, exit(91); end; exit(0);"

        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $resolvedMatlab
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.WorkingDirectory = $temporaryDirectory
        $startInfo.ArgumentList.Add('-batch')
        $startInfo.ArgumentList.Add($batchExpression)

        $process = [System.Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        if (-not $process.Start()) {
            throw 'MATLAB process did not start.'
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill($true)
            $process.WaitForExit()
            [void] $stdoutTask.GetAwaiter().GetResult()
            [void] $stderrTask.GetAwaiter().GetResult()
            Write-Error "MATLAB oracle timed out after $TimeoutSeconds seconds." -ErrorAction Continue
            return 124
        }
        [void] $stdoutTask.GetAwaiter().GetResult()
        [void] $stderrTask.GetAwaiter().GetResult()
        $stopwatch.Stop()
        if ($process.ExitCode -ne 0) {
            Write-Error "MATLAB wrapper exited with code $($process.ExitCode)." -ErrorAction Continue
            return 4
        }

        [void] (New-Item -ItemType Directory -Path $resultPath -Force)
        $caseResults = [System.Collections.Generic.List[object]]::new()
        $failed = 0
        $matlabRelease = $null
        foreach ($case in $cases) {
            $caseId = [string] $case.Manifest.id
            $observationPath = Join-Path $stagedOutput "$caseId.json"
            if (-not (Test-Path -LiteralPath $observationPath -PathType Leaf)) {
                Write-Error "MATLAB produced no normalized output for $caseId." -ErrorAction Continue
                return 4
            }
            $actualJson = Get-Content -LiteralPath $observationPath -Raw
            $observationSchema = Join-Path $schemaDirectory $(
                if ($case.SchemaVersion -eq 1) {
                    'observation.schema.json'
                } else {
                    'observation-v2.schema.json'
                }
            )
            $actual = $actualJson | ConvertFrom-Json -Depth 100
            $normalizationCategory = Get-OracleNormalizationCategory $actual
            if ($null -eq $normalizationCategory) {
                Assert-JsonSchema $actualJson $observationSchema `
                    "MATLAB observation $caseId"
            } else {
                Assert-OracleNormalizationObservation `
                    $actual $caseId $case.SchemaVersion "MATLAB observation $caseId"
            }
            if ($null -eq $matlabRelease) {
                $matlabRelease = [string] $actual.oracle.release
            }
            $problems = @(
                if ($null -eq $normalizationCategory) {
                    Compare-Observation `
                        $actual $case.Manifest.expected $caseId $case.SchemaVersion
                } else {
                    "oracle normalization failed: $normalizationCategory"
                }
            )
            $status = if ($problems.Count -eq 0) { 'passed' } else { 'failed' }
            if ($status -eq 'failed') { $failed++ }
            $caseResults.Add([ordered]@{
                id = $caseId
                status = $status
                details = @($problems)
            })
            Copy-Item -LiteralPath $observationPath `
                -Destination (Join-Path $resultPath "$caseId.json") -Force
            Write-Host ('[{0}] {1}' -f $status.ToUpperInvariant(), $caseId)
        }

        $summary = [ordered]@{
            schema_version = 1
            oracle = [ordered]@{ name = 'MATLAB'; release = $matlabRelease }
            total = $cases.Count
            passed = $cases.Count - $failed
            failed = $failed
            elapsed_ms = [Math]::Round($stopwatch.Elapsed.TotalMilliseconds)
            cases = $caseResults.ToArray()
        }
        [System.IO.File]::WriteAllText(
            (Join-Path $resultPath 'run-summary.json'),
            (ConvertTo-Json $summary -Depth 20),
            $utf8WithoutBom
        )
        Write-Host "Completed $($cases.Count) case(s) in $($summary.elapsed_ms) ms; $failed failed."
        if ($failed -gt 0) { return 2 }
        return 0
    } catch {
        $location = if ([string]::IsNullOrWhiteSpace($_.ScriptStackTrace)) {
            ''
        } else {
            "`n$($_.ScriptStackTrace)"
        }
        Write-Error "$($_.Exception.Message)$location" -ErrorAction Continue
        return 3
    } finally {
        if ($null -ne $temporaryDirectory -and
                (Test-Path -LiteralPath $temporaryDirectory -PathType Container)) {
            if ($KeepTemporary) {
                Write-Host "Temporary oracle directory preserved: $temporaryDirectory"
            } else {
                $resolvedTemporary = (Resolve-Path -LiteralPath $temporaryDirectory).Path
                $parent = [System.IO.Path]::GetDirectoryName($resolvedTemporary)
                $leaf = [System.IO.Path]::GetFileName($resolvedTemporary)
                if ($parent -cne $temporaryParent -or
                        $leaf -cnotmatch '^openmat-oracle-[0-9a-f]{32}$') {
                    throw 'Refusing to clean an unexpected temporary path.'
                }
                Remove-Item -LiteralPath $resolvedTemporary -Recurse -Force
            }
        }
    }
}

exit (Invoke-Oracle)
