$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root '.run'
New-Item -ItemType Directory -Path $work -Force | Out-Null

Write-Host '== Rust path probe =='
cargo build --quiet --manifest-path (Join-Path $root 'Cargo.toml')
$binary = Join-Path $root 'target\debug\codex-native-config-contract.exe'
& $binary paths

$portFile = Join-Path $work 'port.txt'
$serverLog = Join-Path $work 'server.jsonl'
Remove-Item -LiteralPath $portFile,$serverLog -Force -ErrorAction SilentlyContinue
$server = Start-Process -FilePath $binary `
  -ArgumentList @('serve',$portFile,$serverLog,'2') `
  -WorkingDirectory $root -WindowStyle Hidden -PassThru
try {
  for ($i = 0; $i -lt 100 -and -not (Test-Path -LiteralPath $portFile); $i++) {
    Start-Sleep -Milliseconds 100
  }
  if (-not (Test-Path -LiteralPath $portFile)) { throw 'mock server did not publish a port' }
  $port = Get-Content -LiteralPath $portFile -Raw

  $envHome = Join-Path $work 'env-key-home'
  $tokenHome = Join-Path $work 'bearer-token-home'
  New-Item -ItemType Directory -Path $envHome,$tokenHome -Force | Out-Null
  $baseConfig = @"
model = "mock-model"
model_provider = "mock"

[model_providers.mock]
name = "Local Mock"
base_url = "http://127.0.0.1:$port/v1"
wire_api = "responses"
supports_websockets = false
"@

  $env:CODEX_HOME = $envHome
  $env:GPTEASY_SPIKE_KEY = 'spike-secret-value'
  Set-Content -LiteralPath (Join-Path $envHome 'config.toml') -Value ($baseConfig + "`nenv_key = `"GPTEASY_SPIKE_KEY`"`n") -NoNewline
  Write-Host '== env_key config =='
  & codex exec --skip-git-repo-check --sandbox read-only --json 'Reply with the mock server response.' 2>&1 | Tee-Object -FilePath (Join-Path $work 'env-key.codex.json')
  $envResult = $LASTEXITCODE

  Remove-Item Env:GPTEASY_SPIKE_KEY
  $env:CODEX_HOME = $tokenHome
  Set-Content -LiteralPath (Join-Path $tokenHome 'config.toml') -Value ($baseConfig + "`nexperimental_bearer_token = `"spike-secret-value`"`n") -NoNewline
  Write-Host '== experimental_bearer_token config =='
  & codex exec --skip-git-repo-check --sandbox read-only --json 'Reply with the mock server response.' 2>&1 | Tee-Object -FilePath (Join-Path $work 'bearer-token.codex.json')
  $tokenResult = $LASTEXITCODE

  $summary = [ordered]@{
    env_key_exit_code = $envResult
    experimental_bearer_token_exit_code = $tokenResult
    missing_env_exit_code = $null
    server_requests = @()
  }
  $missingHome = Join-Path $work 'missing-env-home'
  New-Item -ItemType Directory -Path $missingHome -Force | Out-Null
  $env:CODEX_HOME = $missingHome
  Set-Content -LiteralPath (Join-Path $missingHome 'config.toml') -Value ($baseConfig + "`nenv_key = `"GPTEASY_SPIKE_KEY`"`n") -NoNewline
  Write-Host '== missing env_key variable =='
  & codex exec --skip-git-repo-check --sandbox read-only --json 'This must fail before reaching the server.' 2>&1 | Tee-Object -FilePath (Join-Path $work 'missing-env.codex.txt')
  $summary.missing_env_exit_code = $LASTEXITCODE
  if (Test-Path -LiteralPath $serverLog) {
    $summary.server_requests = @(Get-Content -LiteralPath $serverLog | ForEach-Object { $_ | ConvertFrom-Json })
  }
  $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $work 'summary.json')
  Get-Content -LiteralPath (Join-Path $work 'summary.json')
  if ($envResult -ne 0 -or $tokenResult -ne 0 -or $summary.missing_env_exit_code -eq 0) { throw 'one or more Codex config contract checks failed' }
}
finally {
  Remove-Item Env:CODEX_HOME -ErrorAction SilentlyContinue
  Remove-Item Env:GPTEASY_SPIKE_KEY -ErrorAction SilentlyContinue
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
  }
}
