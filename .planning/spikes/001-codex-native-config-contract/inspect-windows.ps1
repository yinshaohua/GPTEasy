$ErrorActionPreference = 'Stop'
$out = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '.run/windows-evidence.json'
New-Item -ItemType Directory -Path (Split-Path -Parent $out) -Force | Out-Null

$package = Get-AppxPackage | Where-Object { $_.Name -eq 'OpenAI.Codex' } | Select-Object -First 1
$processes = Get-CimInstance Win32_Process | Where-Object {
  $_.Name -in @('ChatGPT.exe', 'codex.exe')
} | ForEach-Object {
  [ordered]@{
    name = $_.Name
    pid = $_.ProcessId
    parent_pid = $_.ParentProcessId
    executable = $_.ExecutablePath
    command_line_has_codex_home = $_.CommandLine -match 'CODEX_HOME'
    command_line_has_auth_value = $_.CommandLine -match '(?i)(api[_-]?key|authorization|bearer)'
    command_line = $_.CommandLine
  }
}

$userHome = [Environment]::GetFolderPath('UserProfile')
$codexHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $userHome '.codex' }
$evidence = [ordered]@{
  captured_at = (Get-Date).ToUniversalTime().ToString('o')
  os = 'windows'
  user_home = $userHome
  codex_home = $codexHome
  config_toml = Join-Path $codexHome 'config.toml'
  auth_json = Join-Path $codexHome 'auth.json'
  package = if ($package) {
    [ordered]@{
      name = $package.Name
      version = $package.Version.ToString()
      install_location = $package.InstallLocation
    }
  } else { $null }
  processes = @($processes)
}
$evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $out
Get-Content -LiteralPath $out
