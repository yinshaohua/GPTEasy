$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Split-Path -Parent $MyInvocation.MyCommand.Path)).Path
$wsl = Join-Path $env:SystemRoot 'System32\wsl.exe'
$drive = $root.Substring(0, 1).ToLowerInvariant()
$rest = $root.Substring(2).Replace('\', '/')
$rootLinux = "/mnt/$drive$rest"

$systemZshOutput = & $wsl -d Ubuntu -- sh -lc 'command -v zsh 2>/dev/null || true'
$systemZsh = if ($systemZshOutput) { ($systemZshOutput -join '').Trim() } else { '' }
if ($systemZsh) {
    $zsh = $systemZsh
} else {
    $bootstrap = @"
set -e
cd '$rootLinux'
mkdir -p .run/packages .run/zsh-root
if [ ! -x .run/zsh-root/bin/zsh ]; then
  cd .run/packages
  apt download zsh zsh-common
  cd ..
  for package in packages/*.deb; do
    dpkg-deb -x "`$package" zsh-root
  done
fi
"@
    & $wsl -d Ubuntu -- bash -lc $bootstrap
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $zsh = "$rootLinux/.run/zsh-root/bin/zsh"
}

& $wsl -d Ubuntu -- $zsh -f "$rootLinux/tests.zsh"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Get-Content -Raw -LiteralPath (Join-Path $root '.run\summary.json') -Encoding utf8
