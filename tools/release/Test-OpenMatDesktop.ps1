[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string] $Executable,

    [ValidateRange(0, 65535)]
    [int] $KernelPort = 0,

    [string] $RuntimeConfig,

    [switch] $UseDefaultWorkspace,

    [switch] $CheckBundledExamples
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Send-WebSocketJson {
    param(
        [Parameter(Mandatory)] [System.Net.WebSockets.ClientWebSocket] $Socket,
        [Parameter(Mandatory)] [hashtable] $Message
    )

    $json = $Message | ConvertTo-Json -Depth 20 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($json)
    $segment = [ArraySegment[byte]]::new($bytes)
    [void] $Socket.SendAsync(
        $segment,
        [Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult()
}

function Receive-WebSocketJson {
    param(
        [Parameter(Mandatory)] [System.Net.WebSockets.ClientWebSocket] $Socket,
        [int] $TimeoutSeconds = 10
    )

    $buffer = [byte[]]::new(1048576)
    $stream = [IO.MemoryStream]::new()
    $cancel = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds($TimeoutSeconds))
    try {
        do {
            $segment = [ArraySegment[byte]]::new($buffer)
            $result = $Socket.ReceiveAsync($segment, $cancel.Token).GetAwaiter().GetResult()
            if ($result.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) {
                throw "WebSocket closed during desktop smoke: $($Socket.CloseStatus) $($Socket.CloseStatusDescription)"
            }
            $stream.Write($buffer, 0, $result.Count)
        } until ($result.EndOfMessage)
        return ([Text.Encoding]::UTF8.GetString($stream.ToArray()) | ConvertFrom-Json)
    }
    finally {
        $cancel.Dispose()
        $stream.Dispose()
    }
}

function Receive-ResponseFor {
    param(
        [Parameter(Mandatory)] [System.Net.WebSockets.ClientWebSocket] $Socket,
        [Parameter(Mandatory)] [string] $ReplyTo,
        [int] $TimeoutSeconds = 10
    )

    for ($index = 0; $index -lt 32; $index++) {
        $message = Receive-WebSocketJson -Socket $Socket -TimeoutSeconds $TimeoutSeconds
        if ($message.kind -eq 'response' -and $message.replyTo -eq $ReplyTo) {
            return $message
        }
    }
    throw "No response for '$ReplyTo'."
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$workspace = Join-Path $env:TEMP ("openmat-desktop-smoke-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workspace | Out-Null
$previousWorkspace = $env:OPENMAT_WORKSPACE_ROOT
$previousRuntimeConfig = $env:OPENMAT_RUNTIME_CONFIG
$previousTestPort = $env:OPENMAT_DESKTOP_TEST_PORT
$previousWebviewData = $env:WEBVIEW2_USER_DATA_FOLDER
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $workspace 'webview'
$env:OPENMAT_WORKSPACE_ROOT = if ($UseDefaultWorkspace) { $null } else { $workspace }
$env:OPENMAT_RUNTIME_CONFIG = if ([string]::IsNullOrWhiteSpace($RuntimeConfig)) {
    Join-Path $workspace 'runtime.json'
} else { [IO.Path]::GetFullPath($RuntimeConfig) }
if ([string]::IsNullOrWhiteSpace($RuntimeConfig)) {
    @{ schemaVersion = 1; kernelPort = $(if ($KernelPort -eq 0) { 42000 } else { $KernelPort }) } |
        ConvertTo-Json | Set-Content -LiteralPath $env:OPENMAT_RUNTIME_CONFIG -Encoding utf8
}
$env:OPENMAT_DESKTOP_TEST_PORT = if ($KernelPort -eq 0 -and [string]::IsNullOrWhiteSpace($RuntimeConfig)) { '0' } else { $null }
try {
    $app = Start-Process -FilePath $resolvedExecutable -PassThru -WindowStyle Hidden
}
finally {
    $env:OPENMAT_WORKSPACE_ROOT = $previousWorkspace
    $env:OPENMAT_RUNTIME_CONFIG = $previousRuntimeConfig
    $env:OPENMAT_DESKTOP_TEST_PORT = $previousTestPort
    $env:WEBVIEW2_USER_DATA_FOLDER = $previousWebviewData
}

$socket = $null
try {
    $deadline = (Get-Date).AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 200
        if ($app.HasExited) {
            throw "OpenMat.exe exited during desktop smoke with code $($app.ExitCode)."
        }
        $listener = Get-NetTCPConnection -State Listen -OwningProcess $app.Id -ErrorAction SilentlyContinue |
            Where-Object { $_.LocalAddress -eq '127.0.0.1' } |
            Select-Object -First 1
    } until ($null -ne $listener -or (Get-Date) -ge $deadline)
    if ($null -eq $listener) {
        throw 'OpenMat.exe did not open its in-process loopback listener.'
    }
    if ($KernelPort -ne 0 -and $listener.LocalPort -ne $KernelPort) {
        throw "Expected configured kernel port $KernelPort, got $($listener.LocalPort)."
    }

    $sidecars = @(Get-CimInstance Win32_Process -Filter "Name = 'openmat-server.exe'" |
        Where-Object { $_.ParentProcessId -eq $app.Id })
    if ($sidecars.Count -ne 0) {
        throw "OpenMat.exe unexpectedly started $($sidecars.Count) server sidecar process(es)."
    }

    $socket = [Net.WebSockets.ClientWebSocket]::new()
    $socket.Options.SetRequestHeader('Origin', 'http://tauri.localhost')
    $connectCancel = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds(10))
    try {
        $url = [Uri]::new("ws://127.0.0.1:$($listener.LocalPort)/kernel")
        [void] $socket.ConnectAsync($url, $connectCancel.Token).GetAwaiter().GetResult()
    }
    finally {
        $connectCancel.Dispose()
    }

    $session = 'desktop-inprocess-smoke'
    Send-WebSocketJson -Socket $socket -Message @{
        protocol = 'openmat-kernel-v0'
        sessionId = $session
        messageId = 'initialize'
        kind = 'request'
        request = @{
            type = 'initialize'
            params = @{
                client = @{ name = 'desktop-release-smoke'; version = '1' }
                supportedProtocols = @('openmat-kernel-v0')
                capabilities = @{
                    executionModes = @('repl')
                    displayMimeTypes = @('text/plain')
                    maxPreviewElements = 32
                    interrupt = $true
                    workspaceDelta = $true
                }
            }
        }
    }
    $initialized = Receive-ResponseFor -Socket $socket -ReplyTo 'initialize'
    if (-not $initialized.ok) {
        throw "Kernel initialize failed: $($initialized | ConvertTo-Json -Depth 20 -Compress)"
    }

    Send-WebSocketJson -Socket $socket -Message @{
        protocol = 'openmat-kernel-v0'
        sessionId = $session
        messageId = 'execute'
        kind = 'request'
        request = @{
            type = 'execute'
            params = @{
                code = 'A = [4 1; 1 3]; b = [1; 2]; assert(norm(A * (A \ b) - b) < 1e-10); assert(norm(fft([1 0 0 0]) - ones(1, 4)) < 1e-10); desktop_answer = 6 * 7; assert(desktop_answer == 42);'
                sourceName = 'desktop-smoke.m'
                mode = 'repl'
            }
        }
    }
    $executed = Receive-ResponseFor -Socket $socket -ReplyTo 'execute'
    if (-not $executed.ok -or
        $executed.result.type -ne 'execute' -or
        $executed.result.data.interrupted) {
        throw "Kernel execute failed: $($executed | ConvertTo-Json -Depth 20 -Compress)"
    }

    Send-WebSocketJson -Socket $socket -Message @{
        protocol = 'openmat-kernel-v0'
        sessionId = $session
        messageId = 'list-workspace'
        kind = 'request'
        request = @{ type = 'listWorkspace'; params = @{} }
    }
    $listed = Receive-ResponseFor -Socket $socket -ReplyTo 'list-workspace'
    $answer = @($listed.result.data.variables | Where-Object { $_.name -eq 'desktop_answer' })
    if (-not $listed.ok -or $answer.Count -ne 1 -or $answer[0].class -ne 'double') {
        throw "Kernel workspace verification failed: $($listed | ConvertTo-Json -Depth 20 -Compress)"
    }

    Write-Host "Desktop smoke passed: $url (single process, initialize/execute/workspace, linear solve and FFT)."

    if ($CheckBundledExamples) {
        $exampleRoot = Join-Path (Split-Path -Parent $resolvedExecutable) 'resources\examples'
        $catalog = Get-Content -LiteralPath (Join-Path $exampleRoot 'catalog.json') -Raw | ConvertFrom-Json
        if ($UseDefaultWorkspace) {
            $workspaceSocket = [Net.WebSockets.ClientWebSocket]::new()
            $workspaceSocket.Options.SetRequestHeader('Origin', 'http://tauri.localhost')
            $workspaceCancel = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds(10))
            try {
                [void] $workspaceSocket.ConnectAsync(
                    [Uri]::new("ws://127.0.0.1:$($listener.LocalPort)/workspace/v3"), $workspaceCancel.Token
                ).GetAwaiter().GetResult()
                Send-WebSocketJson -Socket $workspaceSocket -Message @{
                    protocol = 'openmat-workspace-v3'; requestId = 'example-folder'
                    request = @{ type = 'currentDirectory'; params = @{} }
                }
                $directory = Receive-WebSocketJson -Socket $workspaceSocket
                if (-not $directory.ok) { throw 'Could not read the default Current Folder.' }
                $currentPath = [string] $directory.result.data.path
                $version = (Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\..\apps\desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
                $expectedPath = Join-Path ([Environment]::GetFolderPath('MyDocuments')) "OpenMat\Examples\$version"
                if (([IO.Path]::GetFullPath($currentPath)) -ne ([IO.Path]::GetFullPath($expectedPath))) {
                    throw "Default Current Folder is '$currentPath', expected '$expectedPath'."
                }
                foreach ($file in @('README.txt') + @($catalog.examples.file)) {
                    if (-not (Test-Path -LiteralPath (Join-Path $currentPath $file) -PathType Leaf)) {
                        throw "The default example workspace is missing '$file'."
                    }
                }
                Write-Host "Default Current Folder passed: $currentPath"
            }
            finally {
                $workspaceSocket.Dispose()
                $workspaceCancel.Dispose()
            }
        }
        foreach ($example in $catalog.examples) {
            $name = [string] $example.file
            if ($name -notmatch '^[A-Za-z0-9_-]+\.m$') { throw "Invalid example filename '$name'." }
            $code = Get-Content -LiteralPath (Join-Path $exampleRoot $name) -Raw
            $validation = "assert(isgraphics(gcf())); assert(~isempty(get(gcf(), 'Children')));"
            if ($name -eq 'fft_spectrum.m') {
                $validation += ' assert(abs(amplitude(51) - 0.8) < 1e-10); assert(abs(amplitude(121) - 0.35) < 1e-10);'
            }
            if ($name -eq 'least_squares_fit.m') {
                $validation += " assert(norm(design' * residual) < 1e-8); assert(rmse > 0 && rmse < 0.3);"
            }
            Send-WebSocketJson -Socket $socket -Message @{
                protocol = 'openmat-kernel-v0'; sessionId = $session
                messageId = "example-$name"; kind = 'request'
                request = @{ type = 'execute'; params = @{
                    code = "close all;`n$code`n$validation`nclose all;"
                    sourceName = $name; mode = 'repl'
                } }
            }
            $result = Receive-ResponseFor -Socket $socket -ReplyTo "example-$name" -TimeoutSeconds 60
            if (-not $result.ok -or $result.result.type -ne 'execute' -or $result.result.data.interrupted) {
                throw "Bundled example '$name' failed: $($result | ConvertTo-Json -Depth 20 -Compress)"
            }
            Write-Host "Bundled example passed: $name"
        }
    }
}
finally {
    if ($null -ne $socket) {
        $socket.Dispose()
    }
    $window = Get-Process -Id $app.Id -ErrorAction SilentlyContinue
    if ($null -ne $window) {
        [void] $window.CloseMainWindow()
        if (-not $window.WaitForExit(10000)) {
            Stop-Process -Id $app.Id -Force
        }
    }
    if ([IO.Directory]::Exists($workspace)) {
        $resolvedWorkspace = [IO.Path]::GetFullPath($workspace)
        $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        if (-not $resolvedWorkspace.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Refusing to clean a smoke workspace outside the temporary directory.'
        }
        # WebView2 can retain Crashpad files briefly after the host exits.
        # Cleanup must not terminate unrelated WebView2 processes or turn a
        # successful kernel verification into a failure due to that file lock.
        for ($cleanupAttempt = 0; $cleanupAttempt -lt 10; $cleanupAttempt++) {
            try {
                [IO.Directory]::Delete($workspace, $true)
                break
            }
            catch [IO.IOException] {
                if ($cleanupAttempt -eq 9) {
                    Write-Warning "Temporary desktop smoke files remain locked at '$workspace'."
                } else {
                    Start-Sleep -Milliseconds 500
                }
            }
        }
    }
}
