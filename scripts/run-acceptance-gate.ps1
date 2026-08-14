[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$isWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
if (-not $isWindowsHost) {
    throw 'The Issue #28 acceptance gate is a Windows x64 gate.'
}
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'The Issue #28 acceptance gate requires an x64 process.'
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$acceptanceRoot = Join-Path $repoRoot 'src-tauri\target\acceptance'
$sessionRoot = Join-Path $acceptanceRoot ([Guid]::NewGuid().ToString('N'))
$pendingEvidence = Join-Path $sessionRoot 'evidence.pending.json'
$evidence = Join-Path $sessionRoot 'evidence.json'
$testLog = Join-Path $sessionRoot 'test.log'
$frontendLog = Join-Path $sessionRoot 'frontend.log'
$summaryPath = Join-Path $sessionRoot 'summary.json'
New-Item -ItemType Directory -Path $sessionRoot -Force | Out-Null

$keyA = "gpteasy-acceptance-a-$([Guid]::NewGuid().ToString('N'))"
$keyB = "gpteasy-acceptance-b-$([Guid]::NewGuid().ToString('N'))"
$previousKeyA = [Environment]::GetEnvironmentVariable('GPTEASY_ACCEPTANCE_KEY_A', 'Process')
$previousKeyB = [Environment]::GetEnvironmentVariable('GPTEASY_ACCEPTANCE_KEY_B', 'Process')
$previousEvidence = [Environment]::GetEnvironmentVariable('GPTEASY_ACCEPTANCE_EVIDENCE_PATH', 'Process')
$previousViteKey = [Environment]::GetEnvironmentVariable('VITE_GPTEASY_ACCEPTANCE_KEY_A', 'Process')

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Restore-EnvironmentVariable([string]$Name, [string]$Value) {
    if ($null -eq $Value) {
        Remove-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
    } else {
        Set-Item -LiteralPath "Env:$Name" -Value $Value
    }
}

function Test-FileContains([string]$Path, [string[]]$Needles) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }
    $content = [System.IO.File]::ReadAllText($Path)
    foreach ($needle in $Needles) {
        if ($content.Contains($needle)) {
            return $true
        }
    }
    return $false
}

try {
    $env:GPTEASY_ACCEPTANCE_KEY_A = $keyA
    $env:GPTEASY_ACCEPTANCE_KEY_B = $keyB
    $env:GPTEASY_ACCEPTANCE_EVIDENCE_PATH = $pendingEvidence
    $env:VITE_GPTEASY_ACCEPTANCE_KEY_A = $keyA

    Push-Location $repoRoot
    try {
        $previousErrorAction = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $testOutput = (& cargo test --manifest-path src-tauri/Cargo.toml --tests -- --nocapture --test-threads=1 2>&1 | Out-String)
        $testExitCode = $LASTEXITCODE
        $frontendOutput = (& npx --no-install vitest run src/App.test.tsx 2>&1 | Out-String)
        $frontendExitCode = $LASTEXITCODE
        $ErrorActionPreference = $previousErrorAction
    } finally {
        if ($null -ne $previousErrorAction) {
            $ErrorActionPreference = $previousErrorAction
        }
        Pop-Location
    }

    $evidenceContainsCanary = Test-FileContains $pendingEvidence @($keyA, $keyB)
    $outputContainsCanary = $testOutput.Contains($keyA) -or $testOutput.Contains($keyB) -or
        $frontendOutput.Contains($keyA) -or $frontendOutput.Contains($keyB)
    if ($evidenceContainsCanary -or $outputContainsCanary) {
        Remove-Item -LiteralPath $pendingEvidence -Force -ErrorAction SilentlyContinue
        throw 'Acceptance output contained an API key canary; no output was persisted.'
    }

    Write-Utf8NoBom $testLog $testOutput
    Write-Utf8NoBom $frontendLog $frontendOutput
    if ($testExitCode -ne 0) {
        throw "Acceptance gate failed with exit code $testExitCode. See $testLog"
    }
    if ($frontendExitCode -ne 0) {
        throw "Frontend leak gate failed with exit code $frontendExitCode. See $frontendLog"
    }
    if (-not (Test-Path -LiteralPath $pendingEvidence -PathType Leaf)) {
        throw 'Acceptance test did not produce redacted evidence.'
    }

    $report = Get-Content -LiteralPath $pendingEvidence -Raw | ConvertFrom-Json
    if ($report.passed -ne $report.total -or $report.leakScan.leaked) {
        throw 'Acceptance evidence did not report a clean, complete matrix.'
    }
    $report.leakScan.scannedSurfaces = @(
        $report.leakScan.scannedSurfaces
        'frontend_test_log'
        'notification_dialog'
        'screenshot_assist_dom'
    ) | Sort-Object -Unique
    Write-Utf8NoBom $pendingEvidence ($report | ConvertTo-Json -Depth 20)
    Move-Item -LiteralPath $pendingEvidence -Destination $evidence

    $summary = [ordered]@{
        platform = 'windows-x64-current-user'
        passed = $report.passed
        total = $report.total
        frontend_leak_gate = 'passed'
        api_key_canary_leak = $false
        evidence = $evidence
        log = $testLog
        frontend_log = $frontendLog
    }
    Write-Utf8NoBom $summaryPath ($summary | ConvertTo-Json -Depth 8)
    Write-Output (Get-Content -LiteralPath $summaryPath -Raw)
} finally {
    Restore-EnvironmentVariable 'GPTEASY_ACCEPTANCE_KEY_A' $previousKeyA
    Restore-EnvironmentVariable 'GPTEASY_ACCEPTANCE_KEY_B' $previousKeyB
    Restore-EnvironmentVariable 'GPTEASY_ACCEPTANCE_EVIDENCE_PATH' $previousEvidence
    Restore-EnvironmentVariable 'VITE_GPTEASY_ACCEPTANCE_KEY_A' $previousViteKey
}
