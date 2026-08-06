[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
$script:Verifier = Join-Path $PSScriptRoot "verify-evidence-provenance.ps1"
$script:FixturePath = Join-Path $script:RepositoryRoot "tests\fixtures\contracts\provenance-negative-cases.json"
$script:Runner = Join-Path $PSScriptRoot "run-phase1-contracts.ps1"
$script:TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("gpteasy-provenance-test-" + [guid]::NewGuid().ToString("N"))
$script:PowershellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
$script:Canary = "GPTEASY-TOKEN-CANARY-7E1B5A"

function Throw-TestFailure {
    throw [System.InvalidOperationException]::new("evidence provenance self-test failed")
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Content
    )

    [System.IO.File]::WriteAllText(
        $Path,
        $Content,
        (New-Object System.Text.UTF8Encoding($false))
    )
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    Write-Utf8NoBom -Path $Path -Content (($Value | ConvertTo-Json -Depth 40) + "`n")
}

function Copy-JsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return (($Value | ConvertTo-Json -Depth 40 -Compress) | ConvertFrom-Json)
}

function Assert-Condition {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition
    )

    if (-not $Condition) {
        Throw-TestFailure
    }
}

function Get-PathParent {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Root,

        [Parameter(Mandatory = $true)]
        [string[]]$Segments
    )

    $current = $Root
    for ($index = 0; $index -lt $Segments.Count - 1; $index++) {
        $segment = $Segments[$index]
        if ($segment -match "^\d+$") {
            $current = @($current)[[int]$segment]
        } else {
            $property = $current.PSObject.Properties[$segment]
            if ($null -eq $property) {
                Throw-TestFailure
            }
            $current = $property.Value
        }
    }
    return [pscustomobject]@{
        Parent = $current
        Leaf = $Segments[$Segments.Count - 1]
    }
}

function Apply-Mutation {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Root,

        [Parameter(Mandatory = $true)]
        [object]$Case
    )

    $segments = @(([string]$Case.path).Split("."))
    $location = Get-PathParent -Root $Root -Segments $segments
    $operation = [string]$Case.operation
    $leaf = [string]$location.Leaf

    if ($leaf -match "^\d+$") {
        $index = [int]$leaf
        if ($operation -in @("set", "replace", "remove")) {
            $location.Parent[$index] = if ($operation -eq "remove") { $null } else { $Case.value }
            return
        }
        Throw-TestFailure
    }

    if ($operation -eq "remove") {
        $location.Parent.PSObject.Properties.Remove($leaf)
        return
    }
    if ($operation -eq "add") {
        $location.Parent | Add-Member -NotePropertyName $leaf -NotePropertyValue $Case.value
        return
    }
    if ($operation -in @("set", "replace")) {
        $property = $location.Parent.PSObject.Properties[$leaf]
        if ($null -eq $property) {
            Throw-TestFailure
        }
        $property.Value = $Case.value
        return
    }
    Throw-TestFailure
}

function Invoke-Verifier {
    param(
        [Parameter(Mandatory = $true)]
        [object]$ManifestDocument,

        [Parameter(Mandatory = $true)]
        [object]$RetrievalDocument,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $caseDirectory = Join-Path $script:TempRoot $Name
    New-Item -ItemType Directory -Path $caseDirectory -Force | Out-Null
    $manifestPath = Join-Path $caseDirectory "manifest.json"
    $transcriptPath = Join-Path $caseDirectory "transcript.json"
    Write-JsonFile -Path $manifestPath -Value $ManifestDocument
    Write-JsonFile -Path $transcriptPath -Value $RetrievalDocument

    $outputLines = @(
        & $script:PowershellExecutable `
            -NoProfile `
            -ExecutionPolicy Bypass `
            -File $script:Verifier `
            -Manifest $manifestPath `
            -Transcript $transcriptPath `
            -GhExecutable "gh-must-not-run" `
            -Now "2026-08-06T01:00:00Z" 2>&1
    )
    $exitCode = [int]$LASTEXITCODE
    $output = ($outputLines -join "`n")
    try {
        $result = $output | ConvertFrom-Json
    } catch {
        Throw-TestFailure
    }

    Assert-Condition -Condition ($output -notmatch [regex]::Escape($script:Canary))
    Assert-Condition -Condition ($output -notmatch "(?i)authorization:")
    return [pscustomobject]@{
        ExitCode = $exitCode
        Result = $result
        Output = $output
    }
}

try {
    Assert-Condition -Condition (Test-Path -LiteralPath $script:Verifier -PathType Leaf)
    Assert-Condition -Condition (Test-Path -LiteralPath $script:FixturePath -PathType Leaf)
    Assert-Condition -Condition (Test-Path -LiteralPath $script:Runner -PathType Leaf)
    New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null

    $fixture = [System.IO.File]::ReadAllText($script:FixturePath) | ConvertFrom-Json
    $positiveManifest = Copy-JsonObject -Value $fixture.positive_control.manifest
    $positiveRetrieval = Copy-JsonObject -Value $fixture.positive_control.retrieval

    $positive = Invoke-Verifier `
        -ManifestDocument $positiveManifest `
        -RetrievalDocument $positiveRetrieval `
        -Name "positive"
    Assert-Condition -Condition ($positive.ExitCode -eq 0)
    Assert-Condition -Condition ([string]$positive.Result.outcome -ceq "passed")
    Assert-Condition -Condition ([bool]$positive.Result.test_only)
    Assert-Condition -Condition (-not [bool]$positive.Result.strict_gate_eligible)
    Assert-Condition -Condition (@($positive.Result.checks).Count -eq 5)

    $cases = @($fixture.cases)
    Assert-Condition -Condition ($cases.Count -ge 17)
    foreach ($case in $cases) {
        $root = [pscustomobject]@{
            manifest = Copy-JsonObject -Value $fixture.positive_control.manifest
            retrieval = Copy-JsonObject -Value $fixture.positive_control.retrieval
        }
        Apply-Mutation -Root $root -Case $case

        if ([string]$case.name -ceq "unavailable-artifact") {
            $forgedDirectory = Join-Path $script:TempRoot "unavailable-artifact-local-fallback"
            New-Item -ItemType Directory -Path $forgedDirectory -Force | Out-Null
            Write-Utf8NoBom -Path (Join-Path $forgedDirectory "codex.exe") -Content $script:Canary
            Write-Utf8NoBom -Path (Join-Path $forgedDirectory "windows-x64-contract-evidence.zip") -Content $script:Canary
        }

        $result = Invoke-Verifier `
            -ManifestDocument $root.manifest `
            -RetrievalDocument $root.retrieval `
            -Name ([string]$case.name)
        $expectedExitCode = if ([string]$case.expected_outcome -ceq "blocked") { 3 } else { 4 }
        Assert-Condition -Condition ($result.ExitCode -eq $expectedExitCode)
        Assert-Condition -Condition ([string]$result.Result.outcome -ceq [string]$case.expected_outcome)
        Assert-Condition -Condition (-not [bool]$result.Result.strict_gate_eligible)
    }

    $source = [System.IO.File]::ReadAllText($script:Verifier)
    Assert-Condition -Condition ($source -match "gh attestation verify")
    Assert-Condition -Condition ($source -match "preflight-gh-evidence\.ps1")
    Assert-Condition -Condition ($source -match "gpteasy-provenance-")
    Assert-Condition -Condition ($source -match '(?s)"run",\s*"download"')
    Assert-Condition -Condition ($source -notmatch "(?i)fallback.+local")

    Write-Output ("evidence provenance self-test passed: independent positive control and {0} fail-closed cases" -f $cases.Count)
    exit 0
} catch {
    Write-Output "evidence provenance self-test failed; untrusted or sensitive evidence is not emitted."
    exit 1
} finally {
    if (Test-Path -LiteralPath $script:TempRoot -PathType Container) {
        Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
