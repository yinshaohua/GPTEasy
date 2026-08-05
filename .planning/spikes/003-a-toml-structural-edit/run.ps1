$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root '.run'
New-Item -ItemType Directory -Path $work -Force | Out-Null
cargo run --quiet --manifest-path (Join-Path $root 'Cargo.toml') -- run $work
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
