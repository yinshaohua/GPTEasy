[CmdletBinding()]
param(
    [ValidateSet('Automated', 'Full')]
    [string]$LinuxWslMode = 'Full',
    [string]$WslDistribution = 'Ubuntu',
    [string]$Bash44Path,
    [string]$BashCurrentPath = 'bash',
    [string]$Zsh59Path,
    [switch]$ConfirmDisposableWsl
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

function Invoke-Captured([string]$Program, [string[]]$Arguments) {
    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Program
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
        [void]$process.Start()
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        return [pscustomobject]@{
            exitCode = $process.ExitCode
            stdout = $stdoutTask.GetAwaiter().GetResult()
            stderr = $stderrTask.GetAwaiter().GetResult()
        }
    } finally {
        $process.Dispose()
    }
}

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw 'The side-by-side Issue #28 and #35 summary requires a Windows host.'
}

$windows = Invoke-Captured 'powershell.exe' @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/run-acceptance-gate.ps1'
)
$linuxArguments = @(
    '-NoProfile', '-File', 'scripts/run-linux-wsl-acceptance-gate.ps1',
    '-Mode', $LinuxWslMode, '-WslDistribution', $WslDistribution,
    '-BashCurrentPath', $BashCurrentPath
)
if (-not [string]::IsNullOrWhiteSpace($Bash44Path)) {
    $linuxArguments += @('-Bash44Path', $Bash44Path)
}
if (-not [string]::IsNullOrWhiteSpace($Zsh59Path)) {
    $linuxArguments += @('-Zsh59Path', $Zsh59Path)
}
if ($ConfirmDisposableWsl) {
    $linuxArguments += '-ConfirmDisposableWsl'
}
$linuxWsl = Invoke-Captured 'pwsh.exe' $linuxArguments

function Convert-ChildSummary([string]$Value) {
    try {
        return $Value | ConvertFrom-Json
    } catch {
        return $null
    }
}

$windowsSummary = Convert-ChildSummary $windows.stdout
$linuxWslSummary = Convert-ChildSummary $linuxWsl.stdout

$gates = @(
    [ordered]@{
        issue = 28
        name = 'Windows x64 current-user acceptance'
        status = if ($windows.exitCode -eq 0) { 'passed' } else { 'failed' }
        summary = $windowsSummary
        error = if ($windows.exitCode -eq 0) { $null } elseif ($null -ne $windowsSummary) { 'Issue #28 gate failed; see its summary.' } else { $windows.stderr.Trim() }
    },
    [ordered]@{
        issue = 35
        name = 'Linux and WSL2 automated acceptance'
        status = if ($linuxWsl.exitCode -eq 0) { 'passed' } else { 'failed' }
        summary = $linuxWslSummary
        error = if ($linuxWsl.exitCode -eq 0) { $null } elseif ($null -ne $linuxWslSummary) { 'Issue #35 gate failed; see its steps and evidenceDirectory.' } else { $linuxWsl.stderr.Trim() }
    }
)
$report = [ordered]@{
    schemaVersion = 1
    passed = @($gates | Where-Object { $_.status -ne 'passed' }).Count -eq 0
    gates = $gates
}
$report | ConvertTo-Json -Depth 16
if (-not $report.passed) {
    exit 1
}
