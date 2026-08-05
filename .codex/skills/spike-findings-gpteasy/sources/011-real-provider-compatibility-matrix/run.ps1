param(
    [switch]$SkipLive
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root '.run'
New-Item -ItemType Directory -Path $work -Force | Out-Null

$env:HTTP_PROXY = 'http://127.0.0.1:7897'
$env:HTTPS_PROXY = 'http://127.0.0.1:7897'
$env:NO_PROXY = '127.0.0.1,localhost,::1'

cargo build --quiet --manifest-path (Join-Path $root 'Cargo.toml')
$binary = Join-Path $root 'target\debug\real-provider-compatibility-matrix.exe'
$env:GPTEASY_PROVIDER_KEY = 'spike-provider-key'

$cases = @(
    @{ scenario = 'happy'; expected = 'validated'; exit = 0 },
    @{ scenario = 'auth-error'; expected = 'authentication'; exit = 1 },
    @{ scenario = 'model-missing'; expected = 'model_discovery'; exit = 1 },
    @{ scenario = 'non-sse'; expected = 'streaming'; exit = 1 },
    @{ scenario = 'no-tool'; expected = 'tool_call'; exit = 1 },
    @{ scenario = 'truncated'; expected = 'streaming'; exit = 1 },
    @{ scenario = 'bad-tool-args'; expected = 'tool_call'; exit = 1 },
    @{ scenario = 'rate-limit'; expected = 'rate_limit'; exit = 1 },
    @{ scenario = 'first-event-timeout'; expected = 'first_event_timeout'; exit = 1 },
    @{ scenario = 'idle-timeout'; expected = 'stream_idle_timeout'; exit = 1 },
    @{ scenario = 'overall-timeout'; expected = 'overall_timeout'; exit = 1 }
)

$results = @()
try {
    foreach ($case in $cases) {
        $caseDir = Join-Path $work $case.scenario
        New-Item -ItemType Directory -Path $caseDir -Force | Out-Null
        $portFile = Join-Path $caseDir 'port.txt'
        $serverLog = Join-Path $caseDir 'server.jsonl'
        $clientLog = Join-Path $caseDir 'client.jsonl'
        $resultFile = Join-Path $caseDir 'result.json'
        Remove-Item -LiteralPath $portFile,$serverLog,$clientLog,$resultFile -Force -ErrorAction SilentlyContinue
        $server = Start-Process -FilePath $binary `
            -ArgumentList @('mock',$case.scenario,$portFile,$serverLog) `
            -WorkingDirectory $root -WindowStyle Hidden -PassThru
        try {
            for ($i = 0; $i -lt 100 -and -not (Test-Path -LiteralPath $portFile); $i++) {
                Start-Sleep -Milliseconds 50
            }
            if (-not (Test-Path -LiteralPath $portFile)) {
                throw "mock $($case.scenario) did not start"
            }
            $port = (Get-Content -LiteralPath $portFile -Raw).Trim()
            & $binary validate "http://127.0.0.1:$port/v1" 'mock-model' $clientLog fast |
                Set-Content -LiteralPath $resultFile -Encoding utf8NoBOM
            $exitCode = $LASTEXITCODE
            $result = Get-Content -LiteralPath $resultFile -Raw | ConvertFrom-Json
            $passed = $exitCode -eq $case.exit -and $result.category -eq $case.expected
            $results += [ordered]@{
                scenario = $case.scenario
                passed = $passed
                exit_code = $exitCode
                expected_exit_code = $case.exit
                category = $result.category
                expected_category = $case.expected
                stages = @($result.stages).Count
            }
        }
        finally {
            if ($server -and -not $server.HasExited) {
                Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
            }
        }
    }

    $policyLog = Join-Path $work 'remote-http-policy.jsonl'
    $policyResult = Join-Path $work 'remote-http-policy-result.json'
    & $binary validate 'http://example.com/v1' 'mock-model' $policyLog fast |
        Set-Content -LiteralPath $policyResult -Encoding utf8NoBOM
    $policyExit = $LASTEXITCODE
    $policy = Get-Content -LiteralPath $policyResult -Raw | ConvertFrom-Json
    $results += [ordered]@{
        scenario = 'remote-http-policy'
        passed = $policyExit -eq 1 -and $policy.category -eq 'security_policy'
        exit_code = $policyExit
        expected_exit_code = 1
        category = $policy.category
        expected_category = 'security_policy'
        stages = @($policy.stages).Count
    }
}
finally {
    Remove-Item Env:GPTEASY_PROVIDER_KEY -ErrorAction SilentlyContinue
}

$deterministicPassed = @($results | Where-Object passed).Count
if ($deterministicPassed -ne $results.Count) {
    $results | ConvertTo-Json -Depth 8
    throw 'provider compatibility deterministic matrix failed'
}

$liveSummary = [ordered]@{
    executed = $false
    ok = $null
    category = $null
    stages = 0
    secret_leak_scan_passed = $null
}

if (-not $SkipLive) {
    $spikesRoot = Split-Path -Parent $root
    $secret = Join-Path $spikesRoot '.secrets\provider.json'
    if (-not (Test-Path -LiteralPath $secret -PathType Leaf)) {
        throw "缺少真实供应商文件：$secret"
    }
    $repository = (Resolve-Path -LiteralPath '.').Path
    $resolvedSecret = (Resolve-Path -LiteralPath $secret).Path
    & git -C $repository check-ignore --quiet -- $resolvedSecret
    if ($LASTEXITCODE -ne 0) {
        throw '真实供应商文件未被 Git 忽略，拒绝读取'
    }
    $liveDir = Join-Path $work 'live'
    New-Item -ItemType Directory -Path $liveDir -Force | Out-Null
    & $binary live $resolvedSecret $liveDir | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "真实供应商验证器执行失败，退出码 $LASTEXITCODE"
    }
    $live = Get-Content -LiteralPath (Join-Path $liveDir 'result.json') -Raw | ConvertFrom-Json
    $liveSummary = [ordered]@{
        executed = $true
        ok = $live.validation.ok
        category = $live.validation.category
        stages = @($live.validation.stages).Count
        secret_leak_scan_passed = $live.secret_leak_scan_passed
    }
    if (-not $live.secret_leak_scan_passed) {
        throw '真实 API Key 泄漏扫描失败'
    }
}

$summary = [ordered]@{
    deterministic_passed = $deterministicPassed
    deterministic_total = $results.Count
    deterministic_results = $results
    live = $liveSummary
}
$summary | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $work 'summary.json') -Encoding utf8NoBOM
Get-Content -LiteralPath (Join-Path $work 'summary.json')
exit 0
