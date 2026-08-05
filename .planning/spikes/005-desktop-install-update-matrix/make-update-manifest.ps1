$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$bundle = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$installer = Get-ChildItem -LiteralPath $bundle -File -Filter '*-setup.exe' | Select-Object -First 1
$signature = Get-ChildItem -LiteralPath $bundle -File -Filter '*-setup.exe.sig' | Select-Object -First 1
if (-not $installer -or -not $signature) { throw 'run run-build.ps1 first' }

$manifest = [ordered]@{
  version = '0.1.0'
  notes = 'Spike-only updater manifest'
  pub_date = (Get-Date).ToUniversalTime().ToString('o')
  platforms = [ordered]@{
    'windows-x86_64' = [ordered]@{
      signature = (Get-Content -LiteralPath $signature.FullName -Raw).Trim()
      url = 'https://updates.example.invalid/gpteasy/GPTEasy-Spike-005_0.1.0_x64-setup.exe'
    }
  }
}
$path = Join-Path $root '.run\latest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $path
$parsed = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
$summary = [ordered]@{
  version = $parsed.version
  platform = 'windows-x86_64'
  signature_length = $parsed.platforms.'windows-x86_64'.signature.Length
  url_uses_https = $parsed.platforms.'windows-x86_64'.url.StartsWith('https://')
}
$summary | ConvertTo-Json
