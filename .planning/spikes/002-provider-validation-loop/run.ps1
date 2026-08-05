$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root '.run'
New-Item -ItemType Directory -Path $work -Force | Out-Null

cargo build --quiet --manifest-path (Join-Path $root 'Cargo.toml')
$binary = Join-Path $root 'target\debug\provider-validation-loop.exe'
$env:GPTEASY_PROVIDER_KEY = 'spike-provider-key'

$cases = @(
  @{ scenario = 'happy'; expected = 'validated'; exit = 0 },
  @{ scenario = 'auth-error'; expected = 'authentication'; exit = 1 },
  @{ scenario = 'model-missing'; expected = 'model_discovery'; exit = 1 },
  @{ scenario = 'non-sse'; expected = 'streaming'; exit = 1 },
  @{ scenario = 'no-tool'; expected = 'tool_call'; exit = 1 },
  @{ scenario = 'truncated'; expected = 'streaming'; exit = 1 },
  @{ scenario = 'bad-tool-args'; expected = 'tool_call'; exit = 1 }
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
        Start-Sleep -Milliseconds 100
      }
      if (-not (Test-Path -LiteralPath $portFile)) { throw "mock $($case.scenario) did not start" }
      $port = Get-Content -LiteralPath $portFile -Raw
      & $binary validate "http://127.0.0.1:$port/v1" 'mock-model' $clientLog 2>&1 | Set-Content -LiteralPath $resultFile
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

  Write-Host '== remote HTTP policy =='
  $policyLog = Join-Path $work 'remote-http-policy.jsonl'
  $policyResult = Join-Path $work 'remote-http-policy-result.json'
  & $binary validate 'http://example.com/v1' 'mock-model' $policyLog 2>&1 | Set-Content -LiteralPath $policyResult
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

  $summary = [ordered]@{
    passed = @($results | Where-Object passed).Count
    total = $results.Count
    results = $results
  }
  $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $work 'summary.json')
  Get-Content -LiteralPath (Join-Path $work 'summary.json')
  if ($summary.passed -ne $summary.total) {
    throw 'provider validation scenario matrix failed'
  }
}
finally {
  Remove-Item Env:GPTEASY_PROVIDER_KEY -ErrorAction SilentlyContinue
}
