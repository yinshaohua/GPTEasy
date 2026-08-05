param(
    [switch]$KeepDistro
)

$ErrorActionPreference = 'Stop'

$spike = Split-Path -Parent $MyInvocation.MyCommand.Path
$run = Join-Path $spike '.run'
$evidence = Join-Path $run 'evidence'
$cache = Join-Path $run 'cache'
$install = Join-Path $run 'distro'
$distro = 'GPTEasy-Spike-013'
$version = '24.04.3'
$archiveName = "ubuntu-base-$version-base-amd64.tar.gz"
$archiveUrl = "https://cdimage.ubuntu.com/ubuntu-base/releases/$version/release/$archiveName"
$sumsUrl = "https://cdimage.ubuntu.com/ubuntu-base/releases/$version/release/SHA256SUMS"
$archive = Join-Path $cache $archiveName
$sums = Join-Path $cache 'SHA256SUMS'

function Get-WslNames([switch]$Running) {
    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = 'wsl.exe'
    $info.ArgumentList.Add('--list')
    if ($Running) {
        $info.ArgumentList.Add('--running')
    }
    $info.ArgumentList.Add('--quiet')
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [System.Text.Encoding]::Unicode
    $info.StandardErrorEncoding = [System.Text.Encoding]::Unicode
    $info.CreateNoWindow = $true
    $process = [System.Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw "wsl --list failed: $stderr"
    }
    @($stdout -split "`r?`n" | ForEach-Object Trim | Where-Object { $_ })
}

function Invoke-WslCommand([string[]]$Arguments) {
    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = 'wsl.exe'
    foreach ($argument in $Arguments) {
        $info.ArgumentList.Add($argument)
    }
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [System.Text.Encoding]::Unicode
    $info.StandardErrorEncoding = [System.Text.Encoding]::Unicode
    $info.CreateNoWindow = $true
    $process = [System.Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw "wsl $($Arguments -join ' ') failed: $stderr"
    }
    $stdout
}

function Remove-SafeDirectory([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $resolvedRoot = [System.IO.Path]::GetFullPath($run).TrimEnd('\')
    $resolvedTarget = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    if (-not $resolvedTarget.StartsWith($resolvedRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing recursive delete outside spike run directory: $resolvedTarget"
    }
    Remove-Item -LiteralPath $resolvedTarget -Recurse -Force
}

New-Item -ItemType Directory -Path $run, $cache -Force | Out-Null
Remove-SafeDirectory $evidence
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$beforeNames = @(Get-WslNames | Sort-Object)
$beforeRunning = @(Get-WslNames -Running | Sort-Object)

$env:HTTP_PROXY = 'http://127.0.0.1:7897'
$env:HTTPS_PROXY = 'http://127.0.0.1:7897'

if (-not (Test-Path -LiteralPath $archive)) {
    curl.exe --fail --location --proxy $env:HTTPS_PROXY --output $archive $archiveUrl
}
if (-not (Test-Path -LiteralPath $sums)) {
    curl.exe --fail --location --proxy $env:HTTPS_PROXY --output $sums $sumsUrl
}

$expected = (Select-String -LiteralPath $sums -Pattern ([regex]::Escape($archiveName) + '$')).Line.Split()[0].ToLowerInvariant()
$actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($expected -ne $actual) {
    throw "Ubuntu base checksum mismatch: expected $expected, got $actual"
}

if ((Get-WslNames) -contains $distro) {
    Invoke-WslCommand @('--unregister', $distro) | Out-Null
}
Remove-SafeDirectory $install
New-Item -ItemType Directory -Path $install -Force | Out-Null

try {
    Invoke-WslCommand @('--import', $distro, $install, $archive, '--version', '2') | Out-Null

    Push-Location $spike
    try {
        cargo run --release -- prepare $distro
        if ($LASTEXITCODE -ne 0) {
            throw "prepare failed with exit code $LASTEXITCODE"
        }
        cargo run --release -- matrix $distro $evidence
        if ($LASTEXITCODE -ne 0) {
            throw "matrix failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}
finally {
    if (-not $KeepDistro -and (Get-WslNames) -contains $distro) {
        Invoke-WslCommand @('--unregister', $distro) | Out-Null
    }
}

$afterNames = @(Get-WslNames | Sort-Object)
$afterRunning = @(Get-WslNames -Running | Sort-Object)
$namesRestored = (($beforeNames -join "`n") -eq ($afterNames -join "`n"))
$runningRestored = (($beforeRunning -join "`n") -eq ($afterRunning -join "`n"))

$lifecycle = [ordered]@{
    generated_at = [DateTimeOffset]::UtcNow.ToString('o')
    before = [ordered]@{
        distributions = $beforeNames
        running = $beforeRunning
    }
    after = [ordered]@{
        distributions = $afterNames
        running = $afterRunning
    }
    distributions_restored = $namesRestored
    running_set_restored = $runningRestored
    ubuntu_base_archive = $archiveName
    ubuntu_base_sha256 = $actual
}
$lifecycle | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'lifecycle.json') -Encoding utf8NoBOM

$summaryPath = Join-Path $evidence 'summary.json'
$summary = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
$summary | Add-Member -NotePropertyName host_lifecycle_restored -NotePropertyValue ($namesRestored -and $runningRestored)
if (-not $summary.host_lifecycle_restored) {
    $summary.verdict = 'partial'
}
$summary | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $summaryPath -Encoding utf8NoBOM

if (-not $namesRestored -or -not $runningRestored) {
    throw 'WSL distribution or running set was not restored after the spike.'
}

Get-Content -LiteralPath $summaryPath
