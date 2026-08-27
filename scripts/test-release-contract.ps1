[CmdletBinding()]
param(
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$contradictions = [System.Collections.Generic.List[string]]::new()
$checkedDocuments = [System.Collections.Generic.List[string]]::new()
$contractPath = Join-Path $root 'scripts/windows-release-contract.json'

try {
    $releaseContract = [System.IO.File]::ReadAllText($contractPath) | ConvertFrom-Json
} catch {
    $releaseContract = $null
    $contradictions.Add('scripts/windows-release-contract.json is missing or invalid.')
}

if ($null -ne $releaseContract) {
    if ($releaseContract.schemaVersion -ne 1 -or
        $releaseContract.issue -ne 28 -or
        $releaseContract.desktopConsumerControl -ne 'trusted_start_confirmed_tree_restart') {
        $contradictions.Add('The structured Windows release contract identity is invalid.')
    }

    foreach ($document in @($releaseContract.documents)) {
        $relativePath = [string]$document.path
        $checkedDocuments.Add($relativePath)
        $path = Join-Path $root $relativePath
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            $contradictions.Add("Missing contract document: $relativePath.")
            continue
        }

        $content = [System.IO.File]::ReadAllText($path)
        if (-not $content.Contains([string]$releaseContract.documentMarker)) {
            $contradictions.Add("$relativePath does not declare the structured desktop consumer control boundary.")
        }
        foreach ($pattern in @($releaseContract.forbiddenDesktopControlPatterns)) {
            if ($content -match [string]$pattern) {
                $contradictions.Add("$relativePath exceeds the trusted desktop consumer control boundary.")
            }
        }
    }

    foreach ($decision in @($releaseContract.supersededDecisions)) {
        $relativePath = [string]$decision.path
        $checkedDocuments.Add($relativePath)
        $path = Join-Path $root $relativePath
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or
            -not [System.IO.File]::ReadAllText($path).Contains([string]$decision.status)) {
            $contradictions.Add("$relativePath does not preserve its superseded decision status.")
        }
    }

    $tauriConfigPath = Join-Path $root 'src-tauri/tauri.conf.json'
    $checkedDocuments.Add('src-tauri/tauri.conf.json')
    try {
        $tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
        $mainWindow = @($tauriConfig.app.windows | Where-Object { $_.label -eq 'main' })
        if ($mainWindow.Count -ne 1 -or
            [int]$mainWindow[0].width -ne [int]$releaseContract.window.defaultWidth -or
            [int]$mainWindow[0].height -ne [int]$releaseContract.window.defaultHeight -or
            [int]$mainWindow[0].minWidth -ne [int]$releaseContract.window.minimumWidth -or
            [int]$mainWindow[0].minHeight -ne [int]$releaseContract.window.minimumHeight) {
            $contradictions.Add('src-tauri/tauri.conf.json does not match the structured window contract.')
        }
    } catch {
        $contradictions.Add('src-tauri/tauri.conf.json could not be read as a release contract.')
    }
}

$report = [ordered]@{
    passed = $contradictions.Count -eq 0
    contradictions = @($contradictions)
    checkedDocuments = @($checkedDocuments)
}

$report | ConvertTo-Json -Depth 5
if (-not $report.passed) {
    exit 1
}
