#Requires -Version 7.0

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '../OpenMat.Conformance.psm1') -Force

function Read-Fixture {
    param([Parameter(Mandatory)] [string] $Name)

    return Get-Content -LiteralPath (Join-Path $PSScriptRoot "fixtures/$Name.json") -Raw |
        ConvertFrom-Json -Depth 100
}

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

function Assert-ContainsProblem {
    param(
        [Parameter(Mandatory)] [object[]] $Problems,
        [Parameter(Mandatory)] [string] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($Expected -cnotin $Problems) {
        throw "$Label`: expected problem '$Expected', got $($Problems -join '; ')."
    }
}

function Assert-ProblemLike {
    param(
        [Parameter(Mandatory)] [object[]] $Problems,
        [Parameter(Mandatory)] [string] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if (@($Problems | Where-Object { $_ -clike $Expected }).Count -eq 0) {
        throw "$Label`: expected problem like '$Expected', got $($Problems -join '; ')."
    }
}

function Copy-JsonValue {
    param([Parameter(Mandatory)] [object] $Value)

    return $Value | ConvertTo-Json -Depth 100 -Compress |
        ConvertFrom-Json -Depth 100
}

function Test-JsonTextAgainstSchema {
    param(
        [Parameter(Mandatory)] [string] $Json,
        [Parameter(Mandatory)] [string] $SchemaName
    )

    $schemaPath = Join-Path $PSScriptRoot `
        "../../../tests/conformance/schema/$SchemaName"
    try {
        return $Json | Test-Json -SchemaFile $schemaPath -ErrorAction Stop
    } catch {
        return $false
    }
}

function Test-AggregateObservationSchemas {
    param([Parameter(Mandatory)] [object] $Observation)

    $observationJson = ConvertTo-Json $Observation -Depth 100 -Compress
    $case = [ordered]@{
        schema_version = 2
        id = [string] $Observation.case_id
        description = 'Aggregate schema-v2 comparator fixture.'
        source = 'programs/aggregate_fixture.m'
        tags = [object[]] @('aggregate-fixture')
        expected = [ordered]@{
            outcome = [string] $Observation.outcome
            value = $Observation.value
        }
    }
    return [pscustomobject]@{
        Observation = Test-JsonTextAgainstSchema `
            $observationJson 'observation-v2.schema.json'
        Case = Test-JsonTextAgainstSchema `
            (ConvertTo-Json $case -Depth 100 -Compress) 'case-v2.schema.json'
    }
}

function Test-AggregateFixtureSchemas {
    param([Parameter(Mandatory)] [string] $Name)

    return Test-AggregateObservationSchemas (Read-Fixture $Name)
}

function New-V2Observation {
    param(
        [Parameter(Mandatory)] [string] $CaseId,
        [Parameter(Mandatory)] [object] $Value
    )

    return [pscustomobject]@{
        schema_version = 2
        case_id = $CaseId
        oracle = [pscustomobject]@{ name = 'fixture'; release = 'test' }
        outcome = 'ok'
        value = $Value
    }
}

function New-EmptyLogicalValue {
    return [pscustomobject]@{
        class = 'logical'
        size = [object[]] @(0, 0)
        ndims = 2
        numel = 0
        complex = $false
        kind = 'logical'
        logical = [object[]] @()
    }
}

function New-CellValue {
    param([Parameter(Mandatory)] [object[]] $Items)

    return [pscustomobject]@{
        class = 'cell'
        size = [object[]] @(1, $Items.Count)
        ndims = 2
        numel = $Items.Count
        complex = $false
        kind = 'cell'
        items = $Items
    }
}

$actual = Read-Fixture 'numeric-actual'
$within = Read-Fixture 'numeric-within-tolerance'
$exact = Read-Fixture 'numeric-exact'
$outside = Read-Fixture 'numeric-outside-tolerance'

$withinProblems = @(Compare-OpenMatObservation $actual $within 'numeric_fixture')
Assert-Equal $withinProblems.Count 0 'tolerance comparison'

$exactProblems = @(Compare-OpenMatObservation $actual $exact 'numeric_fixture')
Assert-Equal $exactProblems.Count 2 'exact numeric and signed-zero comparison'
Assert-Equal $exactProblems[0] 'value.real differs' 'exact real problem'
Assert-Equal $exactProblems[1] 'value.imag differs' 'exact imaginary problem'

$outsideProblems = @(Compare-OpenMatObservation $actual $outside 'numeric_fixture')
Assert-Equal $outsideProblems.Count 1 'outside-tolerance comparison'
Assert-Equal $outsideProblems[0] 'value.real element 0 exceeds tolerance' `
    'outside-tolerance problem'

$errorProblems = @(Compare-OpenMatObservation `
        (Read-Fixture 'error-actual') (Read-Fixture 'error-expected') 'error_fixture')
Assert-Equal $errorProblems.Count 0 'error category comparison'

$v2String = @'
{
  "schema_version": 2,
  "case_id": "v2_string_fixture",
  "oracle": {"name": "fixture", "release": "test"},
  "outcome": "ok",
  "value": {
    "class": "string", "size": [1, 3], "ndims": 2, "numel": 3,
    "complex": false, "kind": "string",
    "string_code_units": [[55357], [], []],
    "missing": [false, false, true]
  }
}
'@ | ConvertFrom-Json -Depth 100

$v2StringProblems = @(Compare-OpenMatObservation `
        $v2String $v2String 'v2_string_fixture' -SchemaVersion 2)
Assert-Equal $v2StringProblems.Count 0 'v2 exact string comparison'

$v1AsV2 = Copy-JsonValue $v2String
$v1AsV2.schema_version = 1
$v1AsV2Problems = @(Compare-OpenMatObservation `
        $v1AsV2 $v2String 'v2_string_fixture' -SchemaVersion 2)
Assert-Equal $v1AsV2Problems.Count 1 'v1 observation rejected by v2 comparator'
Assert-Equal $v1AsV2Problems[0] 'schema_version is not 2' `
    'v1 observation version problem'

$badShape = Copy-JsonValue $v2String
$badShape.value.ndims = 3
$badShapeProblems = @(Compare-OpenMatObservation `
        $badShape $v2String 'v2_string_fixture' -SchemaVersion 2)
Assert-ContainsProblem $badShapeProblems `
    'value.ndims differs from value.size count' 'v2 ndims invariant'

$badMissing = Copy-JsonValue $v2String
$badMissing.value.string_code_units[2] = [object[]] @(65)
$badMissingProblems = @(Compare-OpenMatObservation `
        $badMissing $v2String 'v2_string_fixture' -SchemaVersion 2)
Assert-ContainsProblem $badMissingProblems `
    'value.missing element 2 has non-empty code units' 'v2 missing invariant'

$badKind = Copy-JsonValue $v2String
$badKind.value | Add-Member -NotePropertyName integer -NotePropertyValue @(
    [pscustomobject]@{ real = '0'; imaginary = '0' }
)
$badKindProblems = @(Compare-OpenMatObservation `
        $badKind $v2String 'v2_string_fixture' -SchemaVersion 2)
Assert-ContainsProblem $badKindProblems `
    'value.integer is not valid for kind string' 'v2 kind-exclusive payload'

$integerRanges = @(
    @('int8', '-128', '127'), @('uint8', '0', '255'),
    @('int16', '-32768', '32767'), @('uint16', '0', '65535'),
    @('int32', '-2147483648', '2147483647'),
    @('uint32', '0', '4294967295'),
    @('int64', '-9223372036854775808', '9223372036854775807'),
    @('uint64', '0', '18446744073709551615')
)
foreach ($range in $integerRanges) {
    $rangeObservation = [pscustomobject]@{
        schema_version = 2
        case_id = 'v2_integer_range'
        oracle = [pscustomobject]@{ name = 'fixture'; release = 'test' }
        outcome = 'ok'
        value = [pscustomobject]@{
            class = $range[0]
            size = [object[]] @(1, 2)
            ndims = 2
            numel = 2
            complex = $false
            kind = 'integer'
            integer = [object[]] @(
                [pscustomobject]@{ real = $range[1]; imaginary = '0' },
                [pscustomobject]@{ real = $range[2]; imaginary = '0' }
            )
        }
    }
    $rangeProblems = @(Compare-OpenMatObservation `
            $rangeObservation $rangeObservation 'v2_integer_range' `
            -SchemaVersion 2)
    Assert-Equal $rangeProblems.Count 0 "v2 $($range[0]) endpoint validation"
}

$v2Unsigned = Copy-JsonValue $rangeObservation
$v2UnsignedProblems = @(Compare-OpenMatObservation `
        $v2Unsigned $v2Unsigned 'v2_integer_range' `
        -SchemaVersion 2)
Assert-Equal $v2UnsignedProblems.Count 0 'v2 uint64 maximum comparison'

$differentUnsigned = Copy-JsonValue $v2Unsigned
$differentUnsigned.value.integer[1].real = '18446744073709551614'
$integerWithToleranceProblems = @(Compare-OpenMatObservation `
        $differentUnsigned $v2Unsigned `
        'v2_integer_range' `
        ([pscustomobject]@{ absolute = 2.0 }) -SchemaVersion 2)
Assert-ContainsProblem $integerWithToleranceProblems `
    'value.integer element 1 real differs' `
    'v2 integer comparison ignores numeric tolerance'

$overflow = Copy-JsonValue $v2Unsigned
$overflow.value.integer[1].real = '18446744073709551616'
$overflowProblems = @(Compare-OpenMatObservation `
        $overflow $v2Unsigned 'v2_integer_range' `
        -SchemaVersion 2)
Assert-ContainsProblem $overflowProblems `
    'value.integer.real is outside the uint64 range' 'v2 uint64 range'

$negativeUnsigned = Copy-JsonValue $v2Unsigned
$negativeUnsigned.value.integer[0].real = '-1'
$negativeUnsignedProblems = @(Compare-OpenMatObservation `
        $negativeUnsigned $v2Unsigned 'v2_integer_range' `
        -SchemaVersion 2)
Assert-ContainsProblem $negativeUnsignedProblems `
    'value.integer.real is not canonical for uint64' 'v2 unsigned signedness'

$v2Complex = [pscustomobject]@{
    schema_version = 2
    case_id = 'v2_complex_integer'
    oracle = [pscustomobject]@{ name = 'fixture'; release = 'test' }
    outcome = 'ok'
    value = [pscustomobject]@{
        class = 'int16'
        size = [object[]] @(1, 2)
        ndims = 2
        numel = 2
        complex = $true
        kind = 'integer'
        integer = [object[]] @(
            [pscustomobject]@{ real = '-32768'; imaginary = '1' },
            [pscustomobject]@{ real = '7'; imaginary = '-9' }
        )
    }
}
$realMarkedComplex = Copy-JsonValue $v2Complex
foreach ($component in $realMarkedComplex.value.integer) {
    $component.imaginary = '0'
}
$realMarkedComplexProblems = @(Compare-OpenMatObservation `
        $realMarkedComplex $v2Complex 'v2_complex_integer' `
        -SchemaVersion 2)
Assert-ContainsProblem $realMarkedComplexProblems `
    'value.complex is true but every integer imaginary component is 0' `
    'v2 complex marker'

$integerTolerance = Copy-JsonValue $v2Unsigned
$integerTolerance.value | Add-Member `
    -NotePropertyName tolerance `
    -NotePropertyValue ([pscustomobject]@{ absolute = 1.0 })
$integerToleranceProblems = @(Compare-OpenMatObservation `
        $v2Unsigned $integerTolerance `
        'v2_integer_range' -SchemaVersion 2)
Assert-ContainsProblem $integerToleranceProblems `
    'expected.value.tolerance is valid only for an expected numeric value' `
    'v2 tolerance kind restriction'

$nestedCell = @'
{
  "schema_version": 2,
  "case_id": "v2_cell_fixture",
  "oracle": {"name": "fixture", "release": "test"},
  "outcome": "ok",
  "value": {
    "class": "cell", "size": [1, 1], "ndims": 2, "numel": 1,
    "complex": false, "kind": "cell",
    "items": [{
      "class": "char", "size": [1, 1], "ndims": 2, "numel": 1,
      "complex": false, "kind": "char", "code_units": [65]
    }]
  }
}
'@ | ConvertFrom-Json -Depth 100
$badNestedCell = Copy-JsonValue $nestedCell
$badNestedCell.value.items[0].numel = 2
$badNestedProblems = @(Compare-OpenMatObservation `
        $badNestedCell $nestedCell 'v2_cell_fixture' -SchemaVersion 2)
Assert-ContainsProblem $badNestedProblems `
    'value.items[0].numel differs from value.items[0].size product' `
    'v2 recursive value validation'

$tooLongUnits = [object[]]::new(16385)
for ($index = 0; $index -lt $tooLongUnits.Count; $index++) {
    $tooLongUnits[$index] = 65
}
$tooLongString = Copy-JsonValue $v2String
$tooLongString.value.size = [object[]] @(1, 1)
$tooLongString.value.numel = 1
$tooLongString.value.string_code_units = [object[]] @(, $tooLongUnits)
$tooLongString.value.missing = [object[]] @($false)
$tooLongProblems = @(Compare-OpenMatObservation `
        $tooLongString $tooLongString 'v2_string_fixture' -SchemaVersion 2)
Assert-ContainsProblem $tooLongProblems `
    'value.string_code_units element 0 exceeds 16384 code units' `
    'v2 per-string limit'

$aggregateStrings = [object[]]::new(5)
for ($outer = 0; $outer -lt $aggregateStrings.Count; $outer++) {
    $units = [object[]]::new(16384)
    for ($inner = 0; $inner -lt $units.Count; $inner++) { $units[$inner] = 65 }
    $aggregateStrings[$outer] = $units
}
$aggregateString = Copy-JsonValue $v2String
$aggregateString.value.size = [object[]] @(1, 5)
$aggregateString.value.numel = 5
$aggregateString.value.string_code_units = $aggregateStrings
$aggregateString.value.missing = [object[]] @($false, $false, $false, $false, $false)
$aggregateProblems = @(Compare-OpenMatObservation `
        $aggregateString $aggregateString 'v2_string_fixture' -SchemaVersion 2)
Assert-ContainsProblem $aggregateProblems `
    'value.string_code_units[4] traversal exceeds 65536 UTF-16 code units' `
    'v2 aggregate string limit'

$positiveAggregateFixtures = @(
    'aggregate-empty-cell',
    'aggregate-nested',
    'aggregate-heterogeneous-cell',
    'aggregate-struct-array',
    'aggregate-unfielded-scalar-struct',
    'aggregate-unfielded-empty-struct',
    'aggregate-fielded-shaped-empty-struct',
    'aggregate-table-nested',
    'aggregate-table-nx0'
)
foreach ($fixtureName in $positiveAggregateFixtures) {
    $schemas = Test-AggregateFixtureSchemas $fixtureName
    Assert-Equal $schemas.Observation $true "$fixtureName observation schema"
    Assert-Equal $schemas.Case $true "$fixtureName case schema"
    $fixture = Read-Fixture $fixtureName
    $fixtureProblems = @(Compare-OpenMatObservation `
            $fixture $fixture ([string] $fixture.case_id) -SchemaVersion 2)
    Assert-Equal $fixtureProblems.Count 0 "$fixtureName semantic golden"
}

$negativeSchemaExpectations = @(
    @('aggregate-invalid-shape', $true),
    @('aggregate-invalid-field-schema', $false),
    @('aggregate-invalid-record-width', $true),
    @('aggregate-invalid-unfielded-record-key', $true),
    @('aggregate-invalid-field-name', $false),
    @('aggregate-budget-numel', $true),
    @('aggregate-truncated', $false),
    @('aggregate-unknown-kind', $false),
    @('aggregate-object-leaf', $true)
)
foreach ($expectation in $negativeSchemaExpectations) {
    $schemas = Test-AggregateFixtureSchemas $expectation[0]
    Assert-Equal $schemas.Observation $expectation[1] `
        "$($expectation[0]) observation schema result"
    Assert-Equal $schemas.Case $expectation[1] `
        "$($expectation[0]) case schema result"
}

$nestedTable = Read-Fixture 'aggregate-table-nested'
$nestedTableProblems = @(Compare-OpenMatObservation `
        $nestedTable $nestedTable 'aggregate_table_nested' -SchemaVersion 2)
Assert-Equal $nestedTableProblems.Count 0 'nested table exact comparison'

$differentTableNames = Copy-JsonValue $nestedTable
$differentTableNames.value.items[0].variableNames = `
    [object[]] @('Label', 'Score')
$differentTableNameProblems = @(Compare-OpenMatObservation `
        $nestedTable $differentTableNames 'aggregate_table_nested' `
        -SchemaVersion 2)
Assert-ContainsProblem $differentTableNameProblems `
    'value.items[0].variableNames differs' `
    'nested table variable-name order difference path'

$differentTableVariable = Copy-JsonValue $nestedTable
$differentTableVariable.value.items[0].variables[1].string_code_units[0][0] = 67
$differentTableVariableProblems = @(Compare-OpenMatObservation `
        $differentTableVariable $nestedTable 'aggregate_table_nested' `
        -SchemaVersion 2)
Assert-ContainsProblem $differentTableVariableProblems `
    'value.items[0].variables[1].string_code_units differs' `
    'nested table variable difference path'

$duplicateTableNames = Copy-JsonValue $nestedTable
$duplicateTableNames.value.items[0].variableNames = `
    [object[]] @('Score', 'Score')
$duplicateTableSchemas = Test-AggregateObservationSchemas $duplicateTableNames
Assert-Equal $duplicateTableSchemas.Observation $false `
    'duplicate table names observation schema'
Assert-Equal $duplicateTableSchemas.Case $false `
    'duplicate table names case schema'
$duplicateTableProblems = @(Test-OpenMatExpected `
        $duplicateTableNames -SchemaVersion 2)
Assert-ContainsProblem $duplicateTableProblems `
    'expected.value.items[0].variableNames contains a duplicate name' `
    'duplicate table names semantic validation'

$tableWidthMismatch = Copy-JsonValue $nestedTable
$tableWidthMismatch.value.items[0].size = [object[]] @(2, 3)
$tableWidthMismatch.value.items[0].numel = 6
$tableWidthSchemas = Test-AggregateObservationSchemas $tableWidthMismatch
Assert-Equal $tableWidthSchemas.Observation $true `
    'table width cross-field observation structure'
Assert-Equal $tableWidthSchemas.Case $true `
    'table width cross-field case structure'
$tableWidthProblems = @(Test-OpenMatExpected `
        $tableWidthMismatch -SchemaVersion 2)
Assert-ContainsProblem $tableWidthProblems `
    'expected.value.items[0].variableNames count differs from expected.value.items[0].size[1]' `
    'table variable-name width validation'
Assert-ContainsProblem $tableWidthProblems `
    'expected.value.items[0].variables count differs from expected.value.items[0].size[1]' `
    'table variable payload width validation'

$tableRowMismatch = Copy-JsonValue $nestedTable
$rowVariable = $tableRowMismatch.value.items[0].variables[0]
$rowVariable.size = [object[]] @(3, 2)
$rowVariable.numel = 6
$rowVariable.real = [object[]] @('1', '2', '5', '3', '4', '6')
$rowVariable.imag = [object[]] @('0', '0', '0', '0', '0', '0')
$tableRowSchemas = Test-AggregateObservationSchemas $tableRowMismatch
Assert-Equal $tableRowSchemas.Observation $true `
    'table row cross-field observation structure'
Assert-Equal $tableRowSchemas.Case $true `
    'table row cross-field case structure'
$tableRowProblems = @(Test-OpenMatExpected $tableRowMismatch -SchemaVersion 2)
Assert-ContainsProblem $tableRowProblems `
    'expected.value.items[0].variables[0].size[0] differs from expected.value.items[0].size[0]' `
    'table variable row validation'

$tableUnknown = Copy-JsonValue $nestedTable
$tableUnknown.value.items[0] | Add-Member `
    -NotePropertyName unknownTableField -NotePropertyValue $true
$tableUnknownSchemas = Test-AggregateObservationSchemas $tableUnknown
Assert-Equal $tableUnknownSchemas.Observation $false `
    'unknown table field observation schema'
Assert-Equal $tableUnknownSchemas.Case $false `
    'unknown table field case schema'
$tableUnknownProblems = @(Test-OpenMatExpected $tableUnknown -SchemaVersion 2)
Assert-ContainsProblem $tableUnknownProblems `
    'expected.value.items[0].unknownTableField is not allowed' `
    'unknown table field semantic validation'

$invalidShape = Read-Fixture 'aggregate-invalid-shape'
$invalidShapeProblems = @(Test-OpenMatExpected $invalidShape -SchemaVersion 2)
Assert-ContainsProblem $invalidShapeProblems `
    'expected.value.numel differs from expected.value.size product' `
    'aggregate shape product'

$invalidFieldSchema = Read-Fixture 'aggregate-invalid-field-schema'
$invalidFieldSchemaProblems = @(
    Test-OpenMatExpected $invalidFieldSchema -SchemaVersion 2
)
Assert-ContainsProblem $invalidFieldSchemaProblems `
    'expected.value.fields contains a duplicate field name' `
    'aggregate duplicate field schema'

$invalidRecord = Read-Fixture 'aggregate-invalid-record-width'
$invalidRecordProblems = @(Test-OpenMatExpected $invalidRecord -SchemaVersion 2)
Assert-ContainsProblem $invalidRecordProblems `
    'expected.value.records[0] fields differ from expected.value.fields' `
    'aggregate nonempty schema rejects an empty record key set'

$invalidUnfieldedRecord = Read-Fixture 'aggregate-invalid-unfielded-record-key'
$invalidUnfieldedRecordProblems = @(
    Test-OpenMatExpected $invalidUnfieldedRecord -SchemaVersion 2
)
Assert-ContainsProblem $invalidUnfieldedRecordProblems `
    'expected.value.records[0] fields differ from expected.value.fields' `
    'aggregate empty schema rejects a nonempty record key set'

$invalidFieldName = Read-Fixture 'aggregate-invalid-field-name'
$invalidFieldNameProblems = @(
    Test-OpenMatExpected $invalidFieldName -SchemaVersion 2
)
Assert-ContainsProblem $invalidFieldNameProblems `
    'expected.value.fields contains an invalid field name' `
    'aggregate ASCII field name'

$numelBudget = Read-Fixture 'aggregate-budget-numel'
$numelBudgetProblems = @(Test-OpenMatExpected $numelBudget -SchemaVersion 2)
Assert-ContainsProblem $numelBudgetProblems `
    'expected.value.numel exceeds 4096 elements' `
    'aggregate top-level element budget'

$truncated = Read-Fixture 'aggregate-truncated'
$truncatedProblems = @(Test-OpenMatExpected $truncated -SchemaVersion 2)
Assert-ContainsProblem $truncatedProblems `
    'expected.value.truncation is not allowed' `
    'aggregate nested truncation metadata'
Assert-ContainsProblem $truncatedProblems `
    'expected.value.items count differs from expected.value.numel' `
    'aggregate complete cell payload'

$unknownKind = Read-Fixture 'aggregate-unknown-kind'
$unknownKindProblems = @(Test-OpenMatExpected $unknownKind -SchemaVersion 2)
Assert-ContainsProblem $unknownKindProblems `
    'expected.value.kind is unknown' 'aggregate unknown kind'

$objectLeaf = Read-Fixture 'aggregate-object-leaf'
$objectLeafProblems = @(Test-OpenMatExpected $objectLeaf -SchemaVersion 2)
Assert-ContainsProblem $objectLeafProblems `
    'expected.value.items[0].kind object is unsupported' `
    'aggregate object leaf policy'

$structArray = Read-Fixture 'aggregate-struct-array'
$differentFieldOrder = Copy-JsonValue $structArray
$differentFieldOrder.value.fields = [object[]] @('alpha', 'beta')
$fieldOrderProblems = @(Compare-OpenMatObservation `
        $structArray $differentFieldOrder 'aggregate_struct_array' `
        -SchemaVersion 2)
Assert-Equal $fieldOrderProblems.Count 1 'aggregate field order problem count'
Assert-Equal $fieldOrderProblems[0] 'value.fields differs' `
    'aggregate field order path'

$differentMemberOrder = Copy-JsonValue $structArray
for ($recordIndex = 0; $recordIndex -lt $differentMemberOrder.value.records.Count;
        $recordIndex++) {
    $record = $differentMemberOrder.value.records[$recordIndex]
    $differentMemberOrder.value.records[$recordIndex] = [pscustomobject][ordered]@{
        alpha = $record.alpha
        beta = $record.beta
    }
}
$memberOrderProblems = @(Compare-OpenMatObservation `
        $differentMemberOrder $structArray 'aggregate_struct_array' `
        -SchemaVersion 2)
Assert-Equal $memberOrderProblems.Count 0 'record JSON member order is ignored'

$differentNestedField = Copy-JsonValue $structArray
$differentNestedField.value.records[2].alpha.logical[0] = $false
$nestedFieldProblems = @(Compare-OpenMatObservation `
        $differentNestedField $structArray 'aggregate_struct_array' `
        -SchemaVersion 2)
Assert-ContainsProblem $nestedFieldProblems `
    'value.records[2].alpha.logical differs' `
    'nested struct difference path'

$heterogeneousCell = Read-Fixture 'aggregate-heterogeneous-cell'
$differentCellItem = Copy-JsonValue $heterogeneousCell
$differentCellItem.value.items[1].code_units[0] = 121
$nestedCellProblems = @(Compare-OpenMatObservation `
        $differentCellItem $heterogeneousCell `
        'aggregate_heterogeneous_cell' -SchemaVersion 2)
Assert-ContainsProblem $nestedCellProblems `
    'value.items[1].code_units differs' 'nested cell difference path'

$nestedToleranceExpected = Read-Fixture 'aggregate-nested'
$nestedToleranceActual = Copy-JsonValue $nestedToleranceExpected
$nestedToleranceActual.value.items[0].real[0] = '7.0005'
$nestedToleranceExpected.value.items[0] | Add-Member `
    -NotePropertyName tolerance `
    -NotePropertyValue ([pscustomobject]@{ absolute = 0.001 })
$nestedToleranceProblems = @(Compare-OpenMatObservation `
        $nestedToleranceActual $nestedToleranceExpected 'aggregate_nested' `
        -SchemaVersion 2)
Assert-Equal $nestedToleranceProblems.Count 0 `
    'numeric tolerance applies at a nested numeric leaf'

$deepValue = New-EmptyLogicalValue
for ($level = 0; $level -lt 33; $level++) {
    $deepValue = New-CellValue ([object[]] @(,$deepValue))
}
$depthProblems = @(Test-OpenMatExpected `
        (New-V2Observation 'aggregate_depth_limit' $deepValue) `
        -SchemaVersion 2)
Assert-ProblemLike $depthProblems `
    '* depth exceeds 32' 'aggregate depth budget'

$sharedEmpty = New-EmptyLogicalValue
$childItems = [object[]]::new(4)
for ($index = 0; $index -lt $childItems.Count; $index++) {
    $childItems[$index] = $sharedEmpty
}
$sharedChild = New-CellValue $childItems
$nodeItems = [object[]]::new(4096)
for ($index = 0; $index -lt $nodeItems.Count; $index++) {
    $nodeItems[$index] = $sharedChild
}
$nodeProblems = @(Test-OpenMatExpected `
        (New-V2Observation 'aggregate_node_limit' (New-CellValue $nodeItems)) `
        -SchemaVersion 2)
Assert-ProblemLike $nodeProblems '* traversal exceeds 16384 aggregate nodes' `
    'aggregate node budget and completed-sibling sharing'

$zeroPayload = [object[]]::new(4096)
for ($index = 0; $index -lt $zeroPayload.Count; $index++) {
    $zeroPayload[$index] = '0'
}
$largeNumeric = [pscustomobject]@{
    class = 'double'
    size = [object[]] @(1, 4096)
    ndims = 2
    numel = 4096
    complex = $false
    kind = 'numeric'
    real = $zeroPayload
    imag = $zeroPayload
}
$elementItems = [object[]]::new(17)
for ($index = 0; $index -lt $elementItems.Count; $index++) {
    $elementItems[$index] = $largeNumeric
}
$elementProblems = @(Test-OpenMatExpected `
        (New-V2Observation `
            'aggregate_element_limit' (New-CellValue $elementItems)) `
        -SchemaVersion 2)
Assert-ProblemLike $elementProblems `
    '* traversal exceeds 65536 aggregate elements' 'aggregate element budget'

$codeUnits = [object[]]::new(16384)
for ($index = 0; $index -lt $codeUnits.Count; $index++) {
    $codeUnits[$index] = 65
}
$largeString = [pscustomobject]@{
    class = 'string'
    size = [object[]] @(1, 1)
    ndims = 2
    numel = 1
    complex = $false
    kind = 'string'
    string_code_units = [object[]] @(, $codeUnits)
    missing = [object[]] @($false)
}
$stringItems = [object[]]::new(5)
for ($index = 0; $index -lt $stringItems.Count; $index++) {
    $stringItems[$index] = $largeString
}
$codeUnitProblems = @(Test-OpenMatExpected `
        (New-V2Observation `
            'aggregate_code_unit_limit' (New-CellValue $stringItems)) `
        -SchemaVersion 2)
Assert-ProblemLike $codeUnitProblems `
    '* traversal exceeds 65536 UTF-16 code units' `
    'aggregate recursive UTF-16 budget'

$fieldNames = [object[]]::new(1000)
for ($index = 0; $index -lt $fieldNames.Count; $index++) {
    $fieldNames[$index] = 'f{0:D4}_{1}' -f $index, ('a' * 64)
}
$fieldNameValue = [pscustomobject]@{
    class = 'struct'
    size = [object[]] @(0, 0)
    ndims = 2
    numel = 0
    complex = $false
    kind = 'struct'
    fields = $fieldNames
    records = [object[]] @()
}
$fieldCodeUnitProblems = @(Test-OpenMatExpected `
        (New-V2Observation 'aggregate_field_code_units' $fieldNameValue) `
        -SchemaVersion 2)
Assert-ProblemLike $fieldCodeUnitProblems `
    '* traversal exceeds 65536 UTF-16 code units' `
    'aggregate field schema UTF-16 budget'

$wideNumber = '1' + ('0' * 299)
$wideReal = [object[]]::new(4096)
for ($index = 0; $index -lt $wideReal.Count; $index++) {
    $wideReal[$index] = $wideNumber
}
$encodedValue = [pscustomobject]@{
    class = 'double'
    size = [object[]] @(1, 4096)
    ndims = 2
    numel = 4096
    complex = $false
    kind = 'numeric'
    real = $wideReal
    imag = $zeroPayload
}
$encodedProblems = @(Test-OpenMatExpected `
        (New-V2Observation 'aggregate_encoded_limit' $encodedValue) `
        -SchemaVersion 2)
Assert-ContainsProblem $encodedProblems `
    'expected encoded UTF-8 JSON exceeds 1048576 bytes' `
    'aggregate encoded observation budget'

$cyclicValue = [pscustomobject]@{
    class = 'cell'
    size = [object[]] @(1, 1)
    ndims = 2
    numel = 1
    complex = $false
    kind = 'cell'
    items = [object[]]::new(1)
}
$cyclicValue.items[0] = $cyclicValue
$cycleProblems = @(Test-OpenMatExpected `
        (New-V2Observation 'aggregate_cycle' $cyclicValue) -SchemaVersion 2)
Assert-ContainsProblem $cycleProblems `
    'expected.value.items[0] contains a cyclic value' `
    'aggregate active-path cycle'

$unknownVersionProblems = @(Compare-OpenMatObservation `
        $v2String $v2String 'v2_string_fixture' -SchemaVersion 99)
Assert-Equal $unknownVersionProblems.Count 1 'unknown comparator version count'
Assert-Equal $unknownVersionProblems[0] `
    'unsupported comparator schema_version 99' 'unknown comparator version'

Write-Output 'Comparator fixture tests passed.'
