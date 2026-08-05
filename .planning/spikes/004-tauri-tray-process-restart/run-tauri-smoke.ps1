$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$manifest = Join-Path $root 'src-tauri\Cargo.toml'
cargo build --quiet --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$binary = Join-Path $root 'src-tauri\target\debug\gpteasy-spike-004.exe'
$process = Start-Process -FilePath $binary -WorkingDirectory $root -WindowStyle Hidden -PassThru
try {
  Start-Sleep -Seconds 4
  $started = -not $process.HasExited
  $closeSent = $false
  if ($started) {
    $process.Refresh()
    $closeSent = $process.CloseMainWindow()
    Start-Sleep -Seconds 2
    $process.Refresh()
  }
  $survivedClose = $started -and -not $process.HasExited
  $summary = [ordered]@{
    started = $started
    close_message_sent = $closeSent
    survived_close = $survivedClose
    note = '进程仅由此 smoke test 在 finally 中终止；不会触碰 Codex 进程。'
  }
  $summary | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root '.run\tauri-smoke.json')
  Get-Content -LiteralPath (Join-Path $root '.run\tauri-smoke.json')
  if (-not $started -or -not $survivedClose) { throw 'Tauri tray smoke test failed' }
}
finally {
  if ($process -and -not $process.HasExited) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
  }
}
