[CmdletBinding()]
param(
    [ValidateSet('Automated', 'Full')]
    [string]$Mode = 'Automated',
    [string]$WslDistribution = 'Ubuntu',
    [string]$Bash44Path,
    [string]$BashCurrentPath = 'bash',
    [string]$Zsh59Path,
    [string]$CodexPath,
    [string]$SourceCommit,
    [switch]$ConfirmDisposableWsl
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$contractPath = Join-Path $repoRoot 'scripts/linux-wsl-acceptance-contract.json'
$contract = [System.IO.File]::ReadAllText($contractPath) | ConvertFrom-Json
$isWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
$isLinuxHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Linux
)
if (-not $isWindowsHost -and -not $isLinuxHost) {
    throw 'Linux/WSL2 acceptance only supports Windows and native GNU/Linux hosts.'
}
if ($Mode -eq 'Full' -and $isWindowsHost -and -not $ConfirmDisposableWsl) {
    throw 'Full WSL2 acceptance requires -ConfirmDisposableWsl before any guest command runs.'
}
$sessionName = [Guid]::NewGuid().ToString('N')
$sessionRoot = Join-Path $repoRoot "src-tauri/target/acceptance/linux-wsl/$sessionName"
$canaryA = "gpteasy-linux-wsl-a-$([Guid]::NewGuid().ToString('N'))"
$canaryB = "gpteasy-linux-wsl-b-$([Guid]::NewGuid().ToString('N'))"
$leakDetected = $false
$leakLocation = $null
$processArgumentsScanned = $false
$steps = [System.Collections.Generic.List[object]]::new()
$capturedLogs = [System.Collections.Generic.List[object]]::new()
$applicationLogStepsScanned = [System.Collections.Generic.HashSet[string]]::new(
    [StringComparer]::Ordinal
)
$prerequisites = [System.Collections.Generic.List[object]]::new()
$shellResults = [ordered]@{}
$realResults = [ordered]@{
    'independent-gnu-linux' = [ordered]@{ id = 'independent-gnu-linux'; status = 'not_run'; detail = 'Requires a native GNU/Linux host.' }
    'wsl2-running-guest' = [ordered]@{ id = 'wsl2-running-guest'; status = 'not_run'; detail = 'Run with -Mode Full and explicit disposable WSL2 confirmation.' }
    'wsl2-stopped-guest' = [ordered]@{ id = 'wsl2-stopped-guest'; status = 'not_run'; detail = 'Run with -Mode Full and explicit disposable WSL2 confirmation.' }
}
$realCodexResult = [ordered]@{
    status = 'not_run'
    version = $null
    verificationMethod = [string]$contract.realCodex.verificationMethod
}
$gitCommit = if ([string]::IsNullOrWhiteSpace($SourceCommit)) {
    (& git -C $repoRoot rev-parse HEAD 2>$null | Out-String).Trim()
} elseif ($SourceCommit -match '^[0-9a-fA-F]{40}$') {
    $SourceCommit.ToLowerInvariant()
} else {
    throw 'SourceCommit must be a 40-character Git commit SHA.'
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
    'GPTEASY_WSL_TEST_DISTRIBUTION',
    'GPTEASY_RUN_REAL_CODEX_ACCEPTANCE',
    'GPTEASY_REAL_CODEX',
    'NO_PROXY',
    'no_proxy'
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
    if (@($contract.applicationLogSteps) -contains $Id) {
        [void]$script:applicationLogStepsScanned.Add($Id)
    }
    return $Result.exitCode -eq 0
}

function Invoke-Step([string]$Id, [string]$Label, [string]$Program, [string[]]$Arguments) {
    $result = Invoke-CapturedProcess $Program $Arguments
    return Add-StepResult $Id $Label $result
}

function Invoke-CargoStep([string]$Id, [string]$Label, [string[]]$Arguments) {
    $cargoArguments = @('test')
    if ($isLinuxHost) {
        $cargoArguments += @('--features', 'native-linux-acceptance')
    }
    $cargoArguments += $Arguments
    return Invoke-Step $Id $Label $cargo $cargoArguments
}

function Invoke-ShellVersion([string]$Executable) {
    if ($isWindowsHost) {
        return Invoke-CapturedProcess (Resolve-Program 'wsl.exe') @(
            '--distribution', $WslDistribution, '--exec', $Executable, '--version'
        )
    }
    return Invoke-CapturedProcess $Executable @('--version')
}

function Invoke-CodexVersion([string]$Executable) {
    if ([string]::IsNullOrWhiteSpace($Executable)) {
        return [pscustomobject]@{
            exitCode = 127
            stdout = ''
            stderr = 'The real Codex executable path was not provided.'
        }
    }
    if ($isWindowsHost) {
        return Invoke-CapturedProcess (Resolve-Program 'wsl.exe') @(
            '--distribution', $WslDistribution, '--exec', $Executable, '--version'
        )
    }
    return Invoke-CapturedProcess $Executable @('--version')
}

function Test-NativeLinuxHost {
    if ($isWindowsHost) {
        return $true
    }
    if (-not $isLinuxHost) {
        return $false
    }
    $osReleasePath = '/etc/os-release'
    $kernelPath = '/proc/sys/kernel/osrelease'
    if (-not (Test-Path -LiteralPath $osReleasePath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $kernelPath -PathType Leaf)) {
        return $false
    }
    $idLine = [System.IO.File]::ReadAllLines($osReleasePath) |
        Where-Object { $_.StartsWith('ID=', [StringComparison]::Ordinal) } |
        Select-Object -First 1
    if ($null -eq $idLine) {
        return $false
    }
    $distributionId = $idLine.Substring(3).Trim().Trim('"').Trim("'")
    if (-not $distributionId.Equals(
        [string]$contract.nativeLinux.requiredOsReleaseId,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        return $false
    }
    $release = [System.IO.File]::ReadAllText($kernelPath).ToLowerInvariant()
    foreach ($pattern in @($contract.nativeLinux.rejectedKernelPatterns)) {
        if ($release.Contains(([string]$pattern).ToLowerInvariant(), [StringComparison]::Ordinal)) {
            return $false
        }
    }
    return $true
}

function Wait-WslDistributionStopped([int]$TimeoutSeconds = 60) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $result = Invoke-CapturedProcess (Resolve-Program 'wsl.exe') @('--list', '--running', '--quiet')
        if ($result.exitCode -ne 0) {
            return $false
        }
        $running = ([string]$result.stdout).Replace("`0", '') -split "`r?`n" |
            ForEach-Object { $_.Trim() } |
            Where-Object { $_ }
        if (-not @($running | Where-Object { $_.Equals($WslDistribution, [StringComparison]::OrdinalIgnoreCase) }).Count) {
            return $true
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Invoke-RealCodexTarget {
    $versionResult = Invoke-CodexVersion $CodexPath
    if ((Test-ContainsCanary $versionResult.stdout) -or (Test-ContainsCanary $versionResult.stderr)) {
        $script:leakDetected = $true
        $script:leakLocation = 'real-codex-version'
        $script:realCodexResult.status = 'failed'
        return $false
    }
    $version = (($versionResult.stdout + $versionResult.stderr) -split "`r?`n" | Select-Object -First 1).Trim()
    $minimum = [version]([string]$contract.realCodex.minimumVersion)
    if ($versionResult.exitCode -ne 0 -or $version -notmatch '^codex-cli (\d+\.\d+\.\d+)') {
        $script:realCodexResult.status = 'failed'
        $steps.Add([ordered]@{ id = 'real-codex-config-read'; label = 'Real Codex config/read'; status = 'failed'; exitCode = $versionResult.exitCode })
        return $false
    }
    if ([version]$Matches[1] -lt $minimum) {
        $script:realCodexResult.status = 'failed'
        $script:realCodexResult.version = $version
        $steps.Add([ordered]@{ id = 'real-codex-config-read'; label = 'Real Codex config/read'; status = 'failed'; exitCode = 1 })
        return $false
    }

    $env:GPTEASY_RUN_REAL_CODEX_ACCEPTANCE = '1'
    $env:GPTEASY_REAL_CODEX = $CodexPath
    $env:GPTEASY_TEST_BASH = $BashCurrentPath
    $passed = Invoke-CargoStep 'real-codex-config-read' 'Real Codex app-server config/read' @(
        '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'real_linux_codex',
        'exported_provider_is_effective_in_real_codex_cli', '--', '--ignored', '--nocapture', '--test-threads=1'
    )
    $script:realCodexResult.status = if ($passed) { 'passed' } else { 'failed' }
    $script:realCodexResult.version = $version
    return $passed
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
    $passed = Invoke-CargoStep "shell-matrix-$Id" "$Label public black-box matrix" @(
        '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'linux_export',
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
$prerequisites.Add([ordered]@{ id = 'real-codex'; available = -not [string]::IsNullOrWhiteSpace($CodexPath); value = $CodexPath })

if ($Mode -eq 'Full' -and -not (Test-NativeLinuxHost)) {
    throw 'Independent GNU/Linux acceptance requires Ubuntu on a non-WSL Linux kernel.'
}

$reportJson = $null
$exitCode = 1
try {
    $env:GPTEASY_ACCEPTANCE_KEY_A = $canaryA
    $env:GPTEASY_ACCEPTANCE_KEY_B = $canaryB
    $env:VITE_GPTEASY_ACCEPTANCE_KEY_A = $canaryA
    $env:GPTEASY_TEST_WSL_DISTRIBUTION = $WslDistribution
    $env:NO_PROXY = (@($env:NO_PROXY -split ',' | Where-Object { $_ }) + @('127.0.0.1', 'localhost', '::1')) -join ','
    $env:no_proxy = (@($env:no_proxy -split ',' | Where-Object { $_ }) + @('127.0.0.1', 'localhost', '::1')) -join ','

    $contractArguments = @(
        '-NoProfile', '-File', 'scripts/test-linux-wsl-contract.ps1', '-RepositoryRoot', $repoRoot
    )
    if ($Mode -eq 'Full') {
        $contractArguments += '-CheckGitHubPrd'
    }
    [void](Invoke-Step 'domain-and-interface-contract' 'Domain, ADR, UI and PRD contract' $pwsh $contractArguments)
    [void](Invoke-CargoStep 'linux-export-generator' 'Linux export generator integration tests' @(
        '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'linux_export',
        'export_', '--', '--nocapture', '--test-threads=1'
    ))

    $currentTarget = @($contract.shellMatrix | Where-Object { $_.id -eq 'gnu-bash-current' })[0]
    [void](Invoke-ShellTarget 'gnu-bash-current' $currentTarget.label 'bash' $BashCurrentPath $currentTarget.versionPattern)
    Set-NotRunShellResult 'gnu-bash-4.4' 'Run with -Mode Full and provide -Bash44Path.'
    Set-NotRunShellResult 'zsh-5.9' 'Run with -Mode Full and provide -Zsh59Path.'

    [void](Invoke-CargoStep 'wsl-shared-protocol' 'WSL2 shared protocol and lifecycle tests' @(
        '--manifest-path', 'src-tauri/Cargo.toml', '--lib', 'wsl::tests',
        '--', '--nocapture', '--test-threads=1'
    ))
    [void](Invoke-CargoStep 'sqlite-schema-and-saga' 'SQLite schema and migration tests' @(
        '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'state_store',
        '--', '--nocapture', '--test-threads=1'
    ))
    [void](Invoke-CargoStep 'provider-deletion-and-credential-cleanup' 'Provider deletion and recovery integration tests' @(
        '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'provider_workflow',
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
        [void](Invoke-RealCodexTarget)

        if ($isWindowsHost) {
            $env:GPTEASY_RUN_WSL_GUEST_HARNESS = '1'
            $env:GPTEASY_WSL_TEST_DISTRIBUTION = $WslDistribution
            $runningPassed = Invoke-CargoStep 'wsl2-running-guest' 'Running WSL2 real guest harness' @(
                '--manifest-path', 'src-tauri/Cargo.toml', '--features', 'wsl-guest-harness',
                '--test', 'wsl_guest_harness', 'running_guest_',
                '--', '--ignored', '--nocapture', '--test-threads=1'
            )
            $realResults['wsl2-running-guest'] = [ordered]@{
                id = 'wsl2-running-guest'
                status = if ($runningPassed) { 'passed' } else { 'failed' }
                detail = "WSL2 distribution: $WslDistribution"
            }
            $stoppedReady = Wait-WslDistributionStopped
            $steps.Add([ordered]@{
                id = 'wsl2-actual-stopped-precondition'
                label = 'Actual Stopped WSL2 precondition'
                status = if ($stoppedReady) { 'passed' } else { 'failed' }
                exitCode = if ($stoppedReady) { 0 } else { 1 }
            })
            $stoppedPassed = $false
            if ($stoppedReady) {
                $stoppedPassed = Invoke-CargoStep 'wsl2-stopped-guest' 'Stopped WSL2 real guest harness' @(
                    '--manifest-path', 'src-tauri/Cargo.toml', '--features', 'wsl-guest-harness',
                    '--test', 'wsl_guest_harness', 'stopped_guest_harness_authorizes_start',
                    '--', '--ignored', '--nocapture', '--test-threads=1'
                )
            }
            $stoppedAfter = Wait-WslDistributionStopped
            $steps.Add([ordered]@{
                id = 'wsl2-actual-stopped-final'
                label = 'Actual Stopped WSL2 final state'
                status = if ($stoppedAfter) { 'passed' } else { 'failed' }
                exitCode = if ($stoppedAfter) { 0 } else { 1 }
            })
            $stoppedPassed = $stoppedPassed -and $stoppedAfter
            $realResults['wsl2-stopped-guest'] = [ordered]@{
                id = 'wsl2-stopped-guest'
                status = if ($stoppedPassed) { 'passed' } else { 'failed' }
                detail = "WSL2 distribution: $WslDistribution"
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
                $(if ($realCodexResult.status -eq 'passed') { 0 } else { 1 }) +
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
                if ($verifier -eq '__application_logs__') {
                    if ($applicationLogStepsScanned.Count -eq 0) {
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
            gitCommit = $gitCommit
            platform = [ordered]@{
                os = if ($isWindowsHost) { 'windows' } else { 'linux' }
                architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
            }
            automatedMatrix = @($automated)
            shellMatrix = @($contract.shellMatrix | ForEach-Object { $shellResults[[string]$_.id] })
            realEnvironmentGates = @($contract.realEnvironmentGates | ForEach-Object { $realResults[[string]$_.id] })
            realCodex = $realCodexResult
            unexecutedRealEnvironmentGates = @($realResults.Values | Where-Object { $_.status -in @('not_run', 'blocked') } | ForEach-Object { $_.id })
            platformPrerequisites = @($prerequisites)
            githubPrdCheck = if ($Mode -eq 'Full') { 'checked' } else { 'not_run' }
            evidenceDirectory = $sessionRoot
            leakScan = [ordered]@{
                leaked = $false
                scannedSurfaces = @($scannedSurfaces)
                applicationLogSteps = @($applicationLogStepsScanned | Sort-Object)
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
