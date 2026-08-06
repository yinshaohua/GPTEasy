[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string[]]$Scripts,

    [Parameter(Mandatory = $true)]
    [string]$Workflow,

    [string]$EvidenceWorkflow
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Throw-ContractFailure {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Reason
    )

    throw [System.InvalidOperationException]::new($Reason)
}

function Assert-Condition {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition,

        [Parameter(Mandatory = $true)]
        [string]$Reason
    )

    if (-not $Condition) {
        Throw-ContractFailure -Reason $Reason
    }
}

function Resolve-RepositoryPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $repositoryRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $PSScriptRoot "..\..")
    )
    $candidate = if ([System.IO.Path]::IsPathRooted($Path)) {
        [System.IO.Path]::GetFullPath($Path)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $Path))
    }
    $prefix = $repositoryRoot.TrimEnd("\", "/") +
        [System.IO.Path]::DirectorySeparatorChar
    Assert-Condition `
        -Condition (
            $candidate.Equals(
                $repositoryRoot,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -or
            $candidate.StartsWith(
                $prefix,
                [System.StringComparison]::OrdinalIgnoreCase
            )
        ) `
        -Reason "contract input resolves outside the repository"
    return $candidate
}

function Read-ContractText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $resolved = Resolve-RepositoryPath -Path $Path
    Assert-Condition `
        -Condition (Test-Path -LiteralPath $resolved -PathType Leaf) `
        -Reason ("required contract file is missing: " + $Path)
    $bytes = [System.IO.File]::ReadAllBytes($resolved)
    Assert-Condition `
        -Condition (-not ($bytes -contains 13)) `
        -Reason ("contract file must use Unix line endings: " + $Path)
    return [System.Text.UTF8Encoding]::new($false, $true).GetString($bytes)
}

function Assert-Match {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text,

        [Parameter(Mandatory = $true)]
        [string]$Pattern,

        [Parameter(Mandatory = $true)]
        [string]$Reason
    )

    Assert-Condition `
        -Condition ([regex]::IsMatch(
            $Text,
            $Pattern,
            [System.Text.RegularExpressions.RegexOptions]::Multiline
        )) `
        -Reason $Reason
}

function Expand-ScriptArguments {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Values
    )

    return @(
        foreach ($value in $Values) {
            foreach ($part in ($value -split ",")) {
                $trimmed = $part.Trim()
                if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
                    $trimmed
                }
            }
        }
    )
}

try {
    $expandedScripts = @(Expand-ScriptArguments -Values $Scripts)
    Assert-Condition `
        -Condition ($expandedScripts.Count -ge 3) `
        -Reason "Wave 0 must receive the probe and verifier zsh scripts"

    $scriptTexts = @{}
    foreach ($scriptPath in $expandedScripts) {
        $text = Read-ContractText -Path $scriptPath
        Assert-Match `
            -Text $text `
            -Pattern "\A#!/bin/zsh\n" `
            -Reason ("zsh script must use the native zsh shebang: " + $scriptPath)
        Assert-Match `
            -Text $text `
            -Pattern "(?m)^set -euo pipefail$" `
            -Reason ("zsh script must fail closed: " + $scriptPath)
        $scriptTexts[[System.IO.Path]::GetFileName($scriptPath)] = $text
    }

    foreach ($requiredScript in @(
        "probe-codex-macos.zsh",
        "probe-macos-host.zsh",
        "test-macos-contract-probes.zsh"
    )) {
        Assert-Condition `
            -Condition $scriptTexts.ContainsKey($requiredScript) `
            -Reason ("missing required Wave 0 script input: " + $requiredScript)
    }

    $codexProbe = [string]$scriptTexts["probe-codex-macos.zsh"]
    foreach ($contract in @(
        "Darwin",
        "sw_vers",
        "uname -m",
        "0.146.1",
        "generate-json-schema",
        "includeLayers",
        "config_root_category",
        "model_sha256",
        "provider_sha256",
        "origin_sha256",
        "credential_carrier",
        "shared_user_layer"
    )) {
        Assert-Condition `
            -Condition $codexProbe.Contains($contract) `
            -Reason ("Codex probe is missing contract marker: " + $contract)
    }
    foreach ($method in @("initialize", "initialized", "config/read")) {
        Assert-Match `
            -Text $codexProbe `
            -Pattern (
                '\\?"method\\?"\s*:\s*\\?"' +
                [regex]::Escape($method) +
                '\\?"'
            ) `
            -Reason ("Codex probe is missing app-server method: " + $method)
    }

    $hostProbe = [string]$scriptTexts["probe-macos-host.zsh"]
    foreach ($contract in @(
        "/Applications/Codex.app",
        "/Applications/ChatGPT.app",
        '$HOME/Applications/Codex.app',
        '$HOME/Applications/ChatGPT.app',
        "Contents/Resources/codex",
        "GPTEASY-CONTRACT-CANARY-NONSECRET-01-12",
        "official_cli",
        "bundled_host",
        "config_root",
        "model_digest",
        "provider_digest",
        "origin_digest",
        "credential_carrier",
        "shared_user_layer"
    )) {
        Assert-Condition `
            -Condition $hostProbe.Contains($contract) `
            -Reason ("host parity probe is missing contract marker: " + $contract)
    }

    $probeTests = [string]$scriptTexts["test-macos-contract-probes.zsh"]
    foreach ($syntaxTarget in @(
        "probe-codex-macos.zsh",
        "probe-macos-host.zsh",
        "test-macos-contract-probes.zsh"
    )) {
        Assert-Condition `
            -Condition (
                $probeTests.Contains("zsh -n") -and
                $probeTests.Contains($syntaxTarget)
            ) `
            -Reason ("zsh verifier must syntax-check: " + $syntaxTarget)
    }
    foreach ($fixtureCase in @(
        "positive",
        "host_missing",
        "wrong_arch",
        "root_mismatch",
        "origin_mismatch",
        "provider_mismatch",
        "carrier_mismatch"
    )) {
        Assert-Condition `
            -Condition $probeTests.Contains($fixtureCase) `
            -Reason ("zsh verifier is missing fixture case: " + $fixtureCase)
    }

    foreach ($source in @($codexProbe, $hostProbe)) {
        Assert-Condition `
            -Condition ($source -notmatch "(?i)authorization\s*:") `
            -Reason "zsh contract source must not embed Authorization output"
        Assert-Condition `
            -Condition ($source -notmatch "(?i)command[_ ]?line") `
            -Reason "zsh contract source must not expose complete command lines"
    }

    $workflowText = Read-ContractText -Path $Workflow
    foreach ($contractPattern in @(
        "(?m)^\s*workflow_call:\s*$",
        "(?m)^\s*workflow_dispatch:\s*$",
        "(?m)^\s*permissions:\s*$",
        "(?m)^\s*contents:\s*read\s*$",
        "(?m)^\s*-\s*runner:\s*macos-15\s*$",
        "(?m)^\s*expected_arch:\s*arm64\s*$",
        "(?m)^\s*-\s*runner:\s*macos-15-intel\s*$",
        "(?m)^\s*expected_arch:\s*x86_64\s*$",
        "runs-on:\s*\$\{\{\s*matrix\.runner\s*\}\}",
        "actions/checkout@[0-9a-f]{40}",
        "shell:\s*zsh \{0\}",
        "find scripts/contracts -type f -name '\*\.zsh'",
        "zsh -n",
        "zsh scripts/contracts/test-macos-contract-probes\.zsh",
        "test-macos-package-verifier\.zsh"
    )) {
        Assert-Match `
            -Text $workflowText `
            -Pattern $contractPattern `
            -Reason ("Wave 0 workflow is missing contract pattern: " + $contractPattern)
    }
    Assert-Condition `
        -Condition ($workflowText -notmatch "actions/checkout@v[0-9]+") `
        -Reason "Wave 0 checkout must be pinned to an exact commit"

    if (-not [string]::IsNullOrWhiteSpace($EvidenceWorkflow)) {
        $evidenceText = Read-ContractText -Path $EvidenceWorkflow
        Assert-Match `
            -Text $evidenceText `
            -Pattern (
                "(?ms)^\s*wave0:\s*\n" +
                "\s*uses:\s*\./\.github/workflows/phase1-macos-wave0\.yml\s*$"
            ) `
            -Reason "macOS evidence workflow must consume Wave 0 as a reusable job"
        $needsMatches = [regex]::Matches(
            $evidenceText,
            "(?m)^\s*needs:\s*wave0\s*$"
        )
        Assert-Condition `
            -Condition ($needsMatches.Count -ge 2) `
            -Reason "both native evidence jobs must depend on Wave 0"
    }

    Write-Output (
        "macOS Wave 0 dependency contract passed: reusable dual-architecture " +
        "workflow, exact checkout, zsh syntax/tests, and fail-closed fixtures"
    )
    exit 0
} catch {
    Write-Output (
        "macOS Wave 0 dependency contract failed; no native macOS result was " +
        "claimed: " + $_.Exception.Message
    )
    exit 1
}
