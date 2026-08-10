[CmdletBinding()]
param(
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$trackedOutput = & git -C $root ls-files -z
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to enumerate tracked files for the release tree gate.'
}

$trackedFiles = @(
    ($trackedOutput -join "`n") -split "`0" |
        Where-Object { $_ } |
        ForEach-Object { $_.Replace('\', '/') }
)

$legacySourceEntries = @(
    $trackedFiles | Where-Object {
        $_ -match '(^|/)(\.planning|legacy-source|old-product|src-old|app-old)(/|$)'
    }
)

$activeRoadmapEntries = @(
    $trackedFiles | Where-Object {
        $_ -match '(^|/)(ROADMAP|PROGRESS|STATUS)\.md$' -or
        $_ -match '^docs/roadmap/'
    }
)

$archiveProgressReferences = [System.Collections.Generic.List[string]]::new()
$active88ItemReferences = [System.Collections.Generic.List[string]]::new()
foreach ($relativePath in $trackedFiles) {
    if ($relativePath.StartsWith('docs/archive/') -or $relativePath.StartsWith('docs/adr/')) {
        continue
    }
    if ($relativePath -eq 'scripts/test-release-tree.ps1') {
        continue
    }

    $path = Join-Path $root $relativePath
    try {
        $content = [System.IO.File]::ReadAllText($path)
    } catch {
        continue
    }
    if ($content.Contains([char]0)) {
        continue
    }

    if ($content -match 'docs/archive/|implemented-features-|\u5df2\u5f00\u53d1\u529f\u80fd\u5f52\u6863') {
        $archiveProgressReferences.Add($relativePath)
    }
    if ($content -match '88\s*\u9879|88-item') {
        $active88ItemReferences.Add($relativePath)
    }
}

$activeRoadmapEntries = @(
    $activeRoadmapEntries
    $active88ItemReferences
) | Sort-Object -Unique
$archiveProgressReferences = @($archiveProgressReferences) | Sort-Object -Unique

$passed = $legacySourceEntries.Count -eq 0 -and
    $activeRoadmapEntries.Count -eq 0 -and
    $archiveProgressReferences.Count -eq 0
$report = [ordered]@{
    passed = $passed
    trackedFileCount = $trackedFiles.Count
    legacySourceEntries = @($legacySourceEntries | Sort-Object -Unique)
    activeRoadmapEntries = @($activeRoadmapEntries)
    archiveProgressReferences = @($archiveProgressReferences)
}

$report | ConvertTo-Json -Depth 5
if (-not $passed) {
    exit 1
}
