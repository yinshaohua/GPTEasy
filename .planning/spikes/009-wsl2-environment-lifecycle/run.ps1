$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root '.run'
New-Item -ItemType Directory -Path $work -Force | Out-Null
$evidence = Join-Path $work 'windows-evidence.json'
& (Join-Path $root 'inspect-wsl.ps1') -Output $evidence | Out-Null
cargo run --quiet --manifest-path (Join-Path $root 'Cargo.toml') -- run $work $evidence
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
