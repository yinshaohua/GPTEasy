param(
    [switch]$SkipLive
)

$ErrorActionPreference = 'Stop'

$spike = Split-Path -Parent $MyInvocation.MyCommand.Path
$run = Join-Path $spike '.run'
$work = Join-Path $run 'workspace'
$evidence = Join-Path $run 'evidence'
$skillRoot = Split-Path -Parent (Split-Path -Parent $spike)
$secretPath = Join-Path $skillRoot '.secrets/provider.json'
$env:HTTP_PROXY = 'http://127.0.0.1:7897'
$env:HTTPS_PROXY = 'http://127.0.0.1:7897'

function Remove-SafeDirectory([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $resolvedRoot = [System.IO.Path]::GetFullPath($run).TrimEnd('\')
    $resolvedTarget = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    if (-not $resolvedTarget.StartsWith($resolvedRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing recursive delete outside spike run directory: $resolvedTarget"
    }
    Remove-Item -LiteralPath $resolvedTarget -Recurse -Force
}

function Test-BytesContain([byte[]]$Haystack, [byte[]]$Needle) {
    if ($Needle.Length -eq 0 -or $Haystack.Length -lt $Needle.Length) {
        return $false
    }
    for ($i = 0; $i -le $Haystack.Length - $Needle.Length; $i++) {
        $match = $true
        for ($j = 0; $j -lt $Needle.Length; $j++) {
            if ($Haystack[$i + $j] -ne $Needle[$j]) {
                $match = $false
                break
            }
        }
        if ($match) {
            return $true
        }
    }
    $false
}

New-Item -ItemType Directory -Path $run -Force | Out-Null
Remove-SafeDirectory $work
Remove-SafeDirectory $evidence
New-Item -ItemType Directory -Path $work, $evidence -Force | Out-Null

Push-Location (Join-Path $spike 'src-tauri')
try {
    cargo test
    cargo build --release --bin spike-012-matrix
    $matrixExe = Join-Path $PWD 'target/release/spike-012-matrix.exe'
    & $matrixExe matrix (Join-Path $work 'deterministic') $evidence
    if ($LASTEXITCODE -ne 0) {
        throw "Deterministic matrix failed with exit code $LASTEXITCODE"
    }

    $liveExecuted = $false
    if (-not $SkipLive -and (Test-Path -LiteralPath $secretPath)) {
        git check-ignore --quiet -- $secretPath
        if ($LASTEXITCODE -ne 0) {
            throw 'Provider secret file is not ignored by Git.'
        }
        & $matrixExe live $secretPath (Join-Path $work 'live') (Join-Path $evidence 'live')
        if ($LASTEXITCODE -ne 0) {
            throw "Live pipeline failed with exit code $LASTEXITCODE"
        }
        $liveExecuted = $true
    }
}
finally {
    Pop-Location
}

Push-Location $spike
try {
    npm install
    npm run tauri build -- --no-bundle
}
finally {
    Pop-Location
}

$secretLeak = $false
if ($liveExecuted) {
    $secret = (Get-Content -LiteralPath $secretPath -Raw | ConvertFrom-Json).api_key
    $needle = [System.Text.Encoding]::UTF8.GetBytes($secret)
    foreach ($file in Get-ChildItem -LiteralPath $evidence -Recurse -File) {
        if (Test-BytesContain ([System.IO.File]::ReadAllBytes($file.FullName)) $needle) {
            $secretLeak = $true
            break
        }
    }
}
if ($secretLeak) {
    throw 'API Key leaked into exported evidence.'
}

$deterministic = Get-Content -LiteralPath (Join-Path $evidence 'summary.json') -Raw | ConvertFrom-Json
$combined = [ordered]@{
    generated_at = [DateTimeOffset]::UtcNow.ToString('o')
    deterministic_passed = $deterministic.passed
    deterministic_total = $deterministic.total
    live_executed = $liveExecuted
    live_validated = if ($liveExecuted) {
        (Get-Content -LiteralPath (Join-Path $evidence 'live/live-summary.json') -Raw | ConvertFrom-Json).validation.ok
    } else {
        $false
    }
    tauri_release_build = 'passed'
    evidence_secret_leak = $secretLeak
}
$combined | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'combined-summary.json') -Encoding utf8NoBOM
Get-Content -LiteralPath (Join-Path $evidence 'combined-summary.json')
