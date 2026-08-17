[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [Parameter(Mandatory = $true)]
    [string]$Tag,
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}
$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path

function Read-RustPackage([string]$ManifestPath) {
    $metadata = (& cargo metadata --manifest-path $ManifestPath --no-deps --format-version 1 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo metadata failed: $metadata"
    }
    $parsed = $metadata | ConvertFrom-Json
    $manifest = (Resolve-Path -LiteralPath $ManifestPath).Path
    $package = @($parsed.packages | Where-Object {
        (Resolve-Path -LiteralPath ([string]$_.manifest_path)).Path -eq $manifest
    })
    if ($package.Count -ne 1) {
        throw 'Unable to identify the Rust package version.'
    }
    return [pscustomobject]@{
        name = [string]$package[0].name
        version = [string]$package[0].version
    }
}

if ($Version -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
    throw 'Version must be a stable SemVer value such as 1.2.3.'
}
if ($Tag -cne "v$Version") {
    throw "Tag must exactly match v$Version."
}

$branch = (& git -C $root branch --show-current | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $branch -cne 'main') {
    throw 'Release preparation must run from main.'
}
$worktree = (& git -C $root status --porcelain | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $worktree) {
    throw 'Release preparation requires a clean worktree.'
}
& git -C $root rev-parse --verify --quiet "refs/tags/$Tag" *> $null
if ($LASTEXITCODE -eq 0) {
    throw "Tag already exists: $Tag."
}

$cargoPath = Join-Path $root 'src-tauri/Cargo.toml'
$cargoLockPath = Join-Path $root 'src-tauri/Cargo.lock'
$jsonVersionTool = Join-Path $PSScriptRoot 'release-version-json.mjs'
$jsonVersionOutput = (& node $jsonVersionTool read $root 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "Unable to read JavaScript and Tauri versions: $jsonVersionOutput"
}
$jsonVersions = $jsonVersionOutput | ConvertFrom-Json
$rustPackage = Read-RustPackage $cargoPath
$rustVersion = [string]$rustPackage.version
$currentVersions = @(
    [string]$jsonVersions.package
    [string]$jsonVersions.packageLock
    [string]$jsonVersions.packageLockRoot
    $rustVersion
    [string]$jsonVersions.tauri
)
if (@($currentVersions | Select-Object -Unique).Count -ne 1) {
    throw "Existing JavaScript, lockfile, Rust, and Tauri versions do not match: $($currentVersions -join ', ')."
}

$cargo = [System.IO.File]::ReadAllText($cargoPath)
$packageVersionPattern = '(?ms)(^\[package\][\s\S]*?^version\s*=\s*")[^"]+("\s*$)'
if ([regex]::Matches($cargo, $packageVersionPattern).Count -ne 1) {
    throw 'Cargo.toml must contain exactly one package version.'
}
$cargoLock = [System.IO.File]::ReadAllText($cargoLockPath)
$escapedPackageName = [regex]::Escape([string]$rustPackage.name)
$lockVersionPattern = '(?ms)(^\[\[package\]\]\s*^name\s*=\s*"' + $escapedPackageName + '"\s*^version\s*=\s*")[^"]+("\s*$)'
if ([regex]::Matches($cargoLock, $lockVersionPattern).Count -ne 1) {
    throw 'Cargo.lock must contain exactly one root package version.'
}
& node $jsonVersionTool write $root $Version
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to update JavaScript and Tauri versions.'
}
$cargo = [regex]::Replace(
    $cargo,
    $packageVersionPattern,
    { param($match) $match.Groups[1].Value + $Version + $match.Groups[2].Value }
).Replace("`r`n", "`n")
[System.IO.File]::WriteAllText($cargoPath, $cargo, [System.Text.UTF8Encoding]::new($false))
$cargoLock = [regex]::Replace(
    $cargoLock,
    $lockVersionPattern,
    { param($match) $match.Groups[1].Value + $Version + $match.Groups[2].Value }
).Replace("`r`n", "`n")
[System.IO.File]::WriteAllText($cargoLockPath, $cargoLock, [System.Text.UTF8Encoding]::new($false))

$verifiedJsonOutput = (& node $jsonVersionTool read $root 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "Unable to verify JavaScript and Tauri versions: $verifiedJsonOutput"
}
$verifiedJson = $verifiedJsonOutput | ConvertFrom-Json
$verifiedVersions = @(
    [string]$verifiedJson.package
    [string]$verifiedJson.packageLock
    [string]$verifiedJson.packageLockRoot
    [string](Read-RustPackage $cargoPath).version
    [string]$verifiedJson.tauri
)
if (@($verifiedVersions | Where-Object { $_ -cne $Version }).Count -ne 0) {
    throw 'Release version verification failed after writing files.'
}
$verifiedCargoLock = [System.IO.File]::ReadAllText($cargoLockPath)
$verifiedLockMatch = [regex]::Match($verifiedCargoLock, $lockVersionPattern)
if (-not $verifiedLockMatch.Success -or $verifiedLockMatch.Groups[0].Value -notmatch ('version\s*=\s*"' + [regex]::Escape($Version) + '"')) {
    throw 'Cargo.lock version verification failed after writing files.'
}

[ordered]@{
    passed = $true
    version = $Version
    tag = $Tag
    previousVersion = $currentVersions[0]
    files = @('package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json')
} | ConvertTo-Json -Depth 4
