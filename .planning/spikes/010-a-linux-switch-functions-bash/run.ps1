$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Split-Path -Parent $MyInvocation.MyCommand.Path)).Path
$wsl = Join-Path $env:SystemRoot 'System32\wsl.exe'
$drive = $root.Substring(0, 1).ToLowerInvariant()
$rest = $root.Substring(2).Replace('\', '/')
$rootLinux = "/mnt/$drive$rest"

$bootstrap = @"
set -e
cd '$rootLinux'
mkdir -p .run
if [ ! -x .run/bash-4.4-install/bin/bash ]; then
  if [ ! -f .run/bash-4.4.tar.gz ]; then
    curl -fsSL https://ftp.gnu.org/gnu/bash/bash-4.4.tar.gz -o .run/bash-4.4.tar.gz
  fi
  if [ ! -d .run/bash-4.4 ]; then
    tar -xzf .run/bash-4.4.tar.gz -C .run
  fi
  cd .run/bash-4.4
  ./configure --quiet --without-bash-malloc --prefix='$rootLinux/.run/bash-4.4-install'
  make -s -j2
  make -s install
fi
"@
& $wsl -d Ubuntu -- bash -lc $bootstrap
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& $wsl -d Ubuntu -- "$rootLinux/.run/bash-4.4-install/bin/bash" "$rootLinux/tests.bash"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Get-Content -Raw -LiteralPath (Join-Path $root '.run\summary.json') -Encoding utf8
