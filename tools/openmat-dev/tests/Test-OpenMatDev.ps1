[CmdletBinding()]
param(
    [string] $ServerPath = $env:OPENMAT_SERVER_PATH,

    [string] $RepositoryRoot,

    [string] $CargoPath = $env:OPENMAT_CARGO,

    [ValidateRange(1, 86400)]
    [int] $BuildTimeoutSeconds = 600,

    [ValidateRange(1, 3600)]
    [int] $SmokeTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$toolRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($PSScriptRoot, '..'))
Import-Module -Force ([System.IO.Path]::Combine($toolRoot, 'OpenMat.Dev.psm1'))

$root = Resolve-OpenMatRepositoryRoot -Path $RepositoryRoot
$temporaryBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testRoot = [System.IO.Path]::Combine(
    $temporaryBase,
    'openmat-dev-selftest-' + [Guid]::NewGuid().ToString('N')
)
[void] [System.IO.Directory]::CreateDirectory($testRoot)

$failures = [System.Collections.Generic.List[string]]::new()
$passes = 0

function Invoke-SelfTest {
    param(
        [Parameter(Mandatory)]
        [string] $Name,

        [Parameter(Mandatory)]
        [scriptblock] $Body
    )

    try {
        & $Body
        $script:passes++
        [Console]::WriteLine("PASS $Name")
    }
    catch {
        $script:failures.Add("${Name}: $($_.Exception.Message)")
        [Console]::Error.WriteLine("FAIL ${Name}: $($_.Exception.Message)")
    }
}

function Assert-True {
    param(
        [Parameter(Mandatory)]
        [bool] $Condition,

        [Parameter(Mandatory)]
        [string] $Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-OpenMatExitCode {
    param(
        [Parameter(Mandatory)]
        [scriptblock] $Body,

        [Parameter(Mandatory)]
        [int] $Expected
    )

    try {
        & $Body
    }
    catch {
        $actual = Get-OpenMatFailureExitCode -ErrorRecord $_ -Default -1
        if ($actual -ne $Expected) {
            throw "Expected OpenMat exit code $Expected, got $actual ($($_.Exception.Message))"
        }
        return
    }
    throw "Expected OpenMat exit code $Expected, but the operation succeeded"
}

function Wait-ProcessGone {
    param(
        [Parameter(Mandatory)]
        [int] $Id,

        [int] $TimeoutMilliseconds = 5000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        if ($null -eq (Get-Process -Id $Id -ErrorAction SilentlyContinue)) {
            return
        }
        Start-Sleep -Milliseconds 20
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Process $Id was still alive after cleanup"
}

function Invoke-TreeCleanupCase {
    param(
        [Parameter(Mandatory)]
        [string] $Name,

        [string] $FirstLine,

        [Parameter(Mandatory)]
        [int] $ExpectedExitCode
    )

    $caseRoot = [System.IO.Path]::Combine($testRoot, $Name)
    [void] [System.IO.Directory]::CreateDirectory($caseRoot)
    $childPidPath = [System.IO.Path]::Combine($caseRoot, 'child.pid')
    $fixture = [System.IO.Path]::Combine($PSScriptRoot, 'fixtures', 'Wait-WithChild.ps1')
    $pwsh = Resolve-OpenMatCommand -Name 'pwsh.exe'
    $arguments = @(
        '-NoProfile',
        '-NonInteractive',
        '-File',
        $fixture,
        '-ChildPidPath',
        $childPidPath
    )
    if (-not [string]::IsNullOrEmpty($FirstLine)) {
        $arguments += @('-FirstLine', $FirstLine)
    }

    $record = $null
    $parentPid = 0
    $childPid = 0
    try {
        $record = Start-OpenMatLoggedProcess `
            -Name $Name `
            -FilePath $pwsh `
            -ArgumentList $arguments `
            -WorkingDirectory $root `
            -LogDirectory $caseRoot `
            -LaunchFailureExitCode 90
        $parentPid = $record.Process.Id

        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        while (-not [System.IO.File]::Exists($childPidPath) -and [DateTime]::UtcNow -lt $deadline) {
            Update-OpenMatProcessLogs -Record $record
            Start-Sleep -Milliseconds 20
        }
        Assert-True -Condition ([System.IO.File]::Exists($childPidPath)) -Message 'Fixture did not publish its child PID'
        $childPid = [int] ([System.IO.File]::ReadAllText($childPidPath))

        Assert-OpenMatExitCode `
            -Expected $ExpectedExitCode `
            -Body { [void] (Wait-OpenMatServerUrl -Record $record -TimeoutSeconds 1) }
    }
    finally {
        if ($null -ne $record) {
            Stop-OpenMatProcessTree -Record $record
        }
    }

    Wait-ProcessGone -Id $parentPid
    Wait-ProcessGone -Id $childPid
}

try {
    Invoke-SelfTest -Name 'repository and server path guards' -Body {
        $notRepository = [System.IO.Path]::Combine($testRoot, 'not-a-repository')
        [void] [System.IO.Directory]::CreateDirectory($notRepository)
        Assert-OpenMatExitCode -Expected 2 -Body {
            [void] (Resolve-OpenMatRepositoryRoot -Path $notRepository)
        }
        Assert-OpenMatExitCode -Expected 2 -Body {
            [void] (Resolve-OpenMatServerPath -Path ([System.IO.Path]::Combine($testRoot, 'missing.exe')))
        }
    }

    Invoke-SelfTest -Name 'configured and ephemeral loopback URL guard' -Body {
        $uri = Assert-OpenMatServerUrl -Value 'ws://127.0.0.1:49152/kernel'
        Assert-True -Condition ($uri.Port -eq 49152) -Message 'URL guard changed the announced port'
        $defaultPort = Assert-OpenMatServerUrl -Value 'ws://127.0.0.1:80/kernel'
        Assert-True -Condition ((Get-OpenMatLspUrl -ServerUrl $defaultPort).Port -eq 80) -Message 'Default WebSocket port was rejected'
        $lspUri = Get-OpenMatLspUrl -ServerUrl $uri
        Assert-True `
            -Condition ($lspUri.AbsoluteUri -eq 'ws://127.0.0.1:49152/lsp') `
            -Message 'LSP URL derivation did not preserve the loopback listener identity'
        Assert-OpenMatExitCode -Expected 22 -Body {
            [void] (Assert-OpenMatServerUrl -Value 'ws://0.0.0.0:49152/kernel')
        }
        Assert-OpenMatExitCode -Expected 22 -Body {
            [void] (Assert-OpenMatServerUrl -Value 'ws://127.0.0.1:49152/wrong')
        }
    }

    Invoke-SelfTest -Name 'startup timeout cleans recorded process tree' -Body {
        Invoke-TreeCleanupCase -Name 'timeout-tree' -ExpectedExitCode 21
    }

    Invoke-SelfTest -Name 'invalid startup output cleans recorded process tree' -Body {
        Invoke-TreeCleanupCase -Name 'failure-tree' -FirstLine 'not-a-websocket-url' -ExpectedExitCode 22
    }

    Invoke-SelfTest -Name 'frontend helper passes child WebSocket URL and detects readiness' -Body {
        $caseRoot = [System.IO.Path]::Combine($testRoot, 'frontend-helper')
        [void] [System.IO.Directory]::CreateDirectory($caseRoot)
        $fakePnpm = [System.IO.Path]::Combine($caseRoot, 'fake-pnpm.cmd')
        $escape = [char] 27
        $batch = @(
            '@echo off',
            'echo VITE_OPENMAT_WS_URL=%VITE_OPENMAT_WS_URL%',
            'echo VITE_OPENMAT_LSP_URL=%VITE_OPENMAT_LSP_URL%',
            "echo   Local${escape}[22m:${escape}[36m http://127.0.0.1:${escape}[1m54321${escape}[22m/",
            'ping -n 300 127.0.0.1 >nul'
        ) -join "`r`n"
        [System.IO.File]::WriteAllText($fakePnpm, $batch, [System.Text.Encoding]::ASCII)
        $expectedUrl = [System.Uri] 'ws://127.0.0.1:49153/kernel'

        $frontend = $null
        $frontendPid = 0
        try {
            $frontend = Start-OpenMatFrontend `
                -RepositoryRoot $root `
                -LogDirectory $caseRoot `
                -ServerUrl $expectedUrl `
                -PnpmPath $fakePnpm
            $frontendPid = $frontend.Process.Id
            $ready = Wait-OpenMatFrontendReady -Record $frontend -TimeoutSeconds 5
            Assert-True -Condition ($ready -match 'Local:\s+http://127\.0\.0\.1:54321/') -Message 'Frontend helper did not recognize the ready line'
            Assert-True -Condition (-not $ready.Contains($escape)) -Message 'Frontend helper returned ANSI control sequences'
            $environmentLine = $frontend.StdoutLines |
                Where-Object { $_ -like 'VITE_OPENMAT_WS_URL=*' } |
                Select-Object -First 1
            Assert-True `
                -Condition ($environmentLine -eq "VITE_OPENMAT_WS_URL=$($expectedUrl.AbsoluteUri)") `
                -Message 'Frontend child did not receive the announced WebSocket URL'
            $lspEnvironmentLine = $frontend.StdoutLines |
                Where-Object { $_ -like 'VITE_OPENMAT_LSP_URL=*' } |
                Select-Object -First 1
            Assert-True `
                -Condition ($lspEnvironmentLine -eq 'VITE_OPENMAT_LSP_URL=ws://127.0.0.1:49153/lsp') `
                -Message 'Frontend child did not receive the derived LSP WebSocket URL'
        }
        finally {
            if ($null -ne $frontend) {
                Stop-OpenMatProcessTree -Record $frontend
            }
        }
        Wait-ProcessGone -Id $frontendPid
    }

    Invoke-SelfTest -Name 'real server WebSocket lifecycle' -Body {
        $realLogs = [System.IO.Path]::Combine($testRoot, 'real-smoke')
        [void] [System.IO.Directory]::CreateDirectory($realLogs)
        $resolvedServer = if ([string]::IsNullOrWhiteSpace($ServerPath)) {
            Build-OpenMatServer `
                -RepositoryRoot $root `
                -LogDirectory $realLogs `
                -CargoPath $CargoPath `
                -TimeoutSeconds $BuildTimeoutSeconds
        }
        else {
            Resolve-OpenMatServerPath -Path $ServerPath
        }

        $server = $null
        $workspaceRoot = [System.IO.Path]::Combine($realLogs, 'workspace')
        [void] [System.IO.Directory]::CreateDirectory($workspaceRoot)
        try {
            $server = Start-OpenMatServer `
                -Port 0 `
                -ServerPath $resolvedServer `
                -RepositoryRoot $root `
                -LogDirectory $realLogs `
                -WorkspaceRoot $workspaceRoot
            $url = Wait-OpenMatServerUrl -Record $server -TimeoutSeconds 30
            $result = Invoke-OpenMatSmokeLifecycle -ServerUrl $url -TimeoutSeconds $SmokeTimeoutSeconds
            Assert-True -Condition ($result.Value -eq 40) -Message 'Real smoke returned an unexpected inspect value'
            Assert-True -Condition ($result.CloseCode -eq 1000) -Message 'Real smoke did not observe normal close code 1000'
            Assert-True -Condition ($result.Protocol -eq 'openmat-kernel-v1') -Message 'Real smoke did not prefer kernel-v1'

            $fallback = Invoke-OpenMatSmokeLifecycle `
                -ServerUrl $url `
                -TimeoutSeconds $SmokeTimeoutSeconds `
                -ForceV0
            Assert-True -Condition ($fallback.Value -eq 40) -Message 'V0 fallback smoke returned an unexpected inspect value'
            Assert-True -Condition ($fallback.CloseCode -eq 1000) -Message 'V0 fallback smoke did not observe normal close code 1000'
            Assert-True -Condition ($fallback.Protocol -eq 'openmat-kernel-v0') -Message 'V0 fallback smoke negotiated an unexpected protocol'

            $workspace = Invoke-OpenMatWorkspaceSmokeLifecycle `
                -ServerUrl $url `
                -WorkspaceRoot $workspaceRoot `
                -TimeoutSeconds $SmokeTimeoutSeconds
            Assert-True -Condition ($workspace.CreatedCount -eq 2) -Message 'Workspace smoke did not create both entry kinds'

            $lsp = Invoke-OpenMatLspSmokeLifecycle `
                -ServerUrl $url `
                -TimeoutSeconds $SmokeTimeoutSeconds
            Assert-True -Condition ($lsp.CompletionLabel -eq 'calculate') -Message 'LSP smoke did not return real completion'
            Assert-True -Condition ($lsp.DiagnosticCount -gt 0) -Message 'LSP smoke did not update diagnostics'
            Assert-True -Condition ($lsp.CloseCode -eq 1000) -Message 'LSP smoke did not close normally'
        }
        finally {
            if ($null -ne $server) {
                Stop-OpenMatProcessTree -Record $server
            }
        }
    }
}
finally {
    $fullTestRoot = [System.IO.Path]::GetFullPath($testRoot)
    $temporaryPrefix = $temporaryBase.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    $leaf = [System.IO.Path]::GetFileName($fullTestRoot)
    if ($fullTestRoot.StartsWith($temporaryPrefix, [System.StringComparison]::OrdinalIgnoreCase) -and
        $leaf.StartsWith('openmat-dev-selftest-', [System.StringComparison]::Ordinal)) {
        if ([System.IO.Directory]::Exists($fullTestRoot)) {
            [System.IO.Directory]::Delete($fullTestRoot, $true)
        }
    }
    else {
        $failures.Add("Refused to remove unexpected self-test directory: $fullTestRoot")
    }
}

if ($failures.Count -gt 0) {
    [Console]::Error.WriteLine("$($failures.Count) OpenMat dev self-test(s) failed:")
    foreach ($failure in $failures) {
        [Console]::Error.WriteLine("  $failure")
    }
    exit 1
}

[Console]::WriteLine("All $passes OpenMat dev self-tests passed.")
exit 0
