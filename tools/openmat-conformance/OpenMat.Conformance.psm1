Set-StrictMode -Version Latest

function Test-OpenMatProperty {
    param(
        [Parameter(Mandatory)] [object] $InputObject,
        [Parameter(Mandatory)] [string] $Name
    )

    return $null -ne $InputObject.PSObject.Properties[$Name]
}

function Get-OpenMatProperty {
    param(
        [Parameter(Mandatory)] [object] $InputObject,
        [Parameter(Mandatory)] [string] $Name
    )

    if (Test-OpenMatProperty $InputObject $Name) {
        return $InputObject.$Name
    }
    return $null
}

function ConvertTo-OpenMatCompactJson {
    param([AllowNull()] [object] $InputObject)

    return ConvertTo-Json -InputObject $InputObject -Depth 100 -Compress
}

function Compare-OpenMatExactSequence {
    param(
        [AllowNull()] [object] $Actual,
        [AllowNull()] [object] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    $actualItems = @($Actual)
    $expectedItems = @($Expected)
    if ($actualItems.Count -ne $expectedItems.Count) {
        return "$Label count differs"
    }
    if ((ConvertTo-OpenMatCompactJson $actualItems) -cne
            (ConvertTo-OpenMatCompactJson $expectedItems)) {
        return "$Label differs"
    }
    return $null
}

function ConvertTo-OpenMatFiniteDouble {
    param([Parameter(Mandatory)] [string] $Text)

    return [double]::Parse(
        $Text,
        [System.Globalization.NumberStyles]::Float,
        [System.Globalization.CultureInfo]::InvariantCulture
    )
}

function Compare-OpenMatNumericSequence {
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
        return Compare-OpenMatExactSequence $actualItems $expectedItems $Label
    }

    $absolute = if (Test-OpenMatProperty $Tolerance 'absolute') {
        [double] $Tolerance.absolute
    } else {
        0.0
    }
    $relative = if (Test-OpenMatProperty $Tolerance 'relative') {
        [double] $Tolerance.relative
    } else {
        0.0
    }
    $special = @('NaN', '+Inf', '-Inf')
    for ($index = 0; $index -lt $actualItems.Count; $index++) {
        $actualText = [string] $actualItems[$index]
        $expectedText = [string] $expectedItems[$index]
        if ($special -ccontains $actualText -or $special -ccontains $expectedText) {
            if ($actualText -cne $expectedText) {
                return "$Label element $index differs"
            }
            continue
        }
        try {
            $actualNumber = ConvertTo-OpenMatFiniteDouble $actualText
            $expectedNumber = ConvertTo-OpenMatFiniteDouble $expectedText
        } catch {
            return "$Label element $index is not a finite number string"
        }
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

function Compare-OpenMatValueV1 {
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected,
        [AllowNull()] [object] $Tolerance
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($name in @('class', 'ndims', 'numel', 'kind')) {
        if (-not (Test-OpenMatProperty $Actual $name)) {
            $problems.Add("value.$name is missing")
        } elseif (-not (Test-OpenMatProperty $Expected $name) -or
                [string] $Actual.$name -cne [string] $Expected.$name) {
            $problems.Add("value.$name differs")
        }
    }
    $sizeProblem = Compare-OpenMatExactSequence `
        (Get-OpenMatProperty $Actual 'size') `
        (Get-OpenMatProperty $Expected 'size') `
        'value.size'
    if ($null -ne $sizeProblem) {
        $problems.Add($sizeProblem)
    }

    $kind = [string] (Get-OpenMatProperty $Expected 'kind')
    switch ($kind) {
        'numeric' {
            foreach ($component in @('real', 'imag')) {
                $problem = Compare-OpenMatNumericSequence `
                    (Get-OpenMatProperty $Actual $component) `
                    (Get-OpenMatProperty $Expected $component) `
                    $Tolerance "value.$component"
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
        'integer' {
            $problem = Compare-OpenMatExactSequence `
                (Get-OpenMatProperty $Actual 'integer') `
                (Get-OpenMatProperty $Expected 'integer') 'value.integer'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'logical' {
            $problem = Compare-OpenMatExactSequence `
                (Get-OpenMatProperty $Actual 'logical') `
                (Get-OpenMatProperty $Expected 'logical') 'value.logical'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'char' {
            $problem = Compare-OpenMatExactSequence `
                (Get-OpenMatProperty $Actual 'code_units') `
                (Get-OpenMatProperty $Expected 'code_units') 'value.code_units'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'string' {
            foreach ($payload in @('string', 'missing')) {
                $problem = Compare-OpenMatExactSequence `
                    (Get-OpenMatProperty $Actual $payload) `
                    (Get-OpenMatProperty $Expected $payload) "value.$payload"
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
        'cell' {
            $problem = Compare-OpenMatExactSequence `
                (Get-OpenMatProperty $Actual 'items') `
                (Get-OpenMatProperty $Expected 'items') 'value.items'
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'struct' {
            foreach ($payload in @('fields', 'records')) {
                $problem = Compare-OpenMatExactSequence `
                    (Get-OpenMatProperty $Actual $payload) `
                    (Get-OpenMatProperty $Expected $payload) "value.$payload"
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
        'object' {
            foreach ($payload in @('fields', 'records')) {
                if ((Test-OpenMatProperty $Actual $payload) -or
                        (Test-OpenMatProperty $Expected $payload)) {
                    $problem = Compare-OpenMatExactSequence `
                        (Get-OpenMatProperty $Actual $payload) `
                        (Get-OpenMatProperty $Expected $payload) "value.$payload"
                    if ($null -ne $problem) { $problems.Add($problem) }
                }
            }
        }
    }
    return $problems.ToArray()
}

function Compare-OpenMatObservationV1 {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected,
        [Parameter(Mandatory)] [string] $CaseId,
        [AllowNull()] [object] $Tolerance
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($required in @('schema_version', 'case_id', 'oracle', 'outcome')) {
        if (-not (Test-OpenMatProperty $Actual $required)) {
            $problems.Add("$required is missing")
        }
    }
    if ($problems.Count -gt 0) {
        return $problems.ToArray()
    }
    if ([int] $Actual.schema_version -ne 1) {
        $problems.Add('schema_version is not 1')
    }
    if ([string] $Actual.case_id -cne $CaseId) {
        $problems.Add('case_id differs')
    }
    if (-not (Test-OpenMatProperty $Actual.oracle 'name') -or
            -not (Test-OpenMatProperty $Actual.oracle 'release')) {
        $problems.Add('oracle identity is incomplete')
    }

    $expectedObservation = $Expected
    if (Test-OpenMatProperty $Expected 'case_id') {
        if ([string] $Expected.case_id -cne $CaseId) {
            $problems.Add('expected case_id differs')
        }
    }
    if (-not (Test-OpenMatProperty $expectedObservation 'outcome')) {
        $problems.Add('expected outcome is missing')
        return $problems.ToArray()
    }
    if ([string] $Actual.outcome -cne [string] $expectedObservation.outcome) {
        $problems.Add('outcome differs')
        return $problems.ToArray()
    }

    if ([string] $expectedObservation.outcome -ceq 'ok') {
        if (-not (Test-OpenMatProperty $Actual 'value') -or
                -not (Test-OpenMatProperty $expectedObservation 'value')) {
            $problems.Add('value is missing')
            return $problems.ToArray()
        }
        if ($null -eq $Tolerance -and
                (Test-OpenMatProperty $expectedObservation.value 'tolerance')) {
            $Tolerance = $expectedObservation.value.tolerance
        }
        foreach ($problem in @(Compare-OpenMatValueV1 `
                    $Actual.value $expectedObservation.value $Tolerance)) {
            if ($null -ne $problem) { $problems.Add($problem) }
        }
    } else {
        if (-not (Test-OpenMatProperty $Actual 'error') -or
                -not (Test-OpenMatProperty $expectedObservation 'error')) {
            $problems.Add('error is missing')
        } elseif ([string] $Actual.error.category -cne
                [string] $expectedObservation.error.category) {
            $problems.Add('error.category differs')
        }
    }
    return $problems.ToArray()
}

function Test-OpenMatJsonInteger {
    param([AllowNull()] [object] $Value)

    if ($null -eq $Value -or $Value -is [bool] -or $Value -is [string]) {
        return $false
    }
    if ($Value -is [System.Numerics.BigInteger]) {
        return $true
    }
    if ($Value -isnot [ValueType]) {
        return $false
    }
    try {
        $decimal = [decimal] $Value
        return $decimal -eq [decimal]::Truncate($decimal)
    } catch {
        return $false
    }
}

function ConvertTo-OpenMatBigInteger {
    param([Parameter(Mandatory)] [object] $Value)

    if ($Value -is [System.Numerics.BigInteger]) {
        return $Value
    }
    $text = ([decimal] $Value).ToString(
        '0',
        [System.Globalization.CultureInfo]::InvariantCulture
    )
    return [System.Numerics.BigInteger]::Parse(
        $text,
        [System.Globalization.CultureInfo]::InvariantCulture
    )
}

function Test-OpenMatV2NumberString {
    param([AllowNull()] [object] $Value)

    return $Value -is [string] -and
        [string] $Value -cmatch '^(NaN|[+-]Inf|-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?)$'
}

function Test-OpenMatV2DecimalInteger {
    param(
        [AllowNull()] [object] $Value,
        [switch] $Unsigned
    )

    if ($Value -isnot [string]) {
        return $false
    }
    if ($Unsigned) {
        return [string] $Value -cmatch '^(0|[1-9][0-9]*)$'
    }
    return [string] $Value -cmatch '^(0|-?[1-9][0-9]*)$'
}

function Get-OpenMatV2Sequence {
    param(
        [Parameter(Mandatory)] [object] $Value,
        [Parameter(Mandatory)] [string] $Name
    )

    if (-not (Test-OpenMatProperty $Value $Name)) {
        return @()
    }
    return @($Value.$Name)
}

function Test-OpenMatV2PayloadCount {
    param(
        [Parameter(Mandatory)] [object] $Value,
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [long] $Numel,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [AllowEmptyCollection()]
        [System.Collections.Generic.List[string]] $Problems
    )

    if (Test-OpenMatProperty $Value $Name) {
        if ($Value.$Name -isnot [array]) {
            $Problems.Add("$Path.$Name is not an array")
            return
        }
        $count = @(Get-OpenMatV2Sequence $Value $Name).Count
        if ($count -ne $Numel) {
            $Problems.Add("$Path.$Name count differs from $Path.numel")
        }
    }
}

function Test-OpenMatV2Tolerance {
    param(
        [Parameter(Mandatory)] [AllowNull()] [object] $Tolerance,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [AllowEmptyCollection()]
        [System.Collections.Generic.List[string]] $Problems
    )

    if ($null -eq $Tolerance) {
        $Problems.Add("$Path is null")
        return
    }
    foreach ($property in @($Tolerance.PSObject.Properties)) {
        if ($property.Name -notin @('absolute', 'relative')) {
            $Problems.Add("$Path.$($property.Name) is not allowed")
        }
    }
    foreach ($name in @('absolute', 'relative')) {
        if (Test-OpenMatProperty $Tolerance $name) {
            try {
                $number = [double] $Tolerance.$name
                if ([double]::IsNaN($number) -or [double]::IsInfinity($number) -or
                        $number -lt 0) {
                    throw 'invalid tolerance'
                }
            } catch {
                $Problems.Add("$Path.$name is not a non-negative finite number")
            }
        }
    }
}

function New-OpenMatV2ValidationState {
    return [pscustomobject]@{
        Nodes = [System.Numerics.BigInteger]::Zero
        Elements = [System.Numerics.BigInteger]::Zero
        CodeUnits = [System.Numerics.BigInteger]::Zero
        Active = [System.Collections.Generic.HashSet[object]]::new(
            [System.Collections.Generic.ReferenceEqualityComparer]::Instance
        )
        ReportedLimits = [System.Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal
        )
    }
}

function Add-OpenMatV2TraversalCharge {
    param(
        [Parameter(Mandatory)] [object] $State,
        [Parameter(Mandatory)]
        [ValidateSet('Nodes', 'Elements', 'CodeUnits')] [string] $Counter,
        [Parameter(Mandatory)] [System.Numerics.BigInteger] $Amount,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [AllowEmptyCollection()]
        [System.Collections.Generic.List[string]] $Problems
    )

    $State.$Counter = [System.Numerics.BigInteger] $State.$Counter + $Amount
    $limit = switch ($Counter) {
        'Nodes' { [System.Numerics.BigInteger] 16384 }
        'Elements' { [System.Numerics.BigInteger] 65536 }
        'CodeUnits' { [System.Numerics.BigInteger] 65536 }
    }
    if ($State.$Counter -gt $limit -and $State.ReportedLimits.Add($Counter)) {
        $label = switch ($Counter) {
            'Nodes' { '16384 aggregate nodes' }
            'Elements' { '65536 aggregate elements' }
            'CodeUnits' { '65536 UTF-16 code units' }
        }
        $Problems.Add("$Path traversal exceeds $label")
    }
}

function Test-OpenMatV2NumberIsZero {
    param([Parameter(Mandatory)] [string] $Value)

    if ($Value -in @('NaN', '+Inf', '-Inf')) {
        return $false
    }
    $mantissa = ($Value -split '[eE]', 2)[0].TrimStart('-').Replace('.', '')
    return $mantissa -cmatch '^0+$'
}

function Test-OpenMatV2EncodedSize {
    param(
        [Parameter(Mandatory)] [object] $Value,
        [Parameter(Mandatory)] [string] $Path
    )

    try {
        $json = ConvertTo-OpenMatCompactJson $Value
        $byteCount = [System.Text.Encoding]::UTF8.GetByteCount($json)
    } catch {
        return @("$Path cannot be encoded as one complete JSON value")
    }
    if ($byteCount -gt 1048576) {
        return @("$Path encoded UTF-8 JSON exceeds 1048576 bytes")
    }
    return @()
}

function Test-OpenMatV2Value {
    param(
        [Parameter(Mandatory)] [AllowNull()] [object] $Value,
        [Parameter(Mandatory)] [string] $Path,
        [switch] $AllowTolerance,
        [AllowNull()] [object] $State,
        [int] $Depth = 0
    )

    if ($null -eq $Value) {
        return @("$Path is null")
    }
    if ($null -eq $State) {
        $State = New-OpenMatV2ValidationState
    }
    if (-not $State.Active.Add($Value)) {
        return @("$Path contains a cyclic value")
    }
    try {
        return @(Test-OpenMatV2ValueNode `
            $Value $Path -AllowTolerance:$AllowTolerance `
            -State $State -Depth $Depth)
    } finally {
        [void] $State.Active.Remove($Value)
    }
}

function Test-OpenMatV2ValueNode {
    param(
        [Parameter(Mandatory)] [AllowNull()] [object] $Value,
        [Parameter(Mandatory)] [string] $Path,
        [switch] $AllowTolerance,
        [Parameter(Mandatory)] [object] $State,
        [Parameter(Mandatory)] [int] $Depth
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    $requiredMetadata = @('class', 'size', 'ndims', 'numel', 'complex', 'kind')
    foreach ($name in $requiredMetadata) {
        if (-not (Test-OpenMatProperty $Value $name)) {
            $problems.Add("$Path.$name is missing")
        }
    }
    if ($problems.Count -gt 0) {
        return $problems.ToArray()
    }

    $kind = [string] $Value.kind
    $knownKinds = @(
        'numeric', 'integer', 'logical', 'char',
        'string', 'cell', 'struct', 'table', 'object'
    )
    if ($kind -cnotin $knownKinds) {
        $problems.Add("$Path.kind is unknown")
        return $problems.ToArray()
    }
    if ($kind -ceq 'object') {
        $problems.Add("$Path.kind object is unsupported")
        return $problems.ToArray()
    }
    if ($Value.class -isnot [string] -or [string]::IsNullOrWhiteSpace($Value.class)) {
        $problems.Add("$Path.class is invalid")
    }
    if ($Value.complex -isnot [bool]) {
        $problems.Add("$Path.complex is not boolean")
    }
    if ($Depth -gt 32) {
        if ($State.ReportedLimits.Add('Depth')) {
            $problems.Add("$Path depth exceeds 32")
        }
        return $problems.ToArray()
    }

    $size = @($Value.size)
    $sizeValid = $size.Count -ge 2
    if ($Value.size -isnot [array]) {
        $problems.Add("$Path.size is not an array")
        $sizeValid = $false
    }
    if (-not $sizeValid) {
        $problems.Add("$Path.size has fewer than two dimensions")
    }
    $product = [System.Numerics.BigInteger]::One
    foreach ($dimension in $size) {
        if (-not (Test-OpenMatJsonInteger $dimension)) {
            $problems.Add("$Path.size contains a non-integer dimension")
            $sizeValid = $false
            continue
        }
        $dimensionValue = ConvertTo-OpenMatBigInteger $dimension
        if ($dimensionValue -lt 0 -or
                $dimensionValue -gt 9007199254740991) {
            $problems.Add("$Path.size contains an out-of-range dimension")
            $sizeValid = $false
            continue
        }
        $product *= $dimensionValue
        if ($product -gt 9007199254740991) {
            $problems.Add("$Path.size product exceeds the safe integer range")
            $sizeValid = $false
        }
    }

    $ndimsValid = Test-OpenMatJsonInteger $Value.ndims
    if (-not $ndimsValid) {
        $problems.Add("$Path.ndims is not an integer")
    } else {
        $ndimsValue = ConvertTo-OpenMatBigInteger $Value.ndims
        if ($ndimsValue -lt 2 -or $ndimsValue -gt 9007199254740991) {
            $problems.Add("$Path.ndims is outside the safe rank range")
        }
        if ($ndimsValue -ne $size.Count) {
            $problems.Add("$Path.ndims differs from $Path.size count")
        }
    }

    $numelValid = Test-OpenMatJsonInteger $Value.numel
    $numel = 0L
    $numelWithinNodeLimit = $false
    if (-not $numelValid) {
        $problems.Add("$Path.numel is not an integer")
    } else {
        $numelValue = ConvertTo-OpenMatBigInteger $Value.numel
        if ($numelValue -lt 0 -or $numelValue -gt 9007199254740991) {
            $problems.Add("$Path.numel is outside the safe integer range")
            $numelValid = $false
        } else {
            $numel = [long] $numelValue
            if ($sizeValid -and $numelValue -ne $product) {
                $problems.Add("$Path.numel differs from $Path.size product")
            }
            if ($numelValue -gt 4096) {
                $problems.Add("$Path.numel exceeds 4096 elements")
            } else {
                $numelWithinNodeLimit = $true
            }
        }
    }

    if ($numelValid -and $sizeValid -and $numelValue -eq $product -and
            $numelWithinNodeLimit) {
        Add-OpenMatV2TraversalCharge `
            $State Nodes ([System.Numerics.BigInteger]::One) $Path $problems
        Add-OpenMatV2TraversalCharge `
            $State Elements $numelValue $Path $problems
    }

    $payloadNames = @(
        'real', 'imag', 'integer', 'logical', 'code_units',
        'string_code_units', 'missing', 'items', 'fields', 'records',
        'variableNames', 'variables'
    )
    $allowedPayloads = switch ($kind) {
        'numeric' { @('real', 'imag') }
        'integer' { @('integer') }
        'logical' { @('logical') }
        'char' { @('code_units') }
        'string' { @('string_code_units', 'missing') }
        'cell' { @('items') }
        'struct' { @('fields', 'records') }
        'table' { @('variableNames', 'variables') }
    }
    $knownProperties = @($requiredMetadata) + $payloadNames + @('tolerance')
    foreach ($property in @($Value.PSObject.Properties)) {
        if ($property.Name -cnotin $knownProperties) {
            $problems.Add("$Path.$($property.Name) is not allowed")
        }
    }
    foreach ($name in $payloadNames) {
        if ((Test-OpenMatProperty $Value $name) -and $name -cnotin $allowedPayloads) {
            $problems.Add("$Path.$name is not valid for kind $kind")
        }
    }
    $requiredPayloads = switch ($kind) {
        'numeric' { @('real', 'imag') }
        'integer' { @('integer') }
        'logical' { @('logical') }
        'char' { @('code_units') }
        'string' { @('string_code_units', 'missing') }
        'cell' { @('items') }
        'struct' { @('fields', 'records') }
        'table' { @('variableNames', 'variables') }
        default { @() }
    }
    foreach ($name in $requiredPayloads) {
        if (-not (Test-OpenMatProperty $Value $name)) {
            $problems.Add("$Path.$name is missing for kind $kind")
        }
    }

    if (Test-OpenMatProperty $Value 'tolerance') {
        if (-not $AllowTolerance -or $kind -cne 'numeric') {
            $problems.Add("$Path.tolerance is valid only for an expected numeric value")
        } else {
            Test-OpenMatV2Tolerance $Value.tolerance "$Path.tolerance" $problems
        }
    }

    if ($numelValid) {
        foreach ($name in $allowedPayloads) {
            if ($name -cnotin @('fields', 'variableNames', 'variables')) {
                Test-OpenMatV2PayloadCount $Value $name $numel $Path $problems
            }
        }
    }

    if (-not $numelWithinNodeLimit) {
        return $problems.ToArray()
    }

    switch ($kind) {
        'numeric' {
            if ([string] $Value.class -cnotin @('double', 'single')) {
                $problems.Add("$Path.class is invalid for numeric")
            }
            foreach ($name in @('real', 'imag')) {
                foreach ($item in @(Get-OpenMatV2Sequence $Value $name)) {
                    if (-not (Test-OpenMatV2NumberString $item)) {
                        $problems.Add("$Path.$name contains an invalid number string")
                        break
                    }
                }
            }
            if ($Value.complex -is [bool]) {
                if (-not $Value.complex) {
                    foreach ($item in @(Get-OpenMatV2Sequence $Value 'imag')) {
                        if ([string] $item -cne '0') {
                            $problems.Add("$Path.imag must contain only 0 when complex is false")
                            break
                        }
                    }
                } else {
                    $hasNonzeroImaginary = $false
                    foreach ($item in @(Get-OpenMatV2Sequence $Value 'imag')) {
                        if ((Test-OpenMatV2NumberString $item) -and
                                -not (Test-OpenMatV2NumberIsZero ([string] $item))) {
                            $hasNonzeroImaginary = $true
                            break
                        }
                    }
                    if (-not $hasNonzeroImaginary) {
                        $problems.Add("$Path.complex is true but every numeric imaginary component is 0")
                    }
                }
            }
        }
        'integer' {
            $ranges = @{
                int8 = @('-128', '127'); uint8 = @('0', '255')
                int16 = @('-32768', '32767'); uint16 = @('0', '65535')
                int32 = @('-2147483648', '2147483647')
                uint32 = @('0', '4294967295')
                int64 = @('-9223372036854775808', '9223372036854775807')
                uint64 = @('0', '18446744073709551615')
            }
            $class = [string] $Value.class
            if (-not $ranges.ContainsKey($class)) {
                $problems.Add("$Path.class is invalid for integer")
                break
            }
            $unsigned = $class.StartsWith('uint', [StringComparison]::Ordinal)
            $minimum = [System.Numerics.BigInteger]::Parse($ranges[$class][0])
            $maximum = [System.Numerics.BigInteger]::Parse($ranges[$class][1])
            $hasNonzeroImaginary = $false
            foreach ($item in @(Get-OpenMatV2Sequence $Value 'integer')) {
                if ($null -eq $item -or
                        -not (Test-OpenMatProperty $item 'real') -or
                        -not (Test-OpenMatProperty $item 'imaginary')) {
                    $problems.Add("$Path.integer contains an incomplete component")
                    continue
                }
                $componentNames = @($item.PSObject.Properties.Name)
                if ($componentNames.Count -ne 2 -or
                        'real' -cnotin $componentNames -or
                        'imaginary' -cnotin $componentNames) {
                    $problems.Add("$Path.integer contains a non-canonical component object")
                }
                foreach ($component in @('real', 'imaginary')) {
                    $text = $item.$component
                    if (-not (Test-OpenMatV2DecimalInteger $text -Unsigned:$unsigned)) {
                        $problems.Add("$Path.integer.$component is not canonical for $class")
                        continue
                    }
                    $number = [System.Numerics.BigInteger]::Parse([string] $text)
                    if ($number -lt $minimum -or $number -gt $maximum) {
                        $problems.Add("$Path.integer.$component is outside the $class range")
                    }
                    if ($component -ceq 'imaginary' -and $number -ne 0) {
                        $hasNonzeroImaginary = $true
                    }
                }
            }
            if ($Value.complex -is [bool]) {
                if ($Value.complex -and -not $hasNonzeroImaginary) {
                    $problems.Add("$Path.complex is true but every integer imaginary component is 0")
                } elseif (-not $Value.complex -and $hasNonzeroImaginary) {
                    $problems.Add("$Path.complex is false but an integer imaginary component is nonzero")
                }
            }
        }
        'logical' {
            if ([string] $Value.class -cne 'logical' -or
                    ($Value.complex -is [bool] -and $Value.complex)) {
                $problems.Add("$Path metadata is invalid for logical")
            }
            foreach ($item in @(Get-OpenMatV2Sequence $Value 'logical')) {
                if ($item -isnot [bool]) {
                    $problems.Add("$Path.logical contains a non-boolean value")
                    break
                }
            }
        }
        'char' {
            if ([string] $Value.class -cne 'char' -or
                    ($Value.complex -is [bool] -and $Value.complex)) {
                $problems.Add("$Path metadata is invalid for char")
            }
            foreach ($item in @(Get-OpenMatV2Sequence $Value 'code_units')) {
                if (-not (Test-OpenMatJsonInteger $item)) {
                    $problems.Add("$Path.code_units contains a non-integer code unit")
                    break
                }
                $codeUnit = ConvertTo-OpenMatBigInteger $item
                if ($codeUnit -lt 0 -or $codeUnit -gt 65535) {
                    $problems.Add("$Path.code_units contains an out-of-range code unit")
                    break
                }
            }
            if ($Value.code_units -is [array]) {
                Add-OpenMatV2TraversalCharge `
                    $State CodeUnits `
                    ([System.Numerics.BigInteger] (@($Value.code_units).Count)) `
                    $Path $problems
            }
        }
        'string' {
            if ([string] $Value.class -cne 'string' -or
                    ($Value.complex -is [bool] -and $Value.complex)) {
                $problems.Add("$Path metadata is invalid for string")
            }
            $strings = @(Get-OpenMatV2Sequence $Value 'string_code_units')
            $missing = @(Get-OpenMatV2Sequence $Value 'missing')
            for ($index = 0; $index -lt $strings.Count; $index++) {
                if ($strings[$index] -isnot [array]) {
                    $problems.Add("$Path.string_code_units element $index is not an array")
                    continue
                }
                $codeUnits = @($strings[$index])
                if ($codeUnits.Count -gt 16384) {
                    $problems.Add("$Path.string_code_units element $index exceeds 16384 code units")
                }
                foreach ($codeUnit in $codeUnits) {
                    if (-not (Test-OpenMatJsonInteger $codeUnit)) {
                        $problems.Add("$Path.string_code_units element $index contains a non-integer code unit")
                        break
                    }
                    $codeUnitValue = ConvertTo-OpenMatBigInteger $codeUnit
                    if ($codeUnitValue -lt 0 -or $codeUnitValue -gt 65535) {
                        $problems.Add("$Path.string_code_units element $index contains an out-of-range code unit")
                        break
                    }
                }
                if ($index -lt $missing.Count) {
                    if ($missing[$index] -isnot [bool]) {
                        $problems.Add("$Path.missing element $index is not boolean")
                    } elseif ($missing[$index] -and $codeUnits.Count -ne 0) {
                        $problems.Add("$Path.missing element $index has non-empty code units")
                    }
                }
                if ($index -ge $missing.Count -or
                        $missing[$index] -isnot [bool] -or
                        -not $missing[$index]) {
                    Add-OpenMatV2TraversalCharge `
                        $State CodeUnits `
                        ([System.Numerics.BigInteger] $codeUnits.Count) `
                        "$Path.string_code_units[$index]" $problems
                }
            }
        }
        'cell' {
            if ([string] $Value.class -cne 'cell' -or
                    ($Value.complex -is [bool] -and $Value.complex)) {
                $problems.Add("$Path metadata is invalid for cell")
            }
            $items = @(Get-OpenMatV2Sequence $Value 'items')
            for ($index = 0; $index -lt $items.Count; $index++) {
                foreach ($problem in @(Test-OpenMatV2Value `
                            $items[$index] "$Path.items[$index]" `
                            -AllowTolerance:$AllowTolerance `
                            -State $State -Depth ($Depth + 1))) {
                    if ($null -ne $problem) { $problems.Add($problem) }
                }
            }
        }
        'struct' {
            if ([string] $Value.class -cne 'struct' -or
                    ($Value.complex -is [bool] -and $Value.complex)) {
                $problems.Add("$Path metadata is invalid for struct")
            }
            $hasFields = Test-OpenMatProperty $Value 'fields'
            $hasRecords = Test-OpenMatProperty $Value 'records'
            if ($hasFields -ne $hasRecords) {
                $problems.Add("$Path.fields and $Path.records must appear together")
                break
            }
            if (-not $hasFields) { break }
            $fields = @(Get-OpenMatV2Sequence $Value 'fields')
            $fieldSet = [System.Collections.Generic.HashSet[string]]::new(
                [StringComparer]::Ordinal
            )
            if ($Value.fields -isnot [array]) {
                $problems.Add("$Path.fields is not an array")
            }
            foreach ($field in $fields) {
                if ($field -isnot [string] -or
                        ([string] $field) -cnotmatch '^[A-Za-z][A-Za-z0-9_]*$') {
                    $problems.Add("$Path.fields contains an invalid field name")
                } else {
                    Add-OpenMatV2TraversalCharge `
                        $State CodeUnits `
                        ([System.Numerics.BigInteger] ([string] $field).Length) `
                        "$Path.fields" $problems
                }
                if ($field -is [string] -and
                        -not $fieldSet.Add([string] $field)) {
                    $problems.Add("$Path.fields contains a duplicate field name")
                }
            }
            $records = @(Get-OpenMatV2Sequence $Value 'records')
            for ($recordIndex = 0; $recordIndex -lt $records.Count; $recordIndex++) {
                $record = $records[$recordIndex]
                if ($null -eq $record -or
                        $record -isnot [System.Management.Automation.PSCustomObject]) {
                    $problems.Add("$Path.records[$recordIndex] is not an object")
                    continue
                }
                $recordNames = @(
                    $record.PSObject.Properties |
                        ForEach-Object { [string] $_.Name }
                )
                if ($recordNames.Count -ne $fields.Count -or
                        @($recordNames | Where-Object { $_ -cnotin $fields }).Count -gt 0) {
                    $problems.Add("$Path.records[$recordIndex] fields differ from $Path.fields")
                }
                foreach ($field in $fields) {
                    if (Test-OpenMatProperty $record $field) {
                        foreach ($problem in @(Test-OpenMatV2Value `
                                    $record.$field `
                                    "$Path.records[$recordIndex].$field" `
                                    -AllowTolerance:$AllowTolerance `
                                    -State $State -Depth ($Depth + 1))) {
                            if ($null -ne $problem) { $problems.Add($problem) }
                        }
                    }
                }
            }
        }
        'table' {
            if ([string] $Value.class -cne 'table' -or
                    ($Value.complex -is [bool] -and $Value.complex)) {
                $problems.Add("$Path metadata is invalid for table")
            }
            if ($size.Count -ne 2) {
                $problems.Add("$Path.size must contain exactly two dimensions for table")
            }

            $hasVariableNames = Test-OpenMatProperty $Value 'variableNames'
            $hasVariables = Test-OpenMatProperty $Value 'variables'
            $variableNames = @()
            $variables = @()
            if ($hasVariableNames) {
                if ($Value.variableNames -isnot [array]) {
                    $problems.Add("$Path.variableNames is not an array")
                } else {
                    $variableNames = @(Get-OpenMatV2Sequence $Value 'variableNames')
                }
            }
            if ($hasVariables) {
                if ($Value.variables -isnot [array]) {
                    $problems.Add("$Path.variables is not an array")
                } else {
                    $variables = @(Get-OpenMatV2Sequence $Value 'variables')
                }
            }

            $tableShapeValid = $sizeValid -and $size.Count -eq 2
            if ($tableShapeValid) {
                $height = ConvertTo-OpenMatBigInteger $size[0]
                $width = ConvertTo-OpenMatBigInteger $size[1]
                if ([System.Numerics.BigInteger] $variableNames.Count -ne $width) {
                    $problems.Add(
                        "$Path.variableNames count differs from $Path.size[1]"
                    )
                }
                if ([System.Numerics.BigInteger] $variables.Count -ne $width) {
                    $problems.Add(
                        "$Path.variables count differs from $Path.size[1]"
                    )
                }
            }

            $nameSet = [System.Collections.Generic.HashSet[string]]::new(
                [StringComparer]::Ordinal
            )
            for ($index = 0; $index -lt $variableNames.Count; $index++) {
                $variableName = $variableNames[$index]
                if ($variableName -isnot [string] -or
                        [string]::IsNullOrEmpty([string] $variableName)) {
                    $problems.Add(
                        "$Path.variableNames[$index] is not a nonempty string"
                    )
                    continue
                }
                Add-OpenMatV2TraversalCharge `
                    $State CodeUnits `
                    ([System.Numerics.BigInteger] ([string] $variableName).Length) `
                    "$Path.variableNames[$index]" $problems
                if (-not $nameSet.Add([string] $variableName)) {
                    $problems.Add("$Path.variableNames contains a duplicate name")
                }
            }

            for ($index = 0; $index -lt $variables.Count; $index++) {
                $variable = $variables[$index]
                if ($tableShapeValid -and $null -ne $variable -and
                        (Test-OpenMatProperty $variable 'size') -and
                        $variable.size -is [array] -and
                        @($variable.size).Count -ge 1 -and
                        (Test-OpenMatJsonInteger (@($variable.size)[0]))) {
                    $variableRows = ConvertTo-OpenMatBigInteger `
                        (@($variable.size)[0])
                    if ($variableRows -ne $height) {
                        $problems.Add(
                            "$Path.variables[$index].size[0] differs from $Path.size[0]"
                        )
                    }
                }
                foreach ($problem in @(Test-OpenMatV2Value `
                            $variable "$Path.variables[$index]" `
                            -AllowTolerance:$AllowTolerance `
                            -State $State -Depth ($Depth + 1))) {
                    if ($null -ne $problem) { $problems.Add($problem) }
                }
            }
        }
    }
    return $problems.ToArray()
}

function Compare-OpenMatValueV2 {
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected,
        [AllowNull()] [object] $Tolerance,
        [string] $Path = 'value',
        [switch] $SkipValidation
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    if (-not $SkipValidation) {
        foreach ($problem in @(Test-OpenMatV2Value $Actual $Path)) {
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        foreach ($problem in @(Test-OpenMatV2Value `
                    $Expected "expected.$Path" -AllowTolerance)) {
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        if ($problems.Count -gt 0) {
            return $problems.ToArray()
        }
    }

    foreach ($name in @('class', 'ndims', 'numel', 'complex', 'kind')) {
        if ([string] $Actual.$name -cne [string] $Expected.$name) {
            $problems.Add("$Path.$name differs")
        }
    }
    $sizeProblem = Compare-OpenMatExactSequence $Actual.size $Expected.size "$Path.size"
    if ($null -ne $sizeProblem) { $problems.Add($sizeProblem) }

    $kind = [string] $Expected.kind
    if ([string] $Actual.kind -cne $kind) {
        return $problems.ToArray()
    }
    switch ($kind) {
        'numeric' {
            $numericTolerance = $Tolerance
            if ($null -eq $numericTolerance -and
                    (Test-OpenMatProperty $Expected 'tolerance')) {
                $numericTolerance = $Expected.tolerance
            }
            foreach ($component in @('real', 'imag')) {
                $problem = Compare-OpenMatNumericSequence `
                    $Actual.$component $Expected.$component `
                    $numericTolerance "$Path.$component"
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
        'integer' {
            $actualInteger = @($Actual.integer)
            $expectedInteger = @($Expected.integer)
            if ($actualInteger.Count -ne $expectedInteger.Count) {
                $problems.Add("$Path.integer count differs")
                break
            }
            for ($index = 0; $index -lt $expectedInteger.Count; $index++) {
                foreach ($component in @('real', 'imaginary')) {
                    if ([string] $actualInteger[$index].$component -cne
                            [string] $expectedInteger[$index].$component) {
                        $problems.Add(
                            "$Path.integer element $index $component differs"
                        )
                    }
                }
            }
        }
        'logical' {
            $problem = Compare-OpenMatExactSequence `
                $Actual.logical $Expected.logical "$Path.logical"
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'char' {
            $problem = Compare-OpenMatExactSequence `
                $Actual.code_units $Expected.code_units "$Path.code_units"
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        'string' {
            foreach ($payload in @('string_code_units', 'missing')) {
                $problem = Compare-OpenMatExactSequence `
                    $Actual.$payload $Expected.$payload "$Path.$payload"
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
        'cell' {
            $actualItems = @($Actual.items)
            $expectedItems = @($Expected.items)
            if ($actualItems.Count -ne $expectedItems.Count) {
                $problems.Add("$Path.items count differs")
                break
            }
            for ($index = 0; $index -lt $expectedItems.Count; $index++) {
                foreach ($problem in @(Compare-OpenMatValueV2 `
                            $actualItems[$index] $expectedItems[$index] $null `
                            "$Path.items[$index]" -SkipValidation)) {
                    if ($null -ne $problem) { $problems.Add($problem) }
                }
            }
        }
        'struct' {
            $actualHasRecords = Test-OpenMatProperty $Actual 'records'
            $expectedHasRecords = Test-OpenMatProperty $Expected 'records'
            if ($actualHasRecords -ne $expectedHasRecords) {
                $problems.Add("$Path.records presence differs")
                break
            }
            if ($expectedHasRecords) {
                $fieldProblem = Compare-OpenMatExactSequence `
                    $Actual.fields $Expected.fields "$Path.fields"
                if ($null -ne $fieldProblem) { $problems.Add($fieldProblem) }
                $actualRecords = @($Actual.records)
                $expectedRecords = @($Expected.records)
                if ($actualRecords.Count -ne $expectedRecords.Count) {
                    $problems.Add("$Path.records count differs")
                    break
                }
                for ($recordIndex = 0; $recordIndex -lt $expectedRecords.Count; $recordIndex++) {
                    foreach ($field in @($Expected.fields)) {
                        $actualRecord = $actualRecords[$recordIndex]
                        $expectedRecord = $expectedRecords[$recordIndex]
                        if (-not (Test-OpenMatProperty $actualRecord $field)) {
                            $problems.Add(
                                "$Path.records[$recordIndex].$field is missing"
                            )
                        } elseif (-not (Test-OpenMatProperty $expectedRecord $field)) {
                            $problems.Add(
                                "expected.$Path.records[$recordIndex].$field is missing"
                            )
                        } else {
                            foreach ($problem in @(Compare-OpenMatValueV2 `
                                        $actualRecord.$field `
                                        $expectedRecord.$field $null `
                                        "$Path.records[$recordIndex].$field" `
                                        -SkipValidation)) {
                                if ($null -ne $problem) { $problems.Add($problem) }
                            }
                        }
                    }
                }
            }
        }
        'table' {
            $nameProblem = Compare-OpenMatExactSequence `
                $Actual.variableNames $Expected.variableNames `
                "$Path.variableNames"
            if ($null -ne $nameProblem) { $problems.Add($nameProblem) }

            $actualVariables = @($Actual.variables)
            $expectedVariables = @($Expected.variables)
            if ($actualVariables.Count -ne $expectedVariables.Count) {
                $problems.Add("$Path.variables count differs")
                break
            }
            for ($index = 0; $index -lt $expectedVariables.Count; $index++) {
                foreach ($problem in @(Compare-OpenMatValueV2 `
                            $actualVariables[$index] $expectedVariables[$index] `
                            $null "$Path.variables[$index]" -SkipValidation)) {
                    if ($null -ne $problem) { $problems.Add($problem) }
                }
            }
        }
    }
    return $problems.ToArray()
}

function Compare-OpenMatObservationV2 {
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected,
        [Parameter(Mandatory)] [string] $CaseId,
        [AllowNull()] [object] $Tolerance
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($required in @('schema_version', 'case_id', 'oracle', 'outcome')) {
        if (-not (Test-OpenMatProperty $Actual $required)) {
            $problems.Add("$required is missing")
        }
    }
    if ($problems.Count -gt 0) { return $problems.ToArray() }
    if ([string] $Actual.schema_version -cne '2') {
        $problems.Add('schema_version is not 2')
        return $problems.ToArray()
    }
    if (Test-OpenMatProperty $Expected 'schema_version') {
        if ([string] $Expected.schema_version -cne '2') {
            $problems.Add('expected schema_version is not 2')
            return $problems.ToArray()
        }
    }
    if ([string] $Actual.case_id -cne $CaseId) {
        $problems.Add('case_id differs')
    }
    if ((Test-OpenMatProperty $Expected 'case_id') -and
            [string] $Expected.case_id -cne $CaseId) {
        $problems.Add('expected case_id differs')
    }
    if ($null -eq $Actual.oracle -or
            -not (Test-OpenMatProperty $Actual.oracle 'name') -or
            -not (Test-OpenMatProperty $Actual.oracle 'release')) {
        $problems.Add('oracle identity is incomplete')
    }
    if (-not (Test-OpenMatProperty $Expected 'outcome')) {
        $problems.Add('expected outcome is missing')
        return $problems.ToArray()
    }
    if ([string] $Actual.outcome -cnotin @('ok', 'error')) {
        $problems.Add('outcome is unknown')
        return $problems.ToArray()
    }
    if ([string] $Expected.outcome -cnotin @('ok', 'error')) {
        $problems.Add('expected outcome is unknown')
        return $problems.ToArray()
    }
    if ([string] $Actual.outcome -cne [string] $Expected.outcome) {
        $problems.Add('outcome differs')
        return $problems.ToArray()
    }

    if ([string] $Expected.outcome -ceq 'ok') {
        if (-not (Test-OpenMatProperty $Actual 'value') -or
                $null -eq $Actual.value -or
                -not (Test-OpenMatProperty $Expected 'value') -or
                $null -eq $Expected.value) {
            $problems.Add('value is missing')
            return $problems.ToArray()
        }
        $valueProblems = @(Compare-OpenMatValueV2 `
            $Actual.value $Expected.value $Tolerance)
        foreach ($problem in $valueProblems) {
            if ($null -ne $problem) { $problems.Add($problem) }
        }
        if ($valueProblems.Count -eq 0) {
            foreach ($problem in @(Test-OpenMatV2EncodedSize `
                        $Actual 'observation')) {
                if ($null -ne $problem) { $problems.Add($problem) }
            }
            foreach ($problem in @(Test-OpenMatV2EncodedSize `
                        $Expected 'expected observation')) {
                if ($null -ne $problem) { $problems.Add($problem) }
            }
        }
    } else {
        if (-not (Test-OpenMatProperty $Actual 'error') -or
                -not (Test-OpenMatProperty $Expected 'error')) {
            $problems.Add('error is missing')
        } elseif ([string] $Actual.error.category -cne
                [string] $Expected.error.category) {
            $problems.Add('error.category differs')
        }
    }
    return $problems.ToArray()
}

function Compare-OpenMatObservation {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object] $Actual,
        [Parameter(Mandatory)] [object] $Expected,
        [Parameter(Mandatory)] [string] $CaseId,
        [AllowNull()] [object] $Tolerance,
        [int] $SchemaVersion = 1
    )

    switch ($SchemaVersion) {
        1 { return @(Compare-OpenMatObservationV1 $Actual $Expected $CaseId $Tolerance) }
        2 { return @(Compare-OpenMatObservationV2 $Actual $Expected $CaseId $Tolerance) }
        default { return @("unsupported comparator schema_version $SchemaVersion") }
    }
}

function Test-OpenMatExpected {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object] $Expected,
        [int] $SchemaVersion = 1
    )

    if ($SchemaVersion -eq 1) { return @() }
    if ($SchemaVersion -ne 2) {
        return @("unsupported expected schema_version $SchemaVersion")
    }
    if (-not (Test-OpenMatProperty $Expected 'outcome')) {
        return @('expected outcome is missing')
    }
    if ([string] $Expected.outcome -ceq 'error') { return @() }
    if ([string] $Expected.outcome -cne 'ok') {
        return @('expected outcome is unknown')
    }
    if (-not (Test-OpenMatProperty $Expected 'value') -or
            $null -eq $Expected.value) {
        return @('expected value is missing')
    }
    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($problem in @(Test-OpenMatV2Value `
                $Expected.value 'expected.value' -AllowTolerance)) {
        if ($null -ne $problem) { $problems.Add($problem) }
    }
    if ($problems.Count -eq 0) {
        foreach ($problem in @(Test-OpenMatV2EncodedSize $Expected 'expected')) {
            if ($null -ne $problem) { $problems.Add($problem) }
        }
    }
    return $problems.ToArray()
}

Export-ModuleMember -Function Compare-OpenMatObservation, Test-OpenMatExpected
