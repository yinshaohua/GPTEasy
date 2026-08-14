[CmdletBinding()]
param(
    [switch]$RequireAuthenticode
)

$ErrorActionPreference = 'Stop'

function Invoke-Checked([string]$Description, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $LASTEXITCODE."
    }
}

function Get-Sha256File([string]$Path) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
        $stream.Dispose()
        $sha256.Dispose()
    }
}

function Get-FileSignature([string]$Path) {
    if ($PSVersionTable.PSEdition -eq 'Desktop') {
        $module = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1'
        Import-Module -Name $module -Force -ErrorAction Stop
    } else {
        Import-Module -Name Microsoft.PowerShell.Security -ErrorAction Stop
    }
    return Get-AuthenticodeSignature -LiteralPath $Path
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

$isWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
if (-not $isWindowsHost -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64' -or
    [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') {
    throw 'The Windows release candidate must be built by an x64 process on Windows x64.'
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseContract = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot 'windows-release-contract.json')) | ConvertFrom-Json
$branch = (& git -C $repoRoot branch --show-current | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $branch -ne 'main') {
    throw 'The Windows release candidate must be built from main.'
}
$worktree = (& git -C $repoRoot status --porcelain | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $worktree) {
    throw 'The Windows release candidate requires a clean worktree.'
}
$commit = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()

Push-Location $repoRoot
try {
    Invoke-Checked 'Frontend typecheck and lint' { npm run check }
    Invoke-Checked 'Frontend test suite' { npm test }
    Invoke-Checked 'Frontend layout test suite' { npm run test:layout }
    Invoke-Checked 'Rust test suite' { cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=1 }
    Invoke-Checked "Issue #$($releaseContract.issue) comprehensive acceptance gate" { npm run acceptance }
    Invoke-Checked 'Release tree gate' {
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/test-release-tree.ps1 -RepositoryRoot $repoRoot
    }
    Invoke-Checked 'Release contract gate' {
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/test-release-contract.ps1 -RepositoryRoot $repoRoot
    }
    Invoke-Checked 'Tauri Windows x64 NSIS build' {
        npx --no-install tauri build --target x86_64-pc-windows-msvc
    }
} finally {
    Pop-Location
}

$config = Get-Content -LiteralPath (Join-Path $repoRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = [string]$config.version
$installerPath = Join-Path $repoRoot "src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\GPTEasy_${version}_x64-setup.exe"
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    $installerPath = Join-Path $repoRoot "src-tauri\target\release\bundle\nsis\GPTEasy_${version}_x64-setup.exe"
}
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    throw 'Tauri did not produce the expected Windows x64 NSIS installer.'
}

$installer = Get-Item -LiteralPath $installerPath
$signature = Get-FileSignature $installer.FullName
if ($signature.Status -ne 'Valid' -and $signature.Status -ne 'NotSigned') {
    throw "Installer Authenticode status is not acceptable: $($signature.Status)."
}
if ($RequireAuthenticode -and $signature.Status -ne 'Valid') {
    throw 'Formal release candidate requires a valid Authenticode signature.'
}

$manifestRoot = Join-Path $repoRoot 'src-tauri\target\release-candidate'
New-Item -ItemType Directory -Path $manifestRoot -Force | Out-Null
$manifestPath = Join-Path $manifestRoot 'manifest.json'
if (-not $installer.FullName.StartsWith($repoRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The candidate installer must be inside the repository target directory.'
}
$relativeInstaller = $installer.FullName.Substring($repoRoot.Length).TrimStart('\').Replace('\', '/')
$manifest = [ordered]@{
    schemaVersion = 1
    issue = [int]$releaseContract.issue
    gitCommit = $commit
    builtAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    platform = 'windows-x64-current-user'
    verification = [ordered]@{
        frontendCheck = 'passed'
        frontendTests = 'passed'
        layoutTests = 'passed'
        rustTests = 'passed'
        acceptanceGate = 'passed'
        releaseTree = 'passed'
        releaseContract = 'passed'
    }
    artifact = [ordered]@{
        path = $relativeInstaller
        sha256 = Get-Sha256File $installer.FullName
        size = $installer.Length
        authenticodeStatus = $signature.Status.ToString()
    }
}
$json = $manifest | ConvertTo-Json -Depth 8
Write-Utf8NoBom $manifestPath $json
Write-Output $json
