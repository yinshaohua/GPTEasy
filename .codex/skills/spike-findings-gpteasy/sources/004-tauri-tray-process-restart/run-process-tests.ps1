$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root ('.run\session-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
New-Item -ItemType Directory -Path $work -Force | Out-Null

$fixtureBinary = Join-Path $work 'fixture.exe'
rustc (Join-Path $root 'fixture.rs') -O -o $fixtureBinary

$desktopDir = Join-Path $work 'WindowsApps\OpenAI.Codex_fixture\app'
$resourcesDir = Join-Path $desktopDir 'resources'
$cliDir = Join-Path $work 'cli'
New-Item -ItemType Directory -Path $resourcesDir,$cliDir -Force | Out-Null
$desktop = Join-Path $desktopDir 'ChatGPT.exe'
$desktopChild = Join-Path $resourcesDir 'codex.exe'
$cli = Join-Path $cliDir 'codex.exe'
Copy-Item -LiteralPath $fixtureBinary -Destination $desktop
Copy-Item -LiteralPath $fixtureBinary -Destination $desktopChild
Copy-Item -LiteralPath $fixtureBinary -Destination $cli

$desktopProcess = Start-Process -FilePath $desktop -ArgumentList @('desktop-root',$desktopChild) -WindowStyle Hidden -PassThru
$cliProcess = Start-Process -FilePath $cli -ArgumentList @('cli') -WindowStyle Hidden -PassThru
try {
  Start-Sleep -Seconds 1
  cargo build --quiet --manifest-path (Join-Path $root 'src-tauri\Cargo.toml')
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  $app = Join-Path $root 'src-tauri\target\debug\gpteasy-spike-004.exe'

  & $app --probe | Set-Content -LiteralPath (Join-Path $work 'real-and-fixture-processes.json')
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  & $app --plan immediate | Set-Content -LiteralPath (Join-Path $work 'immediate-plan.json')
  & $app --plan later | Set-Content -LiteralPath (Join-Path $work 'later-plan.json')
  & $app --plan cancel | Set-Content -LiteralPath (Join-Path $work 'cancel-plan.json')
  & $app --fixture-cycle $work | Set-Content -LiteralPath (Join-Path $work 'fixture-cycle.json')
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

  $scan = Get-Content -LiteralPath (Join-Path $work 'real-and-fixture-processes.json') -Raw | ConvertFrom-Json
  $immediate = Get-Content -LiteralPath (Join-Path $work 'immediate-plan.json') -Raw | ConvertFrom-Json
  $later = Get-Content -LiteralPath (Join-Path $work 'later-plan.json') -Raw | ConvertFrom-Json
  $cancel = Get-Content -LiteralPath (Join-Path $work 'cancel-plan.json') -Raw | ConvertFrom-Json
  $cycle = Get-Content -LiteralPath (Join-Path $work 'fixture-cycle.json') -Raw | ConvertFrom-Json

  $fixtureScan = @($scan.processes | Where-Object { $_.executable -like "$work*" })
  $checks = @(
    @{ name = 'fixture-desktop-root-detected'; passed = @($fixtureScan | Where-Object role -eq 'desktop_root').Count -eq 1 },
    @{ name = 'fixture-desktop-child-detected'; passed = @($fixtureScan | Where-Object role -eq 'desktop_codex_child').Count -eq 1 },
    @{ name = 'fixture-cli-detected'; passed = @($fixtureScan | Where-Object role -eq 'cli').Count -eq 1 },
    @{ name = 'immediate-cli-is-manual'; passed = @($immediate.actions | Where-Object { $_.role -eq 'cli' -and $_.action -eq 'manual_restart_required' }).Count -ge 1 },
    @{ name = 'later-writes-and-pends'; passed = $later.write_configuration -and $later.pending_restart },
    @{ name = 'cancel-does-not-write'; passed = -not $cancel.write_configuration },
    @{ name = 'fixture-desktop-relaunched'; passed = $cycle.desktop_relaunched -and $cycle.desktop_child_relaunched },
    @{ name = 'fixture-cli-not-relaunched'; passed = -not $cycle.cli_relaunched }
  )
  $summary = [ordered]@{
    passed = @($checks | Where-Object passed).Count
    total = $checks.Count
    checks = $checks
    windows_app_id = 'OpenAI.Codex_2p2nqsd0c76g0!App'
  }
  $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $root '.run\summary.json')
  Get-Content -LiteralPath (Join-Path $root '.run\summary.json')
  if ($summary.passed -ne $summary.total) { throw 'process/restart matrix failed' }
}
finally {
  Get-Process | Where-Object {
    $_.Path -and $_.Path.StartsWith($work, [StringComparison]::OrdinalIgnoreCase)
  } | Stop-Process -Force -ErrorAction SilentlyContinue
}
