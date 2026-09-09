[CmdletBinding()]
param(
    [switch] $Smoke,

    [string] $RuntimeConfig = $env:OPENMAT_RUNTIME_CONFIG,

    [string] $ServerPath = $env:OPENMAT_SERVER_PATH,

    [string] $RepositoryRoot,

    [string] $WorkspaceRoot = $env:OPENMAT_WORKSPACE_ROOT,

    [string] $LogDirectory = $env:OPENMAT_DEV_LOG_DIR,

    [string] $CargoPath = $env:OPENMAT_CARGO,

    [string] $PnpmPath = $env:OPENMAT_PNPM,

    [ValidateRange(1, 16)]
    [int] $BuildJobs = 6,

    [switch] $DisableCompilerCache,

    [ValidateRange(1, 86400)]
    [int] $BuildTimeoutSeconds = 600,

    [ValidateRange(1, 3600)]
    [int] $ServerStartupTimeoutSeconds = 30,

    [ValidateRange(1, 3600)]
    [int] $FrontendStartupTimeoutSeconds = 30,

    [ValidateRange(1, 3600)]
    [int] $SmokeTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module -Force ([System.IO.Path]::Combine($PSScriptRoot, 'OpenMat.Dev.psm1'))

$exitCode = 1
$ownedProcesses = [System.Collections.Generic.List[object]]::new()

try {
    $root = Resolve-OpenMatRepositoryRoot -Path $RepositoryRoot
    $kernelPort = 0
    if (-not $Smoke -or -not [string]::IsNullOrWhiteSpace($RuntimeConfig)) {
        $runtimeSettings = & (Join-Path $PSScriptRoot 'Read-OpenMatRuntimeSettings.ps1') -ConfigPath $RuntimeConfig
        $kernelPort = $runtimeSettings.KernelPort
        [Console]::WriteLine("Runtime settings: $($runtimeSettings.ConfigPath); kernel: 127.0.0.1:$kernelPort")
    }
    $needsRustBuild = [string]::IsNullOrWhiteSpace($ServerPath) -or -not $Smoke
    if ($needsRustBuild) {
        $buildEnvironment = & ([System.IO.Path]::Combine(
                $root,
                'tools',
                'build',
                'Initialize-OpenMatBuildEnvironment.ps1'
            )) `
            -RepositoryRoot $root `
            -BuildJobs $BuildJobs `
            -DisableCompilerCache:$DisableCompilerCache
        [Console]::WriteLine(
            "Build jobs: $($buildEnvironment.BuildJobs); compiler cache: $($buildEnvironment.CompilerCacheReason)"
        )
    }
    if ([string]::IsNullOrWhiteSpace($WorkspaceRoot)) {
        $WorkspaceRoot = $root
    }
    else {
        try {
            $WorkspaceRoot = [System.IO.Path]::GetFullPath($WorkspaceRoot)
        }
        catch {
            Throw-OpenMatFailure -Message "Invalid workspace root '$WorkspaceRoot': $($_.Exception.Message)" -ExitCode 2
        }
        if (-not [System.IO.Directory]::Exists($WorkspaceRoot)) {
            Throw-OpenMatFailure -Message "Workspace root does not exist: $WorkspaceRoot" -ExitCode 2
        }
    }
    $logs = New-OpenMatLogDirectory -Path $LogDirectory
    [Console]::WriteLine("OpenMat logs: $logs")

    if ([string]::IsNullOrWhiteSpace($ServerPath)) {
        [Console]::WriteLine('Building openmat-server...')
        $ServerPath = Build-OpenMatServer `
            -RepositoryRoot $root `
            -LogDirectory $logs `
            -CargoPath $CargoPath `
            -TimeoutSeconds $BuildTimeoutSeconds
    }
    else {
        $ServerPath = Resolve-OpenMatServerPath -Path $ServerPath
    }
    [Console]::WriteLine("Server executable: $ServerPath")

    $server = Start-OpenMatServer `
        -ServerPath $ServerPath `
        -RepositoryRoot $root `
        -LogDirectory $logs `
        -WorkspaceRoot $WorkspaceRoot `
        -Port $kernelPort
    $ownedProcesses.Add($server)
    $serverUrl = Wait-OpenMatServerUrl `
        -Record $server `
        -TimeoutSeconds $ServerStartupTimeoutSeconds
    [Console]::WriteLine("Kernel WebSocket: $($serverUrl.AbsoluteUri)")
    $lspUrl = Get-OpenMatLspUrl -ServerUrl $serverUrl
    [Console]::WriteLine("LSP WebSocket: $($lspUrl.AbsoluteUri)")

    if ($Smoke) {
        $result = Invoke-OpenMatSmokeLifecycle `
            -ServerUrl $serverUrl `
            -TimeoutSeconds $SmokeTimeoutSeconds
        $workspaceResult = Invoke-OpenMatWorkspaceSmokeLifecycle `
            -ServerUrl $serverUrl `
            -WorkspaceRoot $WorkspaceRoot `
            -TimeoutSeconds $SmokeTimeoutSeconds
        $lspResult = Invoke-OpenMatLspSmokeLifecycle `
            -ServerUrl $serverUrl `
            -TimeoutSeconds $SmokeTimeoutSeconds
        [Console]::WriteLine(
            "Smoke passed: session=$($result.SessionId), protocol=$($result.Protocol), value=$($result.Value), workspace=$($workspaceResult.CreatedCount) created, lsp=$($lspResult.CompletionLabel), close=$($result.CloseCode)"
        )
        $exitCode = 0
    }
    else {
        [Console]::WriteLine('Building Plot Engine WASM...')
        Build-OpenMatPlotWasm `
            -RepositoryRoot $root `
            -LogDirectory $logs `
            -PnpmPath $PnpmPath `
            -TimeoutSeconds $BuildTimeoutSeconds
        $frontend = Start-OpenMatFrontend `
            -RepositoryRoot $root `
            -LogDirectory $logs `
            -ServerUrl $serverUrl `
            -PnpmPath $PnpmPath
        $ownedProcesses.Add($frontend)
        $readyLine = Wait-OpenMatFrontendReady `
            -Record $frontend `
            -TimeoutSeconds $FrontendStartupTimeoutSeconds
        [Console]::WriteLine("Frontend ready: $($readyLine.Trim())")
        [Console]::WriteLine('Press Ctrl-C to stop the frontend and the server.')

        while ($true) {
            Update-OpenMatProcessLogs -Record $server
            Update-OpenMatProcessLogs -Record $frontend
            if ($server.Process.HasExited) {
                $failure = [System.InvalidOperationException]::new(
                    "openmat-server exited unexpectedly with code $($server.Process.ExitCode); see $($server.StderrPath)"
                )
                $failure.Data['OpenMatExitCode'] = 23
                throw $failure
            }
            if ($frontend.Process.HasExited) {
                $failure = [System.InvalidOperationException]::new(
                    "Vite exited unexpectedly with code $($frontend.Process.ExitCode); see $($frontend.StderrPath)"
                )
                $failure.Data['OpenMatExitCode'] = 42
                throw $failure
            }
            Start-Sleep -Milliseconds 100
        }
    }
}
catch [System.Management.Automation.PipelineStoppedException] {
    # PowerShell raises PipelineStoppedException for Ctrl-C. The finally block
    # still owns cleanup of only the process records created above.
    $exitCode = 130
}
catch {
    $exitCode = Get-OpenMatFailureExitCode -ErrorRecord $_ -Default 1
    [Console]::Error.WriteLine("openmat-dev: $($_.Exception.Message)")
}
finally {
    for ($index = $ownedProcesses.Count - 1; $index -ge 0; $index--) {
        Stop-OpenMatProcessTree -Record $ownedProcesses[$index]
    }
}

exit $exitCode
