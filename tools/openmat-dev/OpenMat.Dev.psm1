Set-StrictMode -Version Latest

$script:OpenMatBootstrapProtocol = 'openmat-kernel-v0'
$script:OpenMatPreferredProtocol = 'openmat-kernel-v1'
$script:OpenMatFallbackProtocol = 'openmat-kernel-v0'
$script:OpenMatMaxTextBytes = 1MB
$script:OpenMatUtf8 = [System.Text.UTF8Encoding]::new($false, $true)
$script:OpenMatProcessUtf8 = [System.Text.UTF8Encoding]::new($false, $false)
$script:OpenMatCapturedLineLimit = 256

function New-OpenMatFailure {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $Message,

        [Parameter(Mandatory)]
        [int] $ExitCode
    )

    $exception = [System.InvalidOperationException]::new($Message)
    $exception.Data['OpenMatExitCode'] = $ExitCode
    return $exception
}

function Throw-OpenMatFailure {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $Message,

        [Parameter(Mandatory)]
        [int] $ExitCode
    )

    throw (New-OpenMatFailure -Message $Message -ExitCode $ExitCode)
}

function Get-OpenMatFailureExitCode {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [System.Management.Automation.ErrorRecord] $ErrorRecord,

        [int] $Default = 1
    )

    $exception = $ErrorRecord.Exception
    while ($null -ne $exception) {
        if ($exception.Data.Contains('OpenMatExitCode')) {
            return [int] $exception.Data['OpenMatExitCode']
        }
        $exception = $exception.InnerException
    }
    return $Default
}

function Resolve-OpenMatRepositoryRoot {
    [CmdletBinding()]
    param(
        [string] $Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        $Path = [System.IO.Path]::Combine($PSScriptRoot, '..', '..')
    }

    try {
        $fullPath = [System.IO.Path]::GetFullPath($Path)
    }
    catch {
        Throw-OpenMatFailure -Message "Invalid repository path '$Path': $($_.Exception.Message)" -ExitCode 2
    }

    if (-not [System.IO.Directory]::Exists($fullPath)) {
        Throw-OpenMatFailure -Message "Repository directory does not exist: $fullPath" -ExitCode 2
    }

    $markers = @(
        'Cargo.toml',
        'crates/openmat-server/Cargo.toml',
        'apps/web/package.json'
    )
    foreach ($marker in $markers) {
        $markerPath = [System.IO.Path]::Combine($fullPath, $marker)
        if (-not [System.IO.File]::Exists($markerPath)) {
            Throw-OpenMatFailure -Message "Repository path is missing required marker '$marker': $fullPath" -ExitCode 2
        }
    }

    return $fullPath
}

function Resolve-OpenMatCommand {
    [CmdletBinding()]
    param(
        [string] $Path,

        [Parameter(Mandatory)]
        [string] $Name
    )

    if (-not [string]::IsNullOrWhiteSpace($Path)) {
        try {
            $fullPath = [System.IO.Path]::GetFullPath($Path)
        }
        catch {
            Throw-OpenMatFailure -Message "Invalid $Name path '$Path': $($_.Exception.Message)" -ExitCode 2
        }
        if (-not [System.IO.File]::Exists($fullPath)) {
            Throw-OpenMatFailure -Message "$Name executable does not exist: $fullPath" -ExitCode 2
        }
        return $fullPath
    }

    $candidateNames = if ($Name -eq 'pnpm') {
        @('pnpm.cmd', 'pnpm.exe')
    }
    elseif ($Name -eq 'cargo') {
        @('cargo.exe', 'cargo')
    }
    else {
        @($Name)
    }

    foreach ($candidateName in $candidateNames) {
        $command = Get-Command -Name $candidateName -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -ne $command -and [System.IO.File]::Exists($command.Source)) {
            return [System.IO.Path]::GetFullPath($command.Source)
        }
    }

    Throw-OpenMatFailure -Message "$Name was not found as a native executable on PATH" -ExitCode 3
}

function Resolve-OpenMatServerPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        Throw-OpenMatFailure -Message 'The server path must not be empty' -ExitCode 2
    }
    try {
        $fullPath = [System.IO.Path]::GetFullPath($Path)
    }
    catch {
        Throw-OpenMatFailure -Message "Invalid server path '$Path': $($_.Exception.Message)" -ExitCode 2
    }
    if (-not [System.IO.File]::Exists($fullPath)) {
        Throw-OpenMatFailure -Message "Server executable does not exist: $fullPath" -ExitCode 2
    }
    return $fullPath
}

function New-OpenMatLogDirectory {
    [CmdletBinding()]
    param(
        [string] $Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        $stamp = [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
        $suffix = [Guid]::NewGuid().ToString('N').Substring(0, 8)
        $Path = [System.IO.Path]::Combine(
            [System.IO.Path]::GetTempPath(),
            'openmat-dev',
            "$stamp-$PID-$suffix"
        )
    }

    try {
        $fullPath = [System.IO.Path]::GetFullPath($Path)
    }
    catch {
        Throw-OpenMatFailure -Message "Invalid log directory '$Path': $($_.Exception.Message)" -ExitCode 2
    }

    if ([System.IO.File]::Exists($fullPath)) {
        Throw-OpenMatFailure -Message "Log directory is an existing file: $fullPath" -ExitCode 2
    }
    try {
        [void] [System.IO.Directory]::CreateDirectory($fullPath)
    }
    catch {
        Throw-OpenMatFailure -Message "Could not create log directory '$fullPath': $($_.Exception.Message)" -ExitCode 2
    }
    return $fullPath
}

function New-OpenMatProcessStartInfo {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $FilePath,

        [string[]] $ArgumentList = @(),

        [Parameter(Mandatory)]
        [string] $WorkingDirectory,

        [System.Collections.IDictionary] $Environment
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.WorkingDirectory = $WorkingDirectory
    # Child tools can mix UTF-8 output with localized cmd.exe diagnostics.
    # Replacement fallback keeps log draining reliable; WebSocket text uses
    # the separate strict UTF-8 decoder above.
    $startInfo.StandardOutputEncoding = $script:OpenMatProcessUtf8
    $startInfo.StandardErrorEncoding = $script:OpenMatProcessUtf8

    $extension = [System.IO.Path]::GetExtension($FilePath)
    if ($extension -in @('.cmd', '.bat')) {
        if ([string]::IsNullOrWhiteSpace($env:ComSpec) -or -not [System.IO.File]::Exists($env:ComSpec)) {
            Throw-OpenMatFailure -Message 'ComSpec is unavailable; cannot start the pnpm command wrapper' -ExitCode 3
        }
        if ($FilePath.IndexOfAny([char[]] '"&|<>^%!') -ge 0) {
            Throw-OpenMatFailure -Message "Batch command path contains unsupported cmd.exe metacharacters: $FilePath" -ExitCode 2
        }
        foreach ($argument in $ArgumentList) {
            if ($argument -notmatch '^[A-Za-z0-9_.:/\\=-]+$') {
                Throw-OpenMatFailure -Message "Batch command argument contains unsupported characters: $argument" -ExitCode 2
            }
        }
        $argumentText = $ArgumentList -join ' '
        $startInfo.FileName = $env:ComSpec
        $startInfo.Arguments = "/d /s /c `"`"$FilePath`" $argumentText`""
    }
    else {
        $startInfo.FileName = $FilePath
        foreach ($argument in $ArgumentList) {
            [void] $startInfo.ArgumentList.Add($argument)
        }
    }

    if ($null -ne $Environment) {
        foreach ($entry in $Environment.GetEnumerator()) {
            $startInfo.Environment[[string] $entry.Key] = [string] $entry.Value
        }
    }
    return $startInfo
}

function Start-OpenMatLoggedProcess {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidatePattern('^[A-Za-z0-9._-]+$')]
        [string] $Name,

        [Parameter(Mandatory)]
        [string] $FilePath,

        [string[]] $ArgumentList = @(),

        [Parameter(Mandatory)]
        [string] $WorkingDirectory,

        [Parameter(Mandatory)]
        [string] $LogDirectory,

        [System.Collections.IDictionary] $Environment,

        [int] $LaunchFailureExitCode = 10
    )

    if (-not [System.IO.Directory]::Exists($WorkingDirectory)) {
        Throw-OpenMatFailure -Message "Process working directory does not exist: $WorkingDirectory" -ExitCode 2
    }
    if (-not [System.IO.File]::Exists($FilePath)) {
        Throw-OpenMatFailure -Message "Process executable does not exist: $FilePath" -ExitCode 2
    }

    [void] [System.IO.Directory]::CreateDirectory($LogDirectory)
    $stdoutPath = [System.IO.Path]::Combine($LogDirectory, "$Name.stdout.log")
    $stderrPath = [System.IO.Path]::Combine($LogDirectory, "$Name.stderr.log")
    [System.IO.File]::WriteAllText($stdoutPath, '', $script:OpenMatUtf8)
    [System.IO.File]::WriteAllText($stderrPath, '', $script:OpenMatUtf8)

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = New-OpenMatProcessStartInfo `
        -FilePath $FilePath `
        -ArgumentList $ArgumentList `
        -WorkingDirectory $WorkingDirectory `
        -Environment $Environment

    try {
        if (-not $process.Start()) {
            Throw-OpenMatFailure -Message "Failed to start $Name" -ExitCode $LaunchFailureExitCode
        }
    }
    catch {
        $process.Dispose()
        if ((Get-OpenMatFailureExitCode -ErrorRecord $_ -Default 0) -ne 0) {
            throw
        }
        Throw-OpenMatFailure -Message "Failed to start $Name from '$FilePath': $($_.Exception.Message)" -ExitCode $LaunchFailureExitCode
    }

    return [pscustomobject]@{
        Name          = $Name
        Process       = $process
        StdoutPath    = $stdoutPath
        StderrPath    = $stderrPath
        StdoutReader  = $process.StandardOutput
        StderrReader  = $process.StandardError
        StdoutTask    = $process.StandardOutput.ReadLineAsync()
        StderrTask    = $process.StandardError.ReadLineAsync()
        StdoutEnded   = $false
        StderrEnded   = $false
        StdoutLines   = [System.Collections.Generic.List[string]]::new()
        StderrLines   = [System.Collections.Generic.List[string]]::new()
        Disposed      = $false
    }
}

function Receive-OpenMatProcessLine {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record,

        [Parameter(Mandatory)]
        [ValidateSet('Stdout', 'Stderr')]
        [string] $Stream,

        [switch] $IgnoreReadErrors
    )

    $endedProperty = "${Stream}Ended"
    $taskProperty = "${Stream}Task"
    $readerProperty = "${Stream}Reader"
    $pathProperty = "${Stream}Path"
    $linesProperty = "${Stream}Lines"

    if ($Record.$endedProperty) {
        return $false
    }
    $task = $Record.$taskProperty
    if ($null -eq $task -or -not $task.IsCompleted) {
        return $false
    }

    try {
        $line = $task.GetAwaiter().GetResult()
    }
    catch {
        $Record.$endedProperty = $true
        $Record.$taskProperty = $null
        if (-not $IgnoreReadErrors) {
            throw
        }
        return $true
    }

    if ($null -eq $line) {
        $Record.$endedProperty = $true
        $Record.$taskProperty = $null
        return $true
    }

    [System.IO.File]::AppendAllText(
        $Record.$pathProperty,
        $line + [Environment]::NewLine,
        $script:OpenMatUtf8
    )
    $capturedLines = $Record.$linesProperty
    $capturedLines.Add($line)
    if ($capturedLines.Count -gt $script:OpenMatCapturedLineLimit) {
        $capturedLines.RemoveAt(0)
    }
    $Record.$taskProperty = $Record.$readerProperty.ReadLineAsync()
    return $true
}

function Update-OpenMatProcessLogs {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record,

        [switch] $IgnoreReadErrors
    )

    if ($Record.Disposed) {
        return
    }
    for ($iteration = 0; $iteration -lt 512; $iteration++) {
        $progress = $false
        if (Receive-OpenMatProcessLine -Record $Record -Stream Stdout -IgnoreReadErrors:$IgnoreReadErrors) {
            $progress = $true
        }
        if (Receive-OpenMatProcessLine -Record $Record -Stream Stderr -IgnoreReadErrors:$IgnoreReadErrors) {
            $progress = $true
        }
        if (-not $progress) {
            break
        }
    }
}

function Close-OpenMatProcessRecord {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record
    )

    if ($Record.Disposed) {
        return
    }

    $deadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        Update-OpenMatProcessLogs -Record $Record -IgnoreReadErrors
        if ($Record.StdoutEnded -and $Record.StderrEnded) {
            break
        }
        Start-Sleep -Milliseconds 20
    } while ([DateTime]::UtcNow -lt $deadline)

    $Record.StdoutReader.Dispose()
    $Record.StderrReader.Dispose()
    $Record.Process.Dispose()
    $Record.Disposed = $true
}

function Stop-OpenMatProcessTree {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record,

        [int] $WaitMilliseconds = 5000
    )

    if ($Record.Disposed) {
        return
    }

    try {
        if (-not $Record.Process.HasExited) {
            # Kill(Boolean) is deliberately scoped to this recorded process and
            # its descendants. No process-name enumeration is used.
            $Record.Process.Kill($true)
            [void] $Record.Process.WaitForExit($WaitMilliseconds)
        }
    }
    catch [System.InvalidOperationException] {
        # The recorded process exited between HasExited and Kill.
    }
    finally {
        Close-OpenMatProcessRecord -Record $Record
    }
}

function Wait-OpenMatProcessExit {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record,

        [Parameter(Mandatory)]
        [ValidateRange(1, 86400)]
        [int] $TimeoutSeconds,

        [int] $TimeoutExitCode = 11
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while (-not $Record.Process.HasExited) {
        Update-OpenMatProcessLogs -Record $Record
        if ([DateTime]::UtcNow -ge $deadline) {
            Throw-OpenMatFailure -Message "$($Record.Name) timed out after $TimeoutSeconds seconds" -ExitCode $TimeoutExitCode
        }
        Start-Sleep -Milliseconds 50
    }
    Update-OpenMatProcessLogs -Record $Record
    return $Record.Process.ExitCode
}

function Build-OpenMatServer {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $RepositoryRoot,

        [Parameter(Mandatory)]
        [string] $LogDirectory,

        [string] $CargoPath,

        [ValidateRange(1, 86400)]
        [int] $TimeoutSeconds = 600
    )

    $root = Resolve-OpenMatRepositoryRoot -Path $RepositoryRoot
    $cargo = Resolve-OpenMatCommand -Path $CargoPath -Name cargo
    $record = $null
    try {
        $record = Start-OpenMatLoggedProcess `
            -Name 'cargo-build' `
            -FilePath $cargo `
            -ArgumentList @('build', '--locked', '--package', 'openmat-server', '--bin', 'openmat-server') `
            -WorkingDirectory $root `
            -LogDirectory $LogDirectory `
            -LaunchFailureExitCode 10
        $exitCode = Wait-OpenMatProcessExit -Record $record -TimeoutSeconds $TimeoutSeconds -TimeoutExitCode 11
        if ($exitCode -ne 0) {
            Throw-OpenMatFailure -Message "cargo build failed with exit code $exitCode; see $($record.StderrPath)" -ExitCode 10
        }
    }
    finally {
        if ($null -ne $record) {
            if ($record.Process.HasExited) {
                Close-OpenMatProcessRecord -Record $record
            }
            else {
                Stop-OpenMatProcessTree -Record $record
            }
        }
    }

    $targetDirectory = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        [System.IO.Path]::Combine($root, 'target')
    }
    elseif ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    }
    else {
        [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($root, $env:CARGO_TARGET_DIR))
    }
    $serverPath = [System.IO.Path]::Combine($targetDirectory, 'debug', 'openmat-server.exe')
    return Resolve-OpenMatServerPath -Path $serverPath
}

function Build-OpenMatPlotWasm {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $RepositoryRoot,

        [Parameter(Mandatory)]
        [string] $LogDirectory,

        [string] $PnpmPath,

        [ValidateRange(1, 86400)]
        [int] $TimeoutSeconds = 600
    )

    $root = Resolve-OpenMatRepositoryRoot -Path $RepositoryRoot
    $pnpm = Resolve-OpenMatCommand -Path $PnpmPath -Name pnpm
    $webRoot = [System.IO.Path]::Combine($root, 'apps', 'web')
    $record = $null
    try {
        $record = Start-OpenMatLoggedProcess `
            -Name 'plot-wasm-build' `
            -FilePath $pnpm `
            -ArgumentList @('build:plot-wasm') `
            -WorkingDirectory $webRoot `
            -LogDirectory $LogDirectory `
            -LaunchFailureExitCode 12
        $exitCode = Wait-OpenMatProcessExit -Record $record -TimeoutSeconds $TimeoutSeconds -TimeoutExitCode 13
        if ($exitCode -ne 0) {
            Throw-OpenMatFailure -Message "Plot WASM build failed with exit code $exitCode; see $($record.StderrPath)" -ExitCode 12
        }
    }
    finally {
        if ($null -ne $record) {
            if ($record.Process.HasExited) {
                Close-OpenMatProcessRecord -Record $record
            }
            else {
                Stop-OpenMatProcessTree -Record $record
            }
        }
    }

    $javascriptPath = [System.IO.Path]::Combine($webRoot, 'public', 'wasm', 'openmat_plot_web.js')
    $wasmPath = [System.IO.Path]::Combine($webRoot, 'public', 'wasm', 'openmat_plot_web_bg.wasm')
    if (-not [System.IO.File]::Exists($javascriptPath) -or -not [System.IO.File]::Exists($wasmPath)) {
        Throw-OpenMatFailure -Message 'Plot WASM build did not produce the expected browser artifacts' -ExitCode 12
    }
}

function Assert-OpenMatServerUrl {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string] $Value
    )

    $uri = $null
    if (-not [System.Uri]::TryCreate($Value, [System.UriKind]::Absolute, [ref] $uri)) {
        Throw-OpenMatFailure -Message "Server announced an invalid WebSocket URL: '$Value'" -ExitCode 22
    }
    if ($uri.Scheme -ne 'ws' -or
        $uri.Host -ne '127.0.0.1' -or
        $uri.Port -le 0 -or
        $uri.AbsolutePath -ne '/kernel' -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment) -or
        -not [string]::IsNullOrEmpty($uri.UserInfo)) {
        Throw-OpenMatFailure -Message "Server URL must be ws://127.0.0.1:<port>/kernel, got '$Value'" -ExitCode 22
    }
    return $uri
}

function Get-OpenMatLspUrl {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [System.Uri] $ServerUrl
    )

    $validated = Assert-OpenMatServerUrl -Value $ServerUrl.AbsoluteUri
    $builder = [System.UriBuilder]::new($validated)
    $builder.Path = '/lsp'
    $builder.Query = ''
    $builder.Fragment = ''
    return $builder.Uri
}

function Start-OpenMatServer {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $ServerPath,

        [Parameter(Mandatory)]
        [string] $RepositoryRoot,

        [Parameter(Mandatory)]
        [string] $LogDirectory,

        [string] $WorkspaceRoot = $RepositoryRoot,

        [Parameter(Mandatory)]
        [ValidateRange(0, 65535)]
        [int] $Port
    )

    $resolvedServer = Resolve-OpenMatServerPath -Path $ServerPath
    try {
        $resolvedWorkspace = [System.IO.Path]::GetFullPath($WorkspaceRoot)
    }
    catch {
        Throw-OpenMatFailure -Message "Invalid workspace root '$WorkspaceRoot': $($_.Exception.Message)" -ExitCode 2
    }
    if (-not [System.IO.Directory]::Exists($resolvedWorkspace)) {
        Throw-OpenMatFailure -Message "Workspace root does not exist: $resolvedWorkspace" -ExitCode 2
    }
    return Start-OpenMatLoggedProcess `
        -Name 'server' `
        -FilePath $resolvedServer `
        -ArgumentList @('--listen', "127.0.0.1:$Port", '--workspace-root', $resolvedWorkspace) `
        -WorkingDirectory $RepositoryRoot `
        -LogDirectory $LogDirectory `
        -Environment @{ OPENMAT_PERF_LOG = '1' } `
        -LaunchFailureExitCode 20
}

function Wait-OpenMatServerUrl {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record,

        [ValidateRange(1, 3600)]
        [int] $TimeoutSeconds = 30
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ($true) {
        Update-OpenMatProcessLogs -Record $Record
        if ($Record.StdoutLines.Count -gt 0) {
            return Assert-OpenMatServerUrl -Value $Record.StdoutLines[0]
        }
        if ($Record.Process.HasExited) {
            Update-OpenMatProcessLogs -Record $Record -IgnoreReadErrors
            $details = if ($Record.StderrLines.Count -gt 0) { $Record.StderrLines[-1] } else { 'no stderr output' }
            Throw-OpenMatFailure `
                -Message "Server exited before announcing its URL (exit $($Record.Process.ExitCode)): $details" `
                -ExitCode 20
        }
        if ([DateTime]::UtcNow -ge $deadline) {
            Throw-OpenMatFailure -Message "Server did not announce its URL within $TimeoutSeconds seconds" -ExitCode 21
        }
        Start-Sleep -Milliseconds 20
    }
}

function Start-OpenMatFrontend {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $RepositoryRoot,

        [Parameter(Mandatory)]
        [string] $LogDirectory,

        [Parameter(Mandatory)]
        [System.Uri] $ServerUrl,

        [string] $PnpmPath
    )

    $validatedServerUrl = Assert-OpenMatServerUrl -Value $ServerUrl.AbsoluteUri
    $lspUrl = Get-OpenMatLspUrl -ServerUrl $validatedServerUrl
    $pnpm = Resolve-OpenMatCommand -Path $PnpmPath -Name pnpm
    $webRoot = [System.IO.Path]::Combine($RepositoryRoot, 'apps', 'web')
    return Start-OpenMatLoggedProcess `
        -Name 'frontend' `
        -FilePath $pnpm `
        -ArgumentList @('dev:ready') `
        -WorkingDirectory $webRoot `
        -LogDirectory $LogDirectory `
        -Environment @{
            VITE_OPENMAT_WS_URL  = $validatedServerUrl.AbsoluteUri
            VITE_OPENMAT_LSP_URL = $lspUrl.AbsoluteUri
        } `
        -LaunchFailureExitCode 40
}

function Wait-OpenMatFrontendReady {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject] $Record,

        [ValidateRange(1, 3600)]
        [int] $TimeoutSeconds = 30
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ($true) {
        Update-OpenMatProcessLogs -Record $Record
        foreach ($line in $Record.StdoutLines) {
            $plainLine = [regex]::Replace($line, '\x1B\[[0-?]*[ -/]*[@-~]', '')
            if ($plainLine -match 'Local:.*https?://') {
                return $plainLine
            }
        }
        if ($Record.Process.HasExited) {
            Update-OpenMatProcessLogs -Record $Record -IgnoreReadErrors
            $details = if ($Record.StderrLines.Count -gt 0) { $Record.StderrLines[-1] } else { 'no stderr output' }
            Throw-OpenMatFailure `
                -Message "Frontend exited before Vite became ready (exit $($Record.Process.ExitCode)): $details; see $($Record.StdoutPath) and $($Record.StderrPath)" `
                -ExitCode 40
        }
        if ([DateTime]::UtcNow -ge $deadline) {
            Throw-OpenMatFailure -Message "Frontend did not report a Vite local URL within $TimeoutSeconds seconds" -ExitCode 41
        }
        Start-Sleep -Milliseconds 20
    }
}

function Test-OpenMatMapKey {
    param(
        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Map,

        [Parameter(Mandatory)]
        [string] $Key
    )

    return $Map.Contains($Key)
}

function Get-OpenMatRequiredValue {
    param(
        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Map,

        [Parameter(Mandatory)]
        [string] $Key,

        [Parameter(Mandatory)]
        [string] $Context
    )

    if (-not (Test-OpenMatMapKey -Map $Map -Key $Key)) {
        Throw-OpenMatFailure -Message "$Context is missing '$Key'" -ExitCode 30
    }
    $value = $Map[$Key]
    if ($value -is [System.Collections.IList]) {
        return ,$value
    }
    return $value
}

function Assert-OpenMatNonEmptyString {
    param(
        [AllowNull()]
        [object] $Value,

        [Parameter(Mandatory)]
        [string] $Context
    )

    if ($Value -isnot [string] -or [string]::IsNullOrEmpty($Value)) {
        Throw-OpenMatFailure -Message "$Context must be a non-empty string" -ExitCode 30
    }
    return [string] $Value
}

function Assert-OpenMatMap {
    param(
        [AllowNull()]
        [object] $Value,

        [Parameter(Mandatory)]
        [string] $Context
    )

    if ($Value -isnot [System.Collections.IDictionary]) {
        Throw-OpenMatFailure -Message "$Context must be a JSON object" -ExitCode 30
    }
    return $Value
}

function Assert-OpenMatArray {
    param(
        [AllowEmptyCollection()]
        [AllowNull()]
        [object] $Value,

        [Parameter(Mandatory)]
        [string] $Context
    )

    if ($Value -isnot [System.Collections.IList] -or $Value -is [string]) {
        Throw-OpenMatFailure -Message "$Context must be a JSON array" -ExitCode 30
    }
    return ,$Value
}

function Test-OpenMatJsonNumber {
    param(
        [AllowNull()]
        [object] $Value
    )

    return $Value -is [byte] -or
        $Value -is [sbyte] -or
        $Value -is [int16] -or
        $Value -is [uint16] -or
        $Value -is [int32] -or
        $Value -is [uint32] -or
        $Value -is [int64] -or
        $Value -is [uint64] -or
        $Value -is [single] -or
        $Value -is [double] -or
        $Value -is [decimal]
}

function Assert-OpenMatVariableSummary {
    param(
        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Summary,

        [Parameter(Mandatory)]
        [string] $Context
    )

    [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $Summary -Key name -Context $Context) -Context "$Context.name")
    [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $Summary -Key class -Context $Context) -Context "$Context.class")
    $dimensions = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $Summary -Key dimensions -Context $Context) -Context "$Context.dimensions"
    foreach ($dimension in $dimensions) {
        if (-not (Test-OpenMatJsonNumber -Value $dimension) -or [double] $dimension -lt 0) {
            Throw-OpenMatFailure -Message "$Context.dimensions must contain non-negative numbers" -ExitCode 30
        }
    }
    $complex = Get-OpenMatRequiredValue -Map $Summary -Key complex -Context $Context
    if ($complex -isnot [bool]) {
        Throw-OpenMatFailure -Message "$Context.complex must be Boolean" -ExitCode 30
    }
}

function Assert-OpenMatKnownEvent {
    param(
        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Event,

        [Parameter(Mandatory)]
        [string] $Context
    )

    $type = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $Event -Key type -Context $Context) -Context "$Context.type"
    $data = Get-OpenMatRequiredValue -Map $Event -Key data -Context $Context
    switch ($type) {
        'status' {
            $map = Assert-OpenMatMap -Value $data -Context "$Context.data"
            $status = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $map -Key status -Context "$Context.data") -Context "$Context.data.status"
            if ($status -notin @('starting', 'idle', 'busy', 'interrupted', 'dead')) {
                Throw-OpenMatFailure -Message "$Context contains unknown status '$status'" -ExitCode 30
            }
        }
        'stream' {
            $map = Assert-OpenMatMap -Value $data -Context "$Context.data"
            $stream = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $map -Key stream -Context "$Context.data") -Context "$Context.data.stream"
            if ($stream -notin @('stdout', 'stderr')) {
                Throw-OpenMatFailure -Message "$Context contains invalid stream '$stream'" -ExitCode 30
            }
            $text = Get-OpenMatRequiredValue -Map $map -Key text -Context "$Context.data"
            if ($text -isnot [string]) {
                Throw-OpenMatFailure -Message "$Context.data.text must be a string" -ExitCode 30
            }
        }
        'display' {
            $map = Assert-OpenMatMap -Value $data -Context "$Context.data"
            $representations = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $map -Key representations -Context "$Context.data") -Context "$Context.data.representations"
            foreach ($entry in $representations.GetEnumerator()) {
                if ($entry.Key -isnot [string] -or $entry.Value -isnot [string]) {
                    Throw-OpenMatFailure -Message "$Context display representations must map strings to strings" -ExitCode 30
                }
            }
        }
        'diagnostic' {
            $map = Assert-OpenMatMap -Value $data -Context "$Context.data"
            [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $map -Key severity -Context "$Context.data") -Context "$Context.data.severity")
            [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $map -Key message -Context "$Context.data") -Context "$Context.data.message")
        }
        'workspaceDelta' {
            $map = Assert-OpenMatMap -Value $data -Context "$Context.data"
            foreach ($field in @('added', 'changed')) {
                $summaries = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $map -Key $field -Context "$Context.data") -Context "$Context.data.$field"
                for ($index = 0; $index -lt $summaries.Count; $index++) {
                    $summary = Assert-OpenMatMap -Value $summaries[$index] -Context "$Context.data.$field[$index]"
                    Assert-OpenMatVariableSummary -Summary $summary -Context "$Context.data.$field[$index]"
                }
            }
            $removed = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $map -Key removed -Context "$Context.data") -Context "$Context.data.removed"
            foreach ($name in $removed) {
                [void] (Assert-OpenMatNonEmptyString -Value $name -Context "$Context.data.removed[]")
            }
        }
        default {
            # Unknown event types are forward-compatible in negotiated kernel protocols.
        }
    }
    return $type
}

function Receive-OpenMatWebSocketItem {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken
    )

    $buffer = [byte[]]::new(16384)
    $stream = [System.IO.MemoryStream]::new()
    try {
        while ($true) {
            $segment = [System.ArraySegment[byte]]::new($buffer)
            $result = $Socket.ReceiveAsync(
                $segment,
                [System.Threading.CancellationToken] $CancellationToken
            ).GetAwaiter().GetResult()

            if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
                if ($stream.Length -ne 0) {
                    Throw-OpenMatFailure -Message 'WebSocket close interrupted a fragmented text message' -ExitCode 30
                }
                return [pscustomobject]@{
                    Kind        = 'close'
                    CloseStatus = $result.CloseStatus
                    Description = $result.CloseStatusDescription
                }
            }
            if ($result.MessageType -ne [System.Net.WebSockets.WebSocketMessageType]::Text) {
                Throw-OpenMatFailure -Message 'Kernel sent a non-text WebSocket message' -ExitCode 30
            }
            if ($stream.Length + $result.Count -gt $script:OpenMatMaxTextBytes) {
                Throw-OpenMatFailure -Message 'Kernel text message exceeded the 1 MiB limit' -ExitCode 30
            }
            if ($result.Count -gt 0) {
                $stream.Write($buffer, 0, $result.Count)
            }
            if ($result.EndOfMessage) {
                $text = $script:OpenMatUtf8.GetString($stream.ToArray())
                return [pscustomobject]@{
                    Kind = 'text'
                    Text = $text
                }
            }
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Send-OpenMatRequest {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Request,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken
    )

    $json = $Request | ConvertTo-Json -Depth 32 -Compress
    $bytes = $script:OpenMatUtf8.GetBytes($json)
    if ($bytes.Length -gt $script:OpenMatMaxTextBytes) {
        Throw-OpenMatFailure -Message 'Smoke request exceeded the 1 MiB limit' -ExitCode 30
    }
    $segment = [System.ArraySegment[byte]]::new($bytes)
    [void] $Socket.SendAsync(
        $segment,
        [System.Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        [System.Threading.CancellationToken] $CancellationToken
    ).GetAwaiter().GetResult()
}

function New-OpenMatSmokeRequest {
    param(
        [Parameter(Mandatory)]
        [string] $SessionId,

        [Parameter(Mandatory)]
        [string] $MessageId,

        [Parameter(Mandatory)]
        [string] $Type,

        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Params,

        [string] $Protocol = $script:OpenMatFallbackProtocol
    )

    return [ordered]@{
        protocol  = $Protocol
        sessionId = $SessionId
        messageId = $MessageId
        kind       = 'request'
        request    = [ordered]@{
            type   = $Type
            params = $Params
        }
    }
}

function Receive-OpenMatValidatedMessage {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken,

        [Parameter(Mandatory)]
        [string] $SessionId,

        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[string]] $ReceivedMessageIds,

        [string] $ExpectedProtocol = $script:OpenMatFallbackProtocol
    )

    $item = Receive-OpenMatWebSocketItem -Socket $Socket -CancellationToken $CancellationToken
    if ($item.Kind -eq 'close') {
        return $item
    }

    try {
        $message = $item.Text | ConvertFrom-Json -AsHashtable -Depth 64
    }
    catch {
        Throw-OpenMatFailure -Message "Kernel sent invalid JSON: $($_.Exception.Message)" -ExitCode 30
    }
    $message = Assert-OpenMatMap -Value $message -Context 'server message'
    $protocol = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $message -Key protocol -Context 'server message') -Context 'server message.protocol'
    if ($protocol -ne $ExpectedProtocol) {
        Throw-OpenMatFailure -Message "Kernel used unexpected protocol '$protocol'" -ExitCode 30
    }
    $actualSession = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $message -Key sessionId -Context 'server message') -Context 'server message.sessionId'
    if ($actualSession -ne $SessionId) {
        Throw-OpenMatFailure -Message "Kernel used unexpected sessionId '$actualSession'" -ExitCode 30
    }
    $messageId = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $message -Key messageId -Context 'server message') -Context 'server message.messageId'
    if (-not $ReceivedMessageIds.Add($messageId)) {
        Throw-OpenMatFailure -Message "Kernel reused messageId '$messageId'" -ExitCode 30
    }
    $kind = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $message -Key kind -Context 'server message') -Context 'server message.kind'
    if ($kind -notin @('event', 'response')) {
        Throw-OpenMatFailure -Message "Kernel sent unexpected message kind '$kind'" -ExitCode 30
    }

    if ($kind -eq 'event') {
        $event = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $message -Key event -Context 'server event') -Context 'server event.event'
        [void] (Assert-OpenMatKnownEvent -Event $event -Context "event $messageId")
    }
    else {
        [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $message -Key replyTo -Context 'server response') -Context 'server response.replyTo')
        $ok = Get-OpenMatRequiredValue -Map $message -Key ok -Context 'server response'
        if ($ok -isnot [bool]) {
            Throw-OpenMatFailure -Message 'server response.ok must be Boolean' -ExitCode 30
        }
        if ($ok) {
            if (-not (Test-OpenMatMapKey -Map $message -Key result) -or (Test-OpenMatMapKey -Map $message -Key error)) {
                Throw-OpenMatFailure -Message 'Successful response must contain result and no error' -ExitCode 30
            }
        }
        elseif (-not (Test-OpenMatMapKey -Map $message -Key error) -or (Test-OpenMatMapKey -Map $message -Key result)) {
            Throw-OpenMatFailure -Message 'Failed response must contain error and no result' -ExitCode 30
        }
    }
    return [pscustomobject]@{
        Kind    = 'message'
        Message = $message
    }
}

function Invoke-OpenMatSmokeExchange {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken,

        [Parameter(Mandatory)]
        [string] $SessionId,

        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[string]] $ReceivedMessageIds,

        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Request,

        [Parameter(Mandatory)]
        [string] $ExpectedResultType,

        [string[]] $RequiredStatuses = @(),

        [switch] $AlreadySent,

        [string] $ExpectedProtocol = $script:OpenMatFallbackProtocol
    )

    if (-not $AlreadySent) {
        Send-OpenMatRequest -Socket $Socket -Request $Request -CancellationToken $CancellationToken
    }
    $requestId = [string] $Request.messageId
    $response = $null
    $events = [System.Collections.Generic.List[System.Collections.IDictionary]]::new()
    $statuses = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $messageCount = 0

    while ($true) {
        $messageCount++
        if ($messageCount -gt 4096) {
            Throw-OpenMatFailure -Message "Too many messages while waiting for '$requestId'" -ExitCode 30
        }
        $item = Receive-OpenMatValidatedMessage `
            -Socket $Socket `
            -CancellationToken $CancellationToken `
            -SessionId $SessionId `
            -ReceivedMessageIds $ReceivedMessageIds `
            -ExpectedProtocol $ExpectedProtocol
        if ($item.Kind -eq 'close') {
            Throw-OpenMatFailure -Message "WebSocket closed before response to '$requestId'" -ExitCode 30
        }
        $message = $item.Message
        if ($message.kind -eq 'event') {
            $events.Add($message.event)
            if ($message.event.type -eq 'status') {
                [void] $statuses.Add([string] $message.event.data.status)
            }
        }
        else {
            if ($message.replyTo -ne $requestId) {
                Throw-OpenMatFailure -Message "Response replyTo '$($message.replyTo)' does not match '$requestId'" -ExitCode 30
            }
            if ($null -ne $response) {
                Throw-OpenMatFailure -Message "Kernel sent multiple responses to '$requestId'" -ExitCode 30
            }
            if (-not $message.ok) {
                $error = Assert-OpenMatMap -Value $message.error -Context 'server response.error'
                $category = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $error -Key category -Context 'server response.error') -Context 'server response.error.category'
                $description = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $error -Key message -Context 'server response.error') -Context 'server response.error.message'
                Throw-OpenMatFailure -Message "Request '$requestId' failed: ${category}: $description" -ExitCode 30
            }
            $result = Assert-OpenMatMap -Value $message.result -Context 'server response.result'
            $resultType = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $result -Key type -Context 'server response.result') -Context 'server response.result.type'
            if ($resultType -ne $ExpectedResultType) {
                Throw-OpenMatFailure -Message "Response to '$requestId' has result '$resultType', expected '$ExpectedResultType'" -ExitCode 30
            }
            $response = $result
        }

        $hasAllStatuses = $true
        foreach ($requiredStatus in $RequiredStatuses) {
            if (-not $statuses.Contains($requiredStatus)) {
                $hasAllStatuses = $false
                break
            }
        }
        if ($null -ne $response -and $hasAllStatuses) {
            return [pscustomobject]@{
                Result = $response
                Events = $events
            }
        }
    }
}

function Assert-OpenMatScalarSummary {
    param(
        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Summary,

        [Parameter(Mandatory)]
        [string] $Name
    )

    Assert-OpenMatVariableSummary -Summary $Summary -Context "workspace variable '$Name'"
    if ($Summary.name -ne $Name -or $Summary.class -ne 'double') {
        Throw-OpenMatFailure -Message "Workspace variable '$Name' has unexpected name or class" -ExitCode 30
    }
    $dimensions = Assert-OpenMatArray -Value $Summary.dimensions -Context "workspace variable '$Name'.dimensions"
    if ($dimensions.Count -ne 2 -or [int64] $dimensions[0] -ne 1 -or [int64] $dimensions[1] -ne 1) {
        Throw-OpenMatFailure -Message "Workspace variable '$Name' is not a 1-by-1 scalar" -ExitCode 30
    }
}

function Invoke-OpenMatWorkspaceExchange {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken,

        [Parameter(Mandatory)]
        [string] $RequestId,

        [Parameter(Mandatory)]
        [string] $Type,

        [Parameter(Mandatory)]
        [System.Collections.IDictionary] $Params,

        [string] $ExpectedErrorCode
    )

    $request = [ordered]@{
        protocol  = 'openmat-workspace-v2'
        requestId = $RequestId
        request   = [ordered]@{ type = $Type; params = $Params }
    }
    Send-OpenMatRequest -Socket $Socket -Request $request -CancellationToken $CancellationToken
    $response = $null
    for ($frame = 0; $frame -lt 64; $frame += 1) {
        $item = Receive-OpenMatWebSocketItem -Socket $Socket -CancellationToken $CancellationToken
        if ($item.Kind -ne 'text') {
            Throw-OpenMatFailure -Message "Workspace WebSocket closed before '$RequestId' completed" -ExitCode 30
        }
        try {
            $candidate = $item.Text | ConvertFrom-Json -AsHashtable -Depth 32
        }
        catch {
            Throw-OpenMatFailure -Message "Workspace API sent invalid JSON: $($_.Exception.Message)" -ExitCode 30
        }
        $candidate = Assert-OpenMatMap -Value $candidate -Context 'workspace response'
        if ($candidate.protocol -ne 'openmat-workspace-v2') {
            Throw-OpenMatFailure -Message 'Workspace response used an unexpected protocol' -ExitCode 30
        }
        if ($candidate.Contains('event') -and -not $candidate.Contains('requestId')) {
            continue
        }
        $response = $candidate
        break
    }
    if ($null -eq $response) {
        Throw-OpenMatFailure -Message "Workspace request '$RequestId' did not receive a correlated response" -ExitCode 30
    }
    if ($response.protocol -ne 'openmat-workspace-v2' -or $response.requestId -ne $RequestId -or $response.ok -isnot [bool]) {
        Throw-OpenMatFailure -Message "Workspace response envelope did not match '$RequestId'" -ExitCode 30
    }
    if (-not $response.ok) {
        $error = Assert-OpenMatMap -Value $response.error -Context 'workspace response.error'
        $category = Assert-OpenMatNonEmptyString -Value $error.code -Context 'workspace response.error.code'
        $description = Assert-OpenMatNonEmptyString -Value $error.message -Context 'workspace response.error.message'
        if (-not [string]::IsNullOrEmpty($ExpectedErrorCode) -and $category -eq $ExpectedErrorCode) {
            return $error
        }
        Throw-OpenMatFailure -Message "Workspace request '$RequestId' failed: ${category}: $description" -ExitCode 30
    }
    if (-not [string]::IsNullOrEmpty($ExpectedErrorCode)) {
        Throw-OpenMatFailure -Message "Workspace request '$RequestId' succeeded, expected '$ExpectedErrorCode'" -ExitCode 30
    }
    $result = Assert-OpenMatMap -Value $response.result -Context 'workspace response.result'
    if ($result.type -ne $Type) {
        Throw-OpenMatFailure -Message "Workspace response type '$($result.type)' did not match '$Type'" -ExitCode 30
    }
    return Assert-OpenMatMap -Value $result.data -Context 'workspace response.result.data'
}

function Invoke-OpenMatWorkspaceSmokeLifecycle {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [System.Uri] $ServerUrl,

        [Parameter(Mandatory)]
        [string] $WorkspaceRoot,

        [ValidateRange(1, 3600)]
        [int] $TimeoutSeconds = 30
    )

    [void] (Assert-OpenMatServerUrl -Value $ServerUrl.AbsoluteUri)
    try {
        $root = [System.IO.Path]::GetFullPath($WorkspaceRoot)
    }
    catch {
        Throw-OpenMatFailure -Message "Invalid workspace smoke root '$WorkspaceRoot': $($_.Exception.Message)" -ExitCode 2
    }
    if (-not [System.IO.Directory]::Exists($root)) {
        Throw-OpenMatFailure -Message "Workspace smoke root does not exist: $root" -ExitCode 2
    }

    $builder = [System.UriBuilder]::new($ServerUrl)
    $builder.Path = '/workspace/v2'
    $builder.Query = ''
    $builder.Fragment = ''
    $workspaceUrl = $builder.Uri
    $suffix = [Guid]::NewGuid().ToString('N')
    $directoryName = "openmat-workspace-smoke-$suffix"
    $fileName = "$directoryName/live.m"
    $renamedName = "$directoryName/renamed.m"
    $movedName = "openmat-workspace-moved-$suffix.m"
    $filePath = [System.IO.Path]::Combine($root, $fileName)
    $movedPath = [System.IO.Path]::Combine($root, $movedName)
    $directoryPath = [System.IO.Path]::Combine($root, $directoryName)
    $socket = $null
    $timeout = [System.Threading.CancellationTokenSource]::new()
    $timeout.CancelAfter([TimeSpan]::FromSeconds($TimeoutSeconds))
    $token = $timeout.Token

    try {
        $socket = [System.Net.WebSockets.ClientWebSocket]::new()
        $socket.Options.KeepAliveInterval = [TimeSpan]::FromSeconds(5)
        [void] $socket.ConnectAsync($workspaceUrl, $token).GetAwaiter().GetResult()

        $initial = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0001-list' `
            -Type 'list' `
            -Params ([ordered]@{ path = ''; recursive = $false })
        [void] (Assert-OpenMatNonEmptyString -Value $initial.rootName -Context 'workspace list rootName')
        [void] (Assert-OpenMatArray -Value $initial.entries -Context 'workspace list entries')

        $createdDirectory = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0002-directory' `
            -Type 'create' `
            -Params ([ordered]@{ path = $directoryName; kind = 'directory' })
        if ($createdDirectory.entry.path -ne $directoryName -or $createdDirectory.entry.kind -ne 'directory') {
            Throw-OpenMatFailure -Message 'Workspace directory creation returned an unexpected entry' -ExitCode 30
        }

        $createdFile = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0003-file' `
            -Type 'create' `
            -Params ([ordered]@{ path = $fileName; kind = 'file' })
        if ($createdFile.entry.path -ne $fileName -or $createdFile.entry.kind -ne 'file') {
            Throw-OpenMatFailure -Message 'Workspace file creation returned an unexpected entry' -ExitCode 30
        }

        $opened = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0004-read' `
            -Type 'read' `
            -Params ([ordered]@{ path = $fileName })
        if ($opened.content -ne '' -or [string]::IsNullOrEmpty([string] $opened.revision)) {
            Throw-OpenMatFailure -Message 'Workspace read did not return empty UTF-8 content and a revision' -ExitCode 30
        }

        $saved = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0005-write' `
            -Type 'write' `
            -Params ([ordered]@{
                path = $fileName
                content = "smoke_value = 42;`n"
                expectedRevision = $opened.revision
            })
        if ($saved.entry.path -ne $fileName -or [string]::IsNullOrEmpty([string] $saved.entry.revision)) {
            Throw-OpenMatFailure -Message 'Workspace write did not return the saved file revision' -ExitCode 30
        }

        [System.IO.File]::WriteAllText($filePath, "external_value = 7;`n", $script:OpenMatUtf8)
        $conflict = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0006-conflict' `
            -Type 'write' `
            -Params ([ordered]@{
                path = $fileName
                content = "must_not_win = 1;`n"
                expectedRevision = $saved.entry.revision
            }) `
            -ExpectedErrorCode 'workspace.revisionConflict'
        if ($conflict.details.currentRevision -isnot [string] -or
            [System.IO.File]::ReadAllText($filePath, $script:OpenMatUtf8) -ne "external_value = 7;`n") {
            Throw-OpenMatFailure -Message 'Workspace revision conflict did not preserve the external content' -ExitCode 30
        }

        $renamed = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0007-rename' `
            -Type 'rename' `
            -Params ([ordered]@{ path = $fileName; newName = 'renamed.m' })
        if ($renamed.entry.path -ne $renamedName) {
            Throw-OpenMatFailure -Message 'Workspace rename returned an unexpected path' -ExitCode 30
        }

        $recursive = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0008-recursive-list' `
            -Type 'list' `
            -Params ([ordered]@{ path = $directoryName; recursive = $true })
        $recursiveEntries = Assert-OpenMatArray -Value $recursive.entries -Context 'recursive workspace entries'
        if ($renamedName -notin @($recursiveEntries | ForEach-Object { $_.path })) {
            Throw-OpenMatFailure -Message 'Workspace API did not recursively list the requested relative directory' -ExitCode 30
        }

        $moved = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0009-move' `
            -Type 'move' `
            -Params ([ordered]@{ path = $renamedName; targetPath = $movedName })
        if ($moved.entry.path -ne $movedName) {
            Throw-OpenMatFailure -Message 'Workspace move returned an unexpected path' -ExitCode 30
        }

        $refreshed = Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0010-list' `
            -Type 'list' `
            -Params ([ordered]@{ path = ''; recursive = $false })
        $entries = Assert-OpenMatArray -Value $refreshed.entries -Context 'refreshed workspace entries'
        $paths = @($entries | ForEach-Object { $_.path })
        if ($movedName -notin $paths -or $directoryName -notin $paths -or
            -not [System.IO.File]::Exists($movedPath) -or
            -not [System.IO.Directory]::Exists($directoryPath)) {
            Throw-OpenMatFailure -Message 'Workspace API did not relist moved root content' -ExitCode 30
        }

        [void] (Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0011-delete-file' `
            -Type 'delete' `
            -Params ([ordered]@{ path = $movedName; recursive = $false; confirmPath = $null }))
        [void] (Invoke-OpenMatWorkspaceExchange `
            -Socket $socket `
            -CancellationToken $token `
            -RequestId 'workspace-smoke-0012-delete-directory' `
            -Type 'delete' `
            -Params ([ordered]@{ path = $directoryName; recursive = $true; confirmPath = $directoryName }))
        if ([System.IO.File]::Exists($movedPath) -or [System.IO.Directory]::Exists($directoryPath)) {
            Throw-OpenMatFailure -Message 'Workspace delete operations did not remove their exact targets' -ExitCode 30
        }

        [void] $socket.CloseAsync(
            [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure,
            'workspace smoke complete',
            $token
        ).GetAwaiter().GetResult()
        return [pscustomobject]@{
            CreatedCount = 2
            FileName     = $fileName
            DirectoryName = $directoryName
        }
    }
    catch [System.OperationCanceledException] {
        Throw-OpenMatFailure -Message "Workspace smoke timed out after $TimeoutSeconds seconds" -ExitCode 31
    }
    catch {
        if ((Get-OpenMatFailureExitCode -ErrorRecord $_ -Default 0) -ne 0) {
            throw
        }
        Throw-OpenMatFailure -Message "Workspace smoke failed: $($_.Exception.Message)" -ExitCode 30
    }
    finally {
        if ($null -ne $socket) {
            if ($socket.State -notin @(
                    [System.Net.WebSockets.WebSocketState]::Closed,
                    [System.Net.WebSockets.WebSocketState]::Aborted,
                    [System.Net.WebSockets.WebSocketState]::None
                )) {
                $socket.Abort()
            }
            $socket.Dispose()
        }
        $timeout.Dispose()
        if ([System.IO.File]::Exists($filePath)) {
            [System.IO.File]::Delete($filePath)
        }
        if ([System.IO.File]::Exists($movedPath)) {
            [System.IO.File]::Delete($movedPath)
        }
        if ([System.IO.Directory]::Exists($directoryPath)) {
            [System.IO.Directory]::Delete($directoryPath, $true)
        }
    }
}

function Receive-OpenMatLspJsonRpc {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken
    )

    $item = Receive-OpenMatWebSocketItem -Socket $Socket -CancellationToken $CancellationToken
    if ($item.Kind -eq 'close') {
        return $item
    }
    try {
        $message = $item.Text | ConvertFrom-Json -AsHashtable -Depth 64
    }
    catch {
        Throw-OpenMatFailure -Message "LSP endpoint sent invalid JSON: $($_.Exception.Message)" -ExitCode 30
    }
    $message = Assert-OpenMatMap -Value $message -Context 'LSP JSON-RPC message'
    if ($message.jsonrpc -ne '2.0') {
        Throw-OpenMatFailure -Message 'LSP endpoint sent a non-2.0 JSON-RPC message' -ExitCode 30
    }
    return [pscustomobject]@{
        Kind    = 'message'
        Message = $message
    }
}

function Receive-OpenMatLspJsonRpcResponse {
    param(
        [Parameter(Mandatory)]
        [System.Net.WebSockets.ClientWebSocket] $Socket,

        [Parameter(Mandatory)]
        [System.Threading.CancellationToken] $CancellationToken,

        [Parameter(Mandatory)]
        [string] $ExpectedId
    )

    while ($true) {
        $item = Receive-OpenMatLspJsonRpc `
            -Socket $Socket `
            -CancellationToken $CancellationToken
        if ($item.Kind -eq 'close') {
            Throw-OpenMatFailure `
                -Message "LSP endpoint closed before responding to '$ExpectedId'" `
                -ExitCode 30
        }
        if (-not (Test-OpenMatMapKey -Map $item.Message -Key 'id')) {
            # LSP notifications may legally be interleaved with request responses.
            continue
        }
        $actualId = [string] (Get-OpenMatRequiredValue `
            -Map $item.Message `
            -Key 'id' `
            -Context 'LSP response')
        if ($actualId -ne $ExpectedId) {
            Throw-OpenMatFailure `
                -Message "LSP response id '$actualId' did not match '$ExpectedId'" `
                -ExitCode 30
        }
        return $item
    }
}

function Invoke-OpenMatLspSmokeLifecycle {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [System.Uri] $ServerUrl,

        [ValidateRange(1, 3600)]
        [int] $TimeoutSeconds = 30
    )

    $lspUrl = Get-OpenMatLspUrl -ServerUrl $ServerUrl
    $socket = $null
    $timeout = [System.Threading.CancellationTokenSource]::new()
    $timeout.CancelAfter([TimeSpan]::FromSeconds($TimeoutSeconds))
    $token = $timeout.Token
    $uri = 'file:///openmat-dev-lsp-smoke.m'
    try {
        $socket = [System.Net.WebSockets.ClientWebSocket]::new()
        $socket.Options.KeepAliveInterval = [TimeSpan]::FromSeconds(5)
        [void] $socket.ConnectAsync($lspUrl, $token).GetAwaiter().GetResult()

        $initialize = [ordered]@{
            jsonrpc = '2.0'
            id      = 'lsp-smoke-initialize'
            method  = 'initialize'
            params  = [ordered]@{
                processId = $null
                clientInfo = [ordered]@{ name = 'openmat-dev-lsp-smoke'; version = '1' }
                rootUri = $null
                capabilities = [ordered]@{}
                initializationOptions = [ordered]@{ transport = 'openmat-lsp-websocket-v1' }
            }
        }
        Send-OpenMatRequest -Socket $socket -Request $initialize -CancellationToken $token
        $initialized = Receive-OpenMatLspJsonRpcResponse `
            -Socket $socket `
            -CancellationToken $token `
            -ExpectedId ([string] $initialize['id'])
        if ($initialized.Kind -ne 'message' -or
            $initialized.Message.result.serverInfo.name -ne 'openmat-lsp' -or
            $initialized.Message.result.capabilities.completionProvider.resolveProvider -ne $true) {
            Throw-OpenMatFailure -Message 'LSP initialize response did not advertise openmat-lsp completion' -ExitCode 30
        }
        Send-OpenMatRequest `
            -Socket $socket `
            -CancellationToken $token `
            -Request ([ordered]@{ jsonrpc = '2.0'; method = 'initialized'; params = [ordered]@{} })

        $source = "function y = calculate(x)`ny = x;`nend`ncal"
        Send-OpenMatRequest `
            -Socket $socket `
            -CancellationToken $token `
            -Request ([ordered]@{
                jsonrpc = '2.0'
                method = 'textDocument/didOpen'
                params = [ordered]@{
                    textDocument = [ordered]@{
                        uri = $uri; languageId = 'openmat'; version = 1; text = $source
                    }
                }
            })
        $opened = Receive-OpenMatLspJsonRpc -Socket $socket -CancellationToken $token
        if ($opened.Kind -ne 'message' -or
            $opened.Message.method -ne 'textDocument/publishDiagnostics' -or
            $opened.Message.params.uri -ne $uri -or
            [int] $opened.Message.params.version -ne 1) {
            Throw-OpenMatFailure -Message 'LSP didOpen did not publish versioned diagnostics' -ExitCode 30
        }

        $completionRequest = [ordered]@{
            jsonrpc = '2.0'
            id = 'lsp-smoke-completion'
            method = 'textDocument/completion'
            params = [ordered]@{
                textDocument = [ordered]@{ uri = $uri }
                position = [ordered]@{ line = 3; character = 3 }
            }
        }
        Send-OpenMatRequest -Socket $socket -Request $completionRequest -CancellationToken $token
        $completion = Receive-OpenMatLspJsonRpcResponse `
            -Socket $socket `
            -CancellationToken $token `
            -ExpectedId ([string] $completionRequest['id'])
        if ($completion.Kind -ne 'message') {
            Throw-OpenMatFailure -Message 'LSP completion response was not correlated' -ExitCode 30
        }
        $items = Assert-OpenMatArray -Value $completion.Message.result -Context 'LSP completion result'
        $calculate = @($items | Where-Object { $_.label -eq 'calculate' })
        if ($calculate.Count -ne 1 -or
            $calculate[0].detail -ne 'function' -or
            $calculate[0].textEdit.newText -ne 'calculate') {
            Throw-OpenMatFailure -Message 'LSP completion did not contain the open-document calculate function' -ExitCode 30
        }

        Send-OpenMatRequest `
            -Socket $socket `
            -CancellationToken $token `
            -Request ([ordered]@{
                jsonrpc = '2.0'
                method = 'textDocument/didChange'
                params = [ordered]@{
                    textDocument = [ordered]@{ uri = $uri; version = 2 }
                    contentChanges = @([ordered]@{ text = [string] ([char]::ConvertFromUtf32(0x1F600)) })
                }
            })
        $invalid = Receive-OpenMatLspJsonRpc -Socket $socket -CancellationToken $token
        $invalidDiagnostics = Assert-OpenMatArray -Value $invalid.Message.params.diagnostics -Context 'changed LSP diagnostics'
        if ($invalid.Message.method -ne 'textDocument/publishDiagnostics' -or
            [int] $invalid.Message.params.version -ne 2 -or
            $invalidDiagnostics.Count -lt 1) {
            Throw-OpenMatFailure -Message 'LSP diagnostics did not update after didChange' -ExitCode 30
        }

        Send-OpenMatRequest `
            -Socket $socket `
            -CancellationToken $token `
            -Request ([ordered]@{
                jsonrpc = '2.0'
                method = 'textDocument/didChange'
                params = [ordered]@{
                    textDocument = [ordered]@{ uri = $uri; version = 3 }
                    contentChanges = @([ordered]@{ text = "value = 1;`n" })
                }
            })
        $valid = Receive-OpenMatLspJsonRpc -Socket $socket -CancellationToken $token
        $validDiagnostics = Assert-OpenMatArray -Value $valid.Message.params.diagnostics -Context 'repaired LSP diagnostics'
        if ([int] $valid.Message.params.version -ne 3 -or $validDiagnostics.Count -ne 0) {
            Throw-OpenMatFailure -Message 'LSP diagnostics did not clear after a valid didChange' -ExitCode 30
        }

        Send-OpenMatRequest `
            -Socket $socket `
            -CancellationToken $token `
            -Request ([ordered]@{
                jsonrpc = '2.0'
                method = 'textDocument/didClose'
                params = [ordered]@{ textDocument = [ordered]@{ uri = $uri } }
            })
        $closed = Receive-OpenMatLspJsonRpc -Socket $socket -CancellationToken $token
        $closedDiagnostics = Assert-OpenMatArray -Value $closed.Message.params.diagnostics -Context 'closed LSP diagnostics'
        if ($closed.Message.params.uri -ne $uri -or $closedDiagnostics.Count -ne 0) {
            Throw-OpenMatFailure -Message 'LSP didClose did not publish marker cleanup' -ExitCode 30
        }

        $shutdown = [ordered]@{
            jsonrpc = '2.0'; id = 'lsp-smoke-shutdown'; method = 'shutdown'; params = $null
        }
        Send-OpenMatRequest -Socket $socket -Request $shutdown -CancellationToken $token
        $shutDown = Receive-OpenMatLspJsonRpcResponse `
            -Socket $socket `
            -CancellationToken $token `
            -ExpectedId ([string] $shutdown['id'])
        if (-not (Test-OpenMatMapKey -Map $shutDown.Message -Key result) -or
            $null -ne $shutDown.Message.result) {
            Throw-OpenMatFailure -Message 'LSP shutdown response was not correlated null' -ExitCode 30
        }
        Send-OpenMatRequest `
            -Socket $socket `
            -CancellationToken $token `
            -Request ([ordered]@{ jsonrpc = '2.0'; method = 'exit'; params = $null })
        $close = Receive-OpenMatLspJsonRpc -Socket $socket -CancellationToken $token
        if ($close.Kind -ne 'close' -or
            $close.CloseStatus -ne [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure) {
            Throw-OpenMatFailure -Message 'LSP endpoint did not close normally after shutdown/exit' -ExitCode 30
        }
        [void] $socket.CloseOutputAsync(
            [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure,
            'LSP smoke complete',
            $token
        ).GetAwaiter().GetResult()
        return [pscustomobject]@{
            CompletionLabel = [string] $calculate[0].label
            DiagnosticCount = [int] $invalidDiagnostics.Count
            CloseCode       = 1000
        }
    }
    catch [System.OperationCanceledException] {
        Throw-OpenMatFailure -Message "LSP WebSocket smoke timed out after $TimeoutSeconds seconds" -ExitCode 31
    }
    catch {
        if ((Get-OpenMatFailureExitCode -ErrorRecord $_ -Default 0) -ne 0) {
            throw
        }
        Throw-OpenMatFailure -Message "LSP WebSocket smoke failed: $($_.Exception.Message)" -ExitCode 30
    }
    finally {
        if ($null -ne $socket) {
            if ($socket.State -notin @(
                    [System.Net.WebSockets.WebSocketState]::Closed,
                    [System.Net.WebSockets.WebSocketState]::Aborted,
                    [System.Net.WebSockets.WebSocketState]::None
                )) {
                $socket.Abort()
            }
            $socket.Dispose()
        }
        $timeout.Dispose()
    }
}

function Invoke-OpenMatSmokeLifecycle {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [System.Uri] $ServerUrl,

        [ValidateRange(1, 3600)]
        [int] $TimeoutSeconds = 30,

        [switch] $ForceV0
    )

    [void] (Assert-OpenMatServerUrl -Value $ServerUrl.AbsoluteUri)
    $sessionId = 'smoke-' + [Guid]::NewGuid().ToString('N')
    $receivedMessageIds = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $socket = $null
    $timeout = [System.Threading.CancellationTokenSource]::new()
    $timeout.CancelAfter([TimeSpan]::FromSeconds($TimeoutSeconds))
    $token = $timeout.Token

    try {
        # A listener has bound before it announces the URL, but Windows can
        # still briefly refuse the first connection while the accept loop is
        # being scheduled. Retry only the connection bootstrap, with fresh
        # ClientWebSocket instances and within the same overall timeout.
        $connectDeadline = [DateTime]::UtcNow.AddSeconds([Math]::Min(2, $TimeoutSeconds))
        while ($null -eq $socket) {
            $candidate = [System.Net.WebSockets.ClientWebSocket]::new()
            $candidate.Options.KeepAliveInterval = [TimeSpan]::FromSeconds(5)
            try {
                [void] $candidate.ConnectAsync($ServerUrl, $token).GetAwaiter().GetResult()
                $socket = $candidate
            }
            catch {
                $candidate.Dispose()
                if ($token.IsCancellationRequested -or [DateTime]::UtcNow -ge $connectDeadline) {
                    throw
                }
                Start-Sleep -Milliseconds 25
            }
        }

        $protocolOffer = if ($ForceV0) {
            @($script:OpenMatFallbackProtocol)
        }
        else {
            @($script:OpenMatPreferredProtocol, $script:OpenMatFallbackProtocol)
        }
        $clientCapabilities = [ordered]@{
            executionModes = @('repl')
            displayMimeTypes = @('text/plain')
            maxPreviewElements = 16
            interrupt = $false
            workspaceDelta = $true
        }
        if (-not $ForceV0) {
            $clientCapabilities.maxStringElementCodeUnits = 1024
            $clientCapabilities.maxPreviewCodeUnits = 4096
        }
        $initialize = New-OpenMatSmokeRequest `
            -SessionId $sessionId `
            -MessageId 'smoke-0001-initialize' `
            -Type 'initialize' `
            -Protocol $script:OpenMatBootstrapProtocol `
            -Params ([ordered]@{
                client = [ordered]@{ name = 'openmat-dev-smoke'; version = '1' }
                supportedProtocols = @($protocolOffer)
                capabilities = $clientCapabilities
            })
        Send-OpenMatRequest -Socket $socket -Request $initialize -CancellationToken $token

        $startup = Receive-OpenMatValidatedMessage `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -ExpectedProtocol $script:OpenMatBootstrapProtocol
        if ($startup.Kind -ne 'message' -or
            $startup.Message.kind -ne 'event' -or
            $startup.Message.event.type -ne 'status' -or
            $startup.Message.event.data.status -ne 'starting') {
            Throw-OpenMatFailure -Message 'First kernel message was not the startup status event' -ExitCode 30
        }

        $initialized = Receive-OpenMatValidatedMessage `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -ExpectedProtocol $script:OpenMatBootstrapProtocol
        if ($initialized.Kind -ne 'message' -or
            $initialized.Message.kind -ne 'response' -or
            $initialized.Message.replyTo -ne $initialize.messageId -or
            -not $initialized.Message.ok) {
            Throw-OpenMatFailure -Message 'Kernel did not return a successful bootstrap initialize response' -ExitCode 30
        }
        $initializeResult = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $initialized.Message -Key result -Context 'initialize response') -Context 'initialize result'
        if ($initializeResult.type -ne 'initialize') {
            Throw-OpenMatFailure -Message "Initialize returned unexpected result type '$($initializeResult.type)'" -ExitCode 30
        }
        $initializeData = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $initializeResult -Key data -Context 'initialize result') -Context 'initialize result.data'
        $sessionProtocol = Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $initializeData -Key negotiatedProtocol -Context 'initialize result.data') -Context 'initialize result.data.negotiatedProtocol'
        if ($sessionProtocol -notin $protocolOffer) {
            Throw-OpenMatFailure -Message 'Initialize negotiated an unexpected protocol' -ExitCode 30
        }
        $idle = Receive-OpenMatValidatedMessage `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -ExpectedProtocol $sessionProtocol
        if ($idle.Kind -ne 'message' -or
            $idle.Message.kind -ne 'event' -or
            $idle.Message.event.type -ne 'status' -or
            $idle.Message.event.data.status -ne 'idle') {
            Throw-OpenMatFailure -Message 'Initialize was not immediately followed by negotiated-protocol idle' -ExitCode 30
        }
        $implementation = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $initializeData -Key implementation -Context 'initialize result.data') -Context 'initialize implementation'
        [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $implementation -Key name -Context 'initialize implementation') -Context 'initialize implementation.name')
        [void] (Assert-OpenMatNonEmptyString -Value (Get-OpenMatRequiredValue -Map $implementation -Key version -Context 'initialize implementation') -Context 'initialize implementation.version')
        $capabilities = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $initializeData -Key capabilities -Context 'initialize result.data') -Context 'initialize capabilities'
        $executionModes = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $capabilities -Key executionModes -Context 'initialize capabilities') -Context 'initialize capabilities.executionModes'
        if ('repl' -notin $executionModes) {
            Throw-OpenMatFailure -Message 'Initialize did not negotiate repl execution mode' -ExitCode 30
        }
        $displayMimeTypes = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $capabilities -Key displayMimeTypes -Context 'initialize capabilities') -Context 'initialize capabilities.displayMimeTypes'
        if ('text/plain' -notin $displayMimeTypes) {
            Throw-OpenMatFailure -Message 'Initialize did not negotiate text/plain display output' -ExitCode 30
        }
        $previewLimit = Get-OpenMatRequiredValue -Map $capabilities -Key maxPreviewElements -Context 'initialize capabilities'
        if (-not (Test-OpenMatJsonNumber -Value $previewLimit) -or [int64] $previewLimit -lt 1 -or [int64] $previewLimit -gt 16) {
            Throw-OpenMatFailure -Message 'Initialize returned an invalid negotiated preview limit' -ExitCode 30
        }
        foreach ($booleanCapability in @('interrupt', 'workspaceDelta')) {
            $capabilityValue = Get-OpenMatRequiredValue -Map $capabilities -Key $booleanCapability -Context 'initialize capabilities'
            if ($capabilityValue -isnot [bool]) {
                Throw-OpenMatFailure -Message "Initialize capability '$booleanCapability' must be Boolean" -ExitCode 30
            }
        }
        if ($sessionProtocol -eq $script:OpenMatPreferredProtocol) {
            $stringLimit = Get-OpenMatRequiredValue -Map $capabilities -Key maxStringElementCodeUnits -Context 'initialize capabilities'
            $codeUnitLimit = Get-OpenMatRequiredValue -Map $capabilities -Key maxPreviewCodeUnits -Context 'initialize capabilities'
            if (-not (Test-OpenMatJsonNumber -Value $stringLimit) -or
                -not (Test-OpenMatJsonNumber -Value $codeUnitLimit) -or
                [int64] $stringLimit -lt 1 -or [int64] $stringLimit -gt 1024 -or
                [int64] $codeUnitLimit -lt [int64] $stringLimit -or [int64] $codeUnitLimit -gt 4096) {
                Throw-OpenMatFailure -Message 'Initialize returned invalid v1 code-unit limits' -ExitCode 30
            }
        }
        elseif ((Test-OpenMatMapKey -Map $capabilities -Key maxStringElementCodeUnits) -or
            (Test-OpenMatMapKey -Map $capabilities -Key maxPreviewCodeUnits)) {
            Throw-OpenMatFailure -Message 'V0 fallback returned v1-only capability fields' -ExitCode 30
        }

        $source = @'
counter = 0;
while counter < 4
counter = counter + 1;
end
result = counter * 10;
disp(result);
'@
        $execute = New-OpenMatSmokeRequest `
            -SessionId $sessionId `
            -MessageId 'smoke-0002-execute' `
            -Type 'execute' `
            -Protocol $sessionProtocol `
            -Params ([ordered]@{ code = $source; sourceName = 'openmat-dev-smoke.m'; mode = 'repl' })
        $executed = Invoke-OpenMatSmokeExchange `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -Request $execute `
            -ExpectedResultType 'execute' `
            -RequiredStatuses @('busy', 'idle') `
            -ExpectedProtocol $sessionProtocol
        $executeData = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $executed.Result -Key data -Context 'execute result') -Context 'execute result.data'
        $interrupted = Get-OpenMatRequiredValue -Map $executeData -Key interrupted -Context 'execute result.data'
        if ($interrupted -isnot [bool] -or $interrupted) {
            Throw-OpenMatFailure -Message 'Scalar smoke execution was unexpectedly interrupted' -ExitCode 30
        }
        $sawDisplay = $false
        foreach ($event in $executed.Events) {
            if ($event.type -eq 'display' -and
                (Test-OpenMatMapKey -Map $event.data.representations -Key 'text/plain') -and
                ([string] $event.data.representations['text/plain']).Trim() -eq '40') {
                $sawDisplay = $true
            }
        }
        if (-not $sawDisplay) {
            $observedDisplays = @(
                $executed.Events |
                    Where-Object { $_.type -eq 'display' } |
                    ForEach-Object { $_.data.representations['text/plain'] }
            )
            $observedDescription = if ($observedDisplays.Count -eq 0) {
                'none'
            }
            else {
                ($observedDisplays | ConvertTo-Json -Compress)
            }
            Throw-OpenMatFailure `
                -Message "Execute did not emit the expected text/plain display value '40' (observed: $observedDescription)" `
                -ExitCode 30
        }

        $list = New-OpenMatSmokeRequest `
            -SessionId $sessionId `
            -MessageId 'smoke-0003-list-workspace' `
            -Type 'listWorkspace' `
            -Protocol $sessionProtocol `
            -Params ([ordered]@{})
        $listed = Invoke-OpenMatSmokeExchange `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -Request $list `
            -ExpectedResultType 'listWorkspace' `
            -RequiredStatuses @('busy', 'idle') `
            -ExpectedProtocol $sessionProtocol
        $workspaceData = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $listed.Result -Key data -Context 'listWorkspace result') -Context 'listWorkspace result.data'
        $variables = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $workspaceData -Key variables -Context 'listWorkspace result.data') -Context 'listWorkspace variables'
        $byName = @{}
        $previousName = $null
        for ($index = 0; $index -lt $variables.Count; $index++) {
            $summary = Assert-OpenMatMap -Value $variables[$index] -Context "workspace variable $index"
            Assert-OpenMatVariableSummary -Summary $summary -Context "workspace variable $index"
            if ($null -ne $previousName -and [string]::CompareOrdinal($previousName, [string] $summary.name) -ge 0) {
                Throw-OpenMatFailure -Message 'Workspace variables were not in deterministic name order' -ExitCode 30
            }
            if ($byName.ContainsKey([string] $summary.name)) {
                Throw-OpenMatFailure -Message "Workspace listed duplicate variable '$($summary.name)'" -ExitCode 30
            }
            $byName[[string] $summary.name] = $summary
            $previousName = [string] $summary.name
        }
        foreach ($expectedName in @('counter', 'result')) {
            if (-not $byName.ContainsKey($expectedName)) {
                Throw-OpenMatFailure -Message "Workspace did not contain '$expectedName'" -ExitCode 30
            }
            Assert-OpenMatScalarSummary -Summary $byName[$expectedName] -Name $expectedName
        }

        $inspect = New-OpenMatSmokeRequest `
            -SessionId $sessionId `
            -MessageId 'smoke-0004-inspect' `
            -Type 'inspect' `
            -Protocol $sessionProtocol `
            -Params ([ordered]@{
                name = 'result'
                range = [ordered]@{ start = @(1, 1); size = @(1, 1) }
                maxElements = 1
            })
        $inspected = Invoke-OpenMatSmokeExchange `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -Request $inspect `
            -ExpectedResultType 'inspect' `
            -RequiredStatuses @('busy', 'idle') `
            -ExpectedProtocol $sessionProtocol
        $preview = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $inspected.Result -Key data -Context 'inspect result') -Context 'inspect result.data'
        if ($preview.class -ne 'double') {
            Throw-OpenMatFailure -Message "Inspect returned unexpected class '$($preview.class)'" -ExitCode 30
        }
        $previewDimensions = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $preview -Key dimensions -Context 'inspect result.data') -Context 'inspect dimensions'
        if ($previewDimensions.Count -ne 2 -or [int64] $previewDimensions[0] -ne 1 -or [int64] $previewDimensions[1] -ne 1) {
            Throw-OpenMatFailure -Message 'Inspect did not return 1-by-1 dimensions' -ExitCode 30
        }
        $selectedRange = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $preview -Key selectedRange -Context 'inspect result.data') -Context 'inspect selectedRange'
        $rangeStart = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $selectedRange -Key start -Context 'inspect selectedRange') -Context 'inspect selectedRange.start'
        $rangeSize = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $selectedRange -Key size -Context 'inspect selectedRange') -Context 'inspect selectedRange.size'
        if ($rangeStart.Count -ne 2 -or $rangeSize.Count -ne 2 -or
            [int64] $rangeStart[0] -ne 1 -or [int64] $rangeStart[1] -ne 1 -or
            [int64] $rangeSize[0] -ne 1 -or [int64] $rangeSize[1] -ne 1) {
            Throw-OpenMatFailure -Message 'Inspect returned an unexpected selectedRange' -ExitCode 30
        }
        $values = Assert-OpenMatArray -Value (Get-OpenMatRequiredValue -Map $preview -Key values -Context 'inspect result.data') -Context 'inspect values'
        if ($values.Count -ne 1) {
            Throw-OpenMatFailure -Message 'Inspect did not return exactly one preview value' -ExitCode 30
        }
        $value = Assert-OpenMatMap -Value $values[0] -Context 'inspect values[0]'
        if ($value.kind -ne 'number' -or -not (Test-OpenMatJsonNumber -Value $value.value) -or [double] $value.value -ne 40.0) {
            Throw-OpenMatFailure -Message 'Inspect did not return numeric value 40' -ExitCode 30
        }
        $truncation = Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $preview -Key truncation -Context 'inspect result.data') -Context 'inspect truncation'
        if ($truncation.truncated -isnot [bool] -or $truncation.truncated -or
            -not (Test-OpenMatJsonNumber -Value $truncation.omittedElements) -or
            [int64] $truncation.omittedElements -ne 0) {
            Throw-OpenMatFailure -Message 'Inspect returned inconsistent truncation metadata' -ExitCode 30
        }

        $shutdown = New-OpenMatSmokeRequest `
            -SessionId $sessionId `
            -MessageId 'smoke-0005-shutdown' `
            -Type 'shutdown' `
            -Protocol $sessionProtocol `
            -Params ([ordered]@{})
        $shutDown = Invoke-OpenMatSmokeExchange `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -Request $shutdown `
            -ExpectedResultType 'shutdown' `
            -RequiredStatuses @('dead') `
            -ExpectedProtocol $sessionProtocol
        [void] (Assert-OpenMatMap -Value (Get-OpenMatRequiredValue -Map $shutDown.Result -Key data -Context 'shutdown result') -Context 'shutdown result.data')

        $close = Receive-OpenMatValidatedMessage `
            -Socket $socket `
            -CancellationToken $token `
            -SessionId $sessionId `
            -ReceivedMessageIds $receivedMessageIds `
            -ExpectedProtocol $sessionProtocol
        if ($close.Kind -ne 'close' -or
            $close.CloseStatus -ne [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure) {
            Throw-OpenMatFailure -Message "Kernel did not close normally after shutdown (status: $($close.CloseStatus))" -ExitCode 30
        }
        [void] $socket.CloseOutputAsync(
            [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure,
            'smoke complete',
            $token
        ).GetAwaiter().GetResult()

        return [pscustomobject]@{
            SessionId = $sessionId
            Value     = 40
            CloseCode = 1000
            Protocol  = $sessionProtocol
        }
    }
    catch [System.OperationCanceledException] {
        Throw-OpenMatFailure -Message "WebSocket smoke timed out after $TimeoutSeconds seconds" -ExitCode 31
    }
    catch {
        if ((Get-OpenMatFailureExitCode -ErrorRecord $_ -Default 0) -ne 0) {
            throw
        }
        Throw-OpenMatFailure -Message "WebSocket smoke failed: $($_.Exception.Message)" -ExitCode 30
    }
    finally {
        if ($null -ne $socket) {
            if ($socket.State -notin @(
                    [System.Net.WebSockets.WebSocketState]::Closed,
                    [System.Net.WebSockets.WebSocketState]::Aborted,
                    [System.Net.WebSockets.WebSocketState]::None
                )) {
                $socket.Abort()
            }
            $socket.Dispose()
        }
        $timeout.Dispose()
    }
}

Export-ModuleMember -Function @(
    'Assert-OpenMatServerUrl',
    'Build-OpenMatPlotWasm',
    'Build-OpenMatServer',
    'Close-OpenMatProcessRecord',
    'Get-OpenMatFailureExitCode',
    'Get-OpenMatLspUrl',
    'Invoke-OpenMatLspSmokeLifecycle',
    'Invoke-OpenMatSmokeLifecycle',
    'Invoke-OpenMatWorkspaceSmokeLifecycle',
    'New-OpenMatLogDirectory',
    'Resolve-OpenMatCommand',
    'Resolve-OpenMatRepositoryRoot',
    'Resolve-OpenMatServerPath',
    'Start-OpenMatFrontend',
    'Start-OpenMatLoggedProcess',
    'Start-OpenMatServer',
    'Stop-OpenMatProcessTree',
    'Update-OpenMatProcessLogs',
    'Wait-OpenMatFrontendReady',
    'Wait-OpenMatProcessExit',
    'Wait-OpenMatServerUrl'
)
