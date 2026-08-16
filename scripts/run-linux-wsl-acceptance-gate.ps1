[CmdletBinding()]
param(
    [ValidateSet('Automated', 'Full')]
    [string]$Mode = 'Automated',
    [string]$WslDistribution = 'Ubuntu',
    [string]$Bash44Path,
    [string]$BashCurrentPath = 'bash',
    [string]$Zsh59Path,
    [switch]$ConfirmDisposableWsl
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$contractPath = Join-Path $repoRoot 'scripts/linux-wsl-acceptance-contract.json'
$contract = [System.IO.File]::ReadAllText($contractPath) | ConvertFrom-Json
$isWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
$sessionName = [Guid]::NewGuid().ToString('N')
$sessionRoot = Join-Path $repoRoot "src-tauri/target/acceptance/linux-wsl/$sessionName"
$canaryA = "gpteasy-linux-wsl-a-$([Guid]::NewGuid().ToString('N'))"
$canaryB = "gpteasy-linux-wsl-b-$([Guid]::NewGuid().ToString('N'))"
$leakDetected = $false
$leakLocation = $null
$processArgumentsScanned = $false
$steps = [System.Collections.Generic.List[object]]::new()
$capturedLogs = [System.Collections.Generic.List[object]]::new()
$prerequisites = [System.Collections.Generic.List[object]]::new()
$shellResults = [ordered]@{}
$realResults = [ordered]@{
    'independent-gnu-linux' = [ordered]@{ id = 'independent-gnu-linux'; status = 'not_run'; detail = 'Requires a native GNU/Linux host.' }
    'wsl2-running-guest' = [ordered]@{ id = 'wsl2-running-guest'; status = 'not_run'; detail = 'Run with -Mode Full and explicit disposable WSL2 confirmation.' }
    'wsl2-stopped-guest' = [ordered]@{ id = 'wsl2-stopped-guest'; status = 'not_run'; detail = 'Run with -Mode Full and explicit disposable WSL2 confirmation.' }
}
$environmentNames = @(
    'GPTEASY_ACCEPTANCE_KEY_A',
    'GPTEASY_ACCEPTANCE_KEY_B',
    'VITE_GPTEASY_ACCEPTANCE_KEY_A',
    'GPTEASY_TEST_MATRIX_SHELL',
    'GPTEASY_TEST_BASH',
    'GPTEASY_TEST_ZSH',
    'GPTEASY_REQUIRE_SHELL_MATRIX',
    'GPTEASY_TEST_WSL_DISTRIBUTION',
    'GPTEASY_RUN_WSL_GUEST_HARNESS',
    'GPTEASY_WSL_TEST_DISTRIBUTION'
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Restore-Environment {
    foreach ($name in $environmentNames) {
        $value = $previousEnvironment[$name]
        if ($null -eq $value) {
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        } else {
            [Environment]::SetEnvironmentVariable($name, [string]$value, 'Process')
        }
    }
}

function Resolve-Program([string]$Name) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $command) {
        return $null
    }
    return $command.Source
}

function Invoke-CapturedProcess([string]$FilePath, [string[]]$Arguments) {
    if ([string]::IsNullOrWhiteSpace($FilePath)) {
        return [pscustomobject]@{
            exitCode = 127
            stdout = ''
            stderr = 'Required program is unavailable.'
        }
    }

    $script:processArgumentsScanned = $true
    if ((Test-ContainsCanary $FilePath) -or
        @($Arguments | Where-Object { Test-ContainsCanary ([string]$_) }).Count -gt 0) {
        $script:leakDetected = $true
        $script:leakLocation = 'process_arguments'
        return [pscustomobject]@{
            exitCode = 125
            stdout = ''
            stderr = 'API key canary detected in child process arguments.'
        }
    }

    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $FilePath
    $info.WorkingDirectory = $repoRoot
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        $info.ArgumentList.Add($argument)
    }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) {
            throw "Could not start $FilePath."
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        return [pscustomobject]@{
            exitCode = $process.ExitCode
            stdout = $stdoutTask.GetAwaiter().GetResult()
            stderr = $stderrTask.GetAwaiter().GetResult()
        }
    } catch {
        return [pscustomobject]@{
            exitCode = 126
            stdout = ''
            stderr = $_.Exception.Message
        }
    } finally {
        $process.Dispose()
    }
}

function Test-ContainsCanary([string]$Value) {
    return $Value.Contains($canaryA, [StringComparison]::Ordinal) -or
        $Value.Contains($canaryB, [StringComparison]::Ordinal)
}

function Add-StepResult(
    [string]$Id,
    [string]$Label,
    [object]$Result,
    [string]$LogName = $Id
) {
    if ((Test-ContainsCanary $Result.stdout) -or (Test-ContainsCanary $Result.stderr)) {
        $script:leakDetected = $true
        $script:leakLocation = $Id
        $steps.Add([ordered]@{ id = $Id; label = $Label; status = 'failed'; exitCode = 1 })
        return $false
    }

    $status = if ($Result.exitCode -eq 0) { 'passed' } else { 'failed' }
    $steps.Add([ordered]@{ id = $Id; label = $Label; status = $status; exitCode = $Result.exitCode })
    $capturedLogs.Add([ordered]@{
        name = $LogName
        stdout = [string]$Result.stdout
        stderr = [string]$Result.stderr
    })
    return $Result.exitCode -eq 0
}

function Invoke-Step([string]$Id, [string]$Label, [string]$Program, [string[]]$Arguments) {
    $result = Invoke-CapturedProcess $Program $Arguments
    return Add-StepResult $Id $Label $result
}

function Invoke-ShellVersion([string]$Executable) {
    if ($isWindowsHost) {
        return Invoke-CapturedProcess (Resolve-Program 'wsl.exe') @(
            '--distribution', $WslDistribution, '--exec', $Executable, '--version'
        )
    }
    return Invoke-CapturedProcess $Executable @('--version')
}

function Invoke-ShellTarget(
    [string]$Id,
    [string]$Label,
    [string]$Shell,
    [string]$Executable,
    [string]$VersionPattern
) {
    if ([string]::IsNullOrWhiteSpace($Executable)) {
        $shellResults[$Id] = [ordered]@{
            id = $Id
            label = $Label
            status = 'failed'
            version = $null
            detail = 'The shell executable path was not provided.'
        }
        return $false
    }

    $versionResult = Invoke-ShellVersion $Executable
    if ((Test-ContainsCanary $versionResult.stdout) -or (Test-ContainsCanary $versionResult.stderr)) {
        $script:leakDetected = $true
        $script:leakLocation = "shell-version-$Id"
        return $false
    }
    $version = (($versionResult.stdout + $versionResult.stderr) -split "`r?`n" | Select-Object -First 1).Trim()
    if ($versionResult.exitCode -ne 0 -or $version -notmatch $VersionPattern) {
        $shellResults[$Id] = [ordered]@{
            id = $Id
            label = $Label
            status = 'failed'
            version = if ([string]::IsNullOrWhiteSpace($version)) { $null } else { $version }
            detail = 'The required shell version is unavailable.'
        }
        $steps.Add([ordered]@{ id = "shell-version-$Id"; label = "$Label version"; status = 'failed'; exitCode = $versionResult.exitCode })
        return $false
    }

    $env:GPTEASY_TEST_MATRIX_SHELL = $Shell
    $env:GPTEASY_REQUIRE_SHELL_MATRIX = '1'
    if ($Shell -eq 'bash') {
        $env:GPTEASY_TEST_BASH = $Executable
    } else {
        $env:GPTEASY_TEST_ZSH = $Executable
    }
    $passed = Invoke-Step "shell-matrix-$Id" "$Label public black-box matrix" $cargo @(
        'test', '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'linux_export',
        'shell_snapshots', '--', '--nocapture', '--test-threads=1'
    )
    $shellResults[$Id] = [ordered]@{
        id = $Id
        label = $Label
        status = if ($passed) { 'passed' } else { 'failed' }
        version = $version
        detail = if ($isWindowsHost) { "WSL2 distribution: $WslDistribution" } else { 'Native GNU/Linux host' }
    }
    return $passed
}

function Set-NotRunShellResult([string]$Id, [string]$Detail) {
    $target = @($contract.shellMatrix | Where-Object { $_.id -eq $Id })[0]
    $shellResults[$Id] = [ordered]@{
        id = $Id
        label = [string]$target.label
        status = 'not_run'
        version = $null
        detail = $Detail
    }
}

$cargo = Resolve-Program 'cargo'
$npx = Resolve-Program $(if ($isWindowsHost) { 'npx.cmd' } else { 'npx' })
$pwsh = Resolve-Program $(if ($isWindowsHost) { 'pwsh.exe' } else { 'pwsh' })
$prerequisites.Add([ordered]@{ id = 'cargo'; available = $null -ne $cargo; value = $cargo })
$prerequisites.Add([ordered]@{ id = 'npx'; available = $null -ne $npx; value = $npx })
$prerequisites.Add([ordered]@{ id = 'powershell'; available = $null -ne $pwsh; value = $pwsh })
$gh = Resolve-Program 'gh'
$prerequisites.Add([ordered]@{ id = 'github-cli'; available = $null -ne $gh; value = $gh })
if ($isWindowsHost) {
    $wsl = Resolve-Program 'wsl.exe'
    $prerequisites.Add([ordered]@{ id = 'wsl2'; available = $null -ne $wsl; value = $WslDistribution })
}

$reportJson = $null
$exitCode = 1
try {
    $env:GPTEASY_ACCEPTANCE_KEY_A = $canaryA
    $env:GPTEASY_ACCEPTANCE_KEY_B = $canaryB
    $env:VITE_GPTEASY_ACCEPTANCE_KEY_A = $canaryA
    $env:GPTEASY_TEST_WSL_DISTRIBUTION = $WslDistribution

    $contractArguments = @(
        '-NoProfile', '-File', 'scripts/test-linux-wsl-contract.ps1', '-RepositoryRoot', $repoRoot
    )
    if ($Mode -eq 'Full') {
        $contractArguments += '-CheckGitHubPrd'
    }
    [void](Invoke-Step 'domain-and-interface-contract' 'Domain, ADR, UI and PRD contract' $pwsh $contractArguments)
    [void](Invoke-Step 'linux-export-generator' 'Linux export generator integration tests' $cargo @(
        'test', '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'linux_export',
        'export_', '--', '--nocapture', '--test-threads=1'
    ))

    $currentTarget = @($contract.shellMatrix | Where-Object { $_.id -eq 'gnu-bash-current' })[0]
    [void](Invoke-ShellTarget 'gnu-bash-current' $currentTarget.label 'bash' $BashCurrentPath $currentTarget.versionPattern)
    Set-NotRunShellResult 'gnu-bash-4.4' 'Run with -Mode Full and provide -Bash44Path.'
    Set-NotRunShellResult 'zsh-5.9' 'Run with -Mode Full and provide -Zsh59Path.'

    [void](Invoke-Step 'wsl-shared-protocol' 'WSL2 shared protocol and lifecycle tests' $cargo @(
        'test', '--manifest-path', 'src-tauri/Cargo.toml', '--lib', 'wsl::tests',
        '--', '--nocapture', '--test-threads=1'
    ))
    [void](Invoke-Step 'sqlite-schema-and-saga' 'SQLite schema and migration tests' $cargo @(
        'test', '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'state_store',
        '--', '--nocapture', '--test-threads=1'
    ))
    [void](Invoke-Step 'provider-deletion-and-credential-cleanup' 'Provider deletion and recovery integration tests' $cargo @(
        'test', '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'provider_workflow',
        '--', '--nocapture', '--test-threads=1'
    ))
    [void](Invoke-Step 'react-linux-wsl-workflows' 'React Linux and WSL2 user workflows' $npx @(
        '--no-install', 'vitest', 'run', 'src/App.test.tsx'
    ))

    if ($Mode -eq 'Full') {
        $bash44Target = @($contract.shellMatrix | Where-Object { $_.id -eq 'gnu-bash-4.4' })[0]
        $zshTarget = @($contract.shellMatrix | Where-Object { $_.id -eq 'zsh-5.9' })[0]
        [void](Invoke-ShellTarget 'gnu-bash-4.4' $bash44Target.label 'bash' $Bash44Path $bash44Target.versionPattern)
        [void](Invoke-ShellTarget 'zsh-5.9' $zshTarget.label 'zsh' $Zsh59Path $zshTarget.versionPattern)

        if ($isWindowsHost) {
            if (-not $ConfirmDisposableWsl) {
                foreach ($id in @('wsl2-running-guest', 'wsl2-stopped-guest')) {
                    $realResults[$id] = [ordered]@{
                        id = $id
                        status = 'blocked'
                        detail = 'Full WSL2 harness requires -ConfirmDisposableWsl.'
                    }
                }
            } else {
                $env:GPTEASY_RUN_WSL_GUEST_HARNESS = '1'
                $env:GPTEASY_WSL_TEST_DISTRIBUTION = $WslDistribution
                $runningPassed = Invoke-Step 'wsl2-running-guest' 'Running WSL2 real guest harness' $cargo @(
                    'test', '--manifest-path', 'src-tauri/Cargo.toml', '--features', 'wsl-guest-harness',
                    '--test', 'wsl_guest_harness', 'running_guest_harness_preserves_auth',
                    '--', '--ignored', '--nocapture', '--test-threads=1'
                )
                $realResults['wsl2-running-guest'] = [ordered]@{
                    id = 'wsl2-running-guest'
                    status = if ($runningPassed) { 'passed' } else { 'failed' }
                    detail = "WSL2 distribution: $WslDistribution"
                }
                $stoppedPassed = Invoke-Step 'wsl2-stopped-guest' 'Stopped WSL2 real guest harness' $cargo @(
                    'test', '--manifest-path', 'src-tauri/Cargo.toml', '--features', 'wsl-guest-harness',
                    '--test', 'wsl_guest_harness', 'stopped_guest_harness_authorizes_start',
                    '--', '--ignored', '--nocapture', '--test-threads=1'
                )
                $realResults['wsl2-stopped-guest'] = [ordered]@{
                    id = 'wsl2-stopped-guest'
                    status = if ($stoppedPassed) { 'passed' } else { 'failed' }
                    detail = "WSL2 distribution: $WslDistribution"
                }
            }
        } else {
            $allShellsPassed = @($shellResults.Values | Where-Object { $_.status -ne 'passed' }).Count -eq 0
            $realResults['independent-gnu-linux'] = [ordered]@{
                id = 'independent-gnu-linux'
                status = if ($allShellsPassed) { 'passed' } else { 'failed' }
                detail = 'Native GNU/Linux shell matrix.'
            }
        }
    }

    if ($leakDetected) {
        [Console]::Error.WriteLine("Linux/WSL2 acceptance gate detected an API key canary leak in $leakLocation; no logs or evidence were persisted.")
        $exitCode = 1
    } else {
        $automated = foreach ($matrix in @($contract.automatedMatrix)) {
            $step = @($steps | Where-Object { $_.id -eq $matrix.id }) | Select-Object -Last 1
            $status = if ($matrix.id -eq 'linux-shell-public-behavior') {
                [string]$shellResults['gnu-bash-current'].status
            } elseif ($null -eq $step) {
                'not_run'
            } else {
                [string]$step.status
            }
            [ordered]@{
                id = [string]$matrix.id
                coverage = [string]$matrix.coverage
                status = $status
            }
        }
        $failedSteps = @($steps | Where-Object { $_.status -ne 'passed' })
        $failedAutomated = @($automated | Where-Object { $_.status -ne 'passed' })
        $fullHostFailures = if ($Mode -eq 'Full') {
            @($shellResults.Values | Where-Object { $_.status -ne 'passed' }).Count +
                $(if ($isWindowsHost) { @($realResults.Values | Where-Object { $_.id -like 'wsl2-*' -and $_.status -ne 'passed' }).Count } else { @($realResults.Values | Where-Object { $_.id -eq 'independent-gnu-linux' -and $_.status -ne 'passed' }).Count })
        } else {
            0
        }
        $passed = $failedSteps.Count -eq 0 -and $failedAutomated.Count -eq 0 -and $fullHostFailures -eq 0
        $scannedSurfaces = foreach ($surface in @($contract.canaryScannedSurfaces)) {
            $verifiers = @($contract.canarySurfaceVerifiers.PSObject.Properties[[string]$surface].Value)
            $verified = $true
            foreach ($verifier in $verifiers) {
                if ($verifier -eq '__runner__') {
                    continue
                }
                if ($verifier -eq '__runner_process_arguments__') {
                    if (-not $processArgumentsScanned) {
                        $verified = $false
                        break
                    }
                    continue
                }
                $matrixResult = @($automated | Where-Object { $_.id -eq $verifier }) | Select-Object -Last 1
                if ($null -eq $matrixResult -or $matrixResult.status -ne 'passed') {
                    $verified = $false
                    break
                }
            }
            if ($verified) {
                [string]$surface
            }
        }
        if (@($scannedSurfaces).Count -ne @($contract.canaryScannedSurfaces).Count) {
            $passed = $false
        }
        $report = [ordered]@{
            schemaVersion = 1
            issue = 35
            parentIssue = 29
            mode = $Mode.ToLowerInvariant()
            passed = $passed
            gitCommit = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()
            platform = [ordered]@{
                os = if ($isWindowsHost) { 'windows' } else { 'linux' }
                architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
            }
            automatedMatrix = @($automated)
            shellMatrix = @($contract.shellMatrix | ForEach-Object { $shellResults[[string]$_.id] })
            realEnvironmentGates = @($contract.realEnvironmentGates | ForEach-Object { $realResults[[string]$_.id] })
            unexecutedRealEnvironmentGates = @($realResults.Values | Where-Object { $_.status -in @('not_run', 'blocked') } | ForEach-Object { $_.id })
            platformPrerequisites = @($prerequisites)
            githubPrdCheck = if ($Mode -eq 'Full') { 'checked' } else { 'not_run' }
            evidenceDirectory = $sessionRoot
            leakScan = [ordered]@{
                leaked = $false
                scannedSurfaces = @($scannedSurfaces)
            }
            windowsIssue28Gate = [ordered]@{
                command = 'npm run acceptance'
                status = 'not_run'
                detail = 'Issue #28 remains an independent Windows gate; use npm run acceptance:all for the side-by-side summary.'
            }
            steps = @($steps)
        }
        $reportJson = $report | ConvertTo-Json -Depth 12
        if (Test-ContainsCanary $reportJson) {
            [Console]::Error.WriteLine('Linux/WSL2 acceptance report contained an API key canary; no logs or evidence were persisted.')
            $exitCode = 1
        } else {
            New-Item -ItemType Directory -Path $sessionRoot -Force | Out-Null
            foreach ($log in $capturedLogs) {
                $safeName = ([string]$log.name) -replace '[^a-zA-Z0-9._-]', '-'
                Write-Utf8NoBom (Join-Path $sessionRoot "$safeName.log") ("STDOUT`n$($log.stdout)`nSTDERR`n$($log.stderr)")
            }
            Write-Utf8NoBom (Join-Path $sessionRoot 'evidence.json') $reportJson
            Write-Output $reportJson
            $exitCode = if ($passed) { 0 } else { 1 }
        }
    }
} finally {
    Restore-Environment
}

exit $exitCode
