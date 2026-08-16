[CmdletBinding()]
param(
    [string]$RepositoryRoot,
    [switch]$CheckGitHubPrd
)

$ErrorActionPreference = 'Stop'
$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$contractPath = Join-Path $root 'scripts/linux-wsl-acceptance-contract.json'
$contradictions = [System.Collections.Generic.List[string]]::new()
$checkedDocuments = [System.Collections.Generic.List[string]]::new()

try {
    $contract = [System.IO.File]::ReadAllText($contractPath) | ConvertFrom-Json
} catch {
    $contract = $null
    $contradictions.Add('scripts/linux-wsl-acceptance-contract.json is missing or invalid.')
}

if ($null -ne $contract) {
    if ($contract.schemaVersion -ne 1 -or $contract.issue -ne 35 -or $contract.parentIssue -ne 29) {
        $contradictions.Add('The Linux/WSL2 acceptance contract identity is invalid.')
    }

    foreach ($document in @($contract.documents)) {
        $relativePath = [string]$document.path
        $checkedDocuments.Add($relativePath)
        $path = Join-Path $root $relativePath
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            $contradictions.Add("Missing contract document: $relativePath.")
            continue
        }

        $content = [System.IO.File]::ReadAllText($path)
        foreach ($required in @($document.required)) {
            if (-not $content.Contains([string]$required)) {
                $contradictions.Add("$relativePath is missing required Linux/WSL2 contract text: $required")
            }
        }
    }

    if ($CheckGitHubPrd) {
        $gh = Get-Command gh -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -eq $gh) {
            $contradictions.Add('gh is required to check the current GitHub PRD.')
        } else {
            foreach ($issue in @($contract.githubPrd)) {
                $body = (& $gh.Source issue view ([string]$issue.issue) --repo ([string]$contract.repository) --json body --jq .body 2>$null | Out-String)
                if ($LASTEXITCODE -ne 0) {
                    $contradictions.Add("Could not read GitHub Issue #$($issue.issue).")
                    continue
                }
                foreach ($required in @($issue.required)) {
                    if (-not $body.Contains([string]$required)) {
                        $contradictions.Add("GitHub Issue #$($issue.issue) is missing required Linux/WSL2 contract text: $required")
                    }
                }
            }
        }
    }
}

$report = [ordered]@{
    passed = $contradictions.Count -eq 0
    issue = 35
    parentIssue = 29
    githubPrd = if ($CheckGitHubPrd) { 'checked' } else { 'not_run' }
    contradictions = @($contradictions)
    checkedDocuments = @($checkedDocuments)
}

$report | ConvertTo-Json -Depth 6
if (-not $report.passed) {
    exit 1
}
