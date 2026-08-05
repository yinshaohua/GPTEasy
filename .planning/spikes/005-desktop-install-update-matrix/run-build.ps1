$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$run = Join-Path $root '.run'
New-Item -ItemType Directory -Path $run -Force | Out-Null

$env:HTTP_PROXY = 'http://127.0.0.1:7897'
$env:HTTPS_PROXY = 'http://127.0.0.1:7897'
npm --prefix $root install --no-audit --no-fund
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$privateKey = Join-Path $run 'updater.key'
$publicKey = "$privateKey.pub"
$password = 'gpteasy-spike-only'
if (-not (Test-Path -LiteralPath $privateKey)) {
  npm --prefix $root run tauri -- signer generate --write-keys $privateKey --password $password --ci
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
if (-not (Test-Path -LiteralPath $publicKey)) { throw 'updater public key was not generated' }

$pubkeyText = (Get-Content -LiteralPath $publicKey -Raw).Trim()
$override = [ordered]@{
  plugins = [ordered]@{
    updater = [ordered]@{
      pubkey = $pubkeyText
      endpoints = @('https://updates.example.invalid/gpteasy/{{target}}/{{arch}}/{{current_version}}')
      windows = [ordered]@{ installMode = 'passive' }
    }
  }
}
$overridePath = Join-Path $run 'build-config.json'
$override | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $overridePath

$env:TAURI_SIGNING_PRIVATE_KEY = $privateKey
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $password
try {
  npm --prefix $root run tauri -- build --bundles nsis --config $overridePath
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
}

$bundle = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$installer = Get-ChildItem -LiteralPath $bundle -File -Filter '*-setup.exe' | Select-Object -First 1
$signature = Get-ChildItem -LiteralPath $bundle -File -Filter '*-setup.exe.sig' | Select-Object -First 1
if (-not $installer) { throw 'NSIS installer not found' }
if (-not $signature) { throw 'signed updater artifact not found' }

$summary = [ordered]@{
  tauri_cli = (npm --prefix $root exec tauri -- --version)
  installer = $installer.FullName
  installer_size = $installer.Length
  updater_signature = $signature.FullName
  updater_signature_size = $signature.Length
  updater_public_key_present = $pubkeyText.Length -gt 0
  install_mode = 'currentUser'
  updater_requires_explicit_second_action = $true
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'build-summary.json')
Get-Content -LiteralPath (Join-Path $run 'build-summary.json')
