$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $root '.run'
New-Item -ItemType Directory -Path $work -Force | Out-Null

$codexCmd = Get-Command codex.cmd -ErrorAction SilentlyContinue
if (-not $codexCmd) {
    throw '找不到 codex.cmd，无法定位原生 codex.exe'
}
$nodeGlobal = Split-Path -Parent $codexCmd.Source
$native = Get-ChildItem -LiteralPath (Join-Path $nodeGlobal 'node_modules\@openai\codex\node_modules') `
    -Recurse -File -Filter 'codex.exe' |
    Where-Object { $_.FullName -match '\\vendor\\.*\\bin\\codex\.exe$' } |
    Select-Object -First 1
if (-not $native) {
    throw "无法在 $nodeGlobal 下找到原生 codex.exe"
}
$env:GPTEASY_CODEX_EXE = $native.FullName

cargo run --quiet --manifest-path (Join-Path $root 'Cargo.toml') -- run $work
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
