$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$bundle = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$installer = Get-ChildItem -LiteralPath $bundle -File -Filter '*-setup.exe' | Select-Object -First 1
if (-not $installer) { throw 'run run-build.ps1 first' }

$before = @(Get-StartApps | Where-Object { $_.Name -eq 'GPTEasy Spike 005' })
$process = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "installer failed with exit code $($process.ExitCode)" }
Start-Sleep -Seconds 2

$candidates = @(@(
  (Join-Path $env:LOCALAPPDATA 'Programs\GPTEasy Spike 005'),
  (Join-Path $env:LOCALAPPDATA 'GPTEasy Spike 005')
) | Where-Object { Test-Path -LiteralPath $_ })
if ($candidates.Count -ne 1) { throw "expected one current-user install root, found $($candidates.Count)" }
$installRoot = (Resolve-Path -LiteralPath $candidates[0]).Path
$localRoot = (Resolve-Path -LiteralPath $env:LOCALAPPDATA).Path
if (-not $installRoot.StartsWith($localRoot, [StringComparison]::OrdinalIgnoreCase)) {
  throw "installer escaped current-user LocalAppData: $installRoot"
}
$app = Get-ChildItem -LiteralPath $installRoot -File -Filter 'gpteasy-spike-005.exe' | Select-Object -First 1
$uninstaller = Get-ChildItem -LiteralPath $installRoot -File -Filter 'uninstall.exe' | Select-Object -First 1
if (-not $app -or -not $uninstaller) { throw 'installed app or uninstaller missing' }

$after = @(Get-StartApps | Where-Object { $_.Name -eq 'GPTEasy Spike 005' })
$summary = [ordered]@{
  installer_exit_code = $process.ExitCode
  install_root = $installRoot
  under_local_app_data = $true
  app_exists = $app.Exists
  uninstaller_exists = $uninstaller.Exists
  start_menu_entries_before = $before.Count
  start_menu_entries_after = $after.Count
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $root '.run\install-summary.json')
Get-Content -LiteralPath (Join-Path $root '.run\install-summary.json')

$uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { throw "uninstaller failed with exit code $($uninstall.ExitCode)" }
Start-Sleep -Seconds 2
if (Test-Path -LiteralPath $app.FullName) { throw 'app still exists after uninstall' }
