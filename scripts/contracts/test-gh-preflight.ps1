[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
$script:Preflight = Join-Path $PSScriptRoot "preflight-gh-evidence.ps1"
$script:FixturePath = Join-Path $script:RepositoryRoot "tests\fixtures\contracts\gh-preflight-cases.json"
$script:PowershellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
$script:TempRoot = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    ("gpteasy-gh-preflight-test-" + [guid]::NewGuid().ToString("N"))
$script:Canary = "GPTEASY-GH-TOKEN-CANARY-91D3B7"

function Throw-TestFailure {
    throw [System.InvalidOperationException]::new("gh evidence preflight self-test failed")
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

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    if ($null -eq $Object) {
        Throw-TestFailure
    }
    $actual = @($Object.PSObject.Properties | ForEach-Object { $_.Name } | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    Assert-Condition -Condition (($actual -join "`n") -ceq ($expectedSorted -join "`n"))
}

function Assert-CleanOutput {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    Assert-Condition -Condition ($Text -notmatch [regex]::Escape($script:Canary))
    Assert-Condition -Condition ($Text -notmatch "(?i)authorization\s*:")
    Assert-Condition -Condition ($Text -notmatch "(?i)bearer\s+")
    Assert-Condition -Condition ($Text -notmatch "(?i)oauth_token|gh_token|github_token")
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

    Write-Utf8NoBom -Path $Path -Content (($Value | ConvertTo-Json -Depth 20) + "`n")
}

function Copy-JsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return (($Value | ConvertTo-Json -Depth 20 -Compress) | ConvertFrom-Json)
}

function Invoke-Preflight {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Fixture,

        [Parameter(Mandatory = $true)]
        [string]$Case
    )

    $outputLines = @(
        & $script:PowershellExecutable `
            -NoProfile `
            -ExecutionPolicy Bypass `
            -File $script:Preflight `
            -Repository "yinshaohua/GPTEasy" `
            -MinimumVersion "2.49.0" `
            -GhExecutable "gh-must-not-run" `
            -GhFixture $Fixture `
            -FixtureCase $Case 2>&1
    )
    $exitCode = [int]$LASTEXITCODE
    $output = ($outputLines -join "`n").Trim()
    Assert-CleanOutput -Text $output
    try {
        $document = $output | ConvertFrom-Json
    } catch {
        Throw-TestFailure
    }

    Assert-ExactProperties -Object $document -Expected @(
        "schema_version",
        "repository",
        "minimum_version",
        "detected_version",
        "outcome",
        "exit_code",
        "strict_gate_eligible",
        "test_only",
        "artifact_verified",
        "checks",
        "blocking_reasons"
    )
    foreach ($check in @($document.checks)) {
        Assert-ExactProperties -Object $check -Expected @("name", "outcome", "code")
    }

    return [pscustomobject]@{
        ExitCode = $exitCode
        Document = $document
        Output = $output
    }
}

function Assert-BlockedCase {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Result,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedCode
    )

    Assert-Condition -Condition ($Result.ExitCode -eq 3)
    Assert-Condition -Condition ([int]$Result.Document.exit_code -eq 3)
    Assert-Condition -Condition ([string]$Result.Document.outcome -ceq "blocked")
    Assert-Condition -Condition (-not [bool]$Result.Document.strict_gate_eligible)
    Assert-Condition -Condition ([bool]$Result.Document.test_only)
    Assert-Condition -Condition (-not [bool]$Result.Document.artifact_verified)
    Assert-Condition -Condition (@($Result.Document.blocking_reasons).Count -eq 1)
    Assert-Condition -Condition (
        [string]$Result.Document.blocking_reasons[0] -ceq $ExpectedCode
    )
    $failedChecks = @(
        $Result.Document.checks |
            Where-Object { [string]$_.outcome -ceq "blocked" }
    )
    Assert-Condition -Condition ($failedChecks.Count -eq 1)
    Assert-Condition -Condition ([string]$failedChecks[0].code -ceq $ExpectedCode)
}

try {
    Assert-Condition -Condition (Test-Path -LiteralPath $script:Preflight -PathType Leaf)
    Assert-Condition -Condition (Test-Path -LiteralPath $script:FixturePath -PathType Leaf)
    New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null

    $fixture = [System.IO.File]::ReadAllText($script:FixturePath) | ConvertFrom-Json
    Assert-ExactProperties -Object $fixture -Expected @(
        "schema_version",
        "repository",
        "minimum_version",
        "attestation_probe_digest",
        "positive_control",
        "cases"
    )
    Assert-Condition -Condition ([int]$fixture.schema_version -eq 1)
    Assert-Condition -Condition ([string]$fixture.repository -ceq "yinshaohua/GPTEasy")
    Assert-Condition -Condition ([string]$fixture.minimum_version -ceq "2.49.0")

    $positive = Invoke-Preflight -Fixture $script:FixturePath -Case "positive"
    Assert-Condition -Condition ($positive.ExitCode -eq 0)
    Assert-Condition -Condition ([int]$positive.Document.exit_code -eq 0)
    Assert-Condition -Condition ([string]$positive.Document.outcome -ceq "passed")
    Assert-Condition -Condition (-not [bool]$positive.Document.strict_gate_eligible)
    Assert-Condition -Condition ([bool]$positive.Document.test_only)
    Assert-Condition -Condition (-not [bool]$positive.Document.artifact_verified)
    Assert-Condition -Condition (@($positive.Document.blocking_reasons).Count -eq 0)
    Assert-Condition -Condition (@($positive.Document.checks).Count -eq 7)
    Assert-Condition -Condition (
        [string]$positive.Document.checks[-1].code -ceq "GH_ATTESTATION_NOT_FOUND_AUTHORIZED"
    )

    $cases = @($fixture.cases)
    Assert-Condition -Condition ($cases.Count -ge 10)
    foreach ($case in $cases) {
        $result = Invoke-Preflight `
            -Fixture $script:FixturePath `
            -Case ([string]$case.name)
        Assert-BlockedCase `
            -Result $result `
            -ExpectedCode ([string]$case.expected_error_code)
    }

    foreach ($requiredCase in @(
        "old-version",
        "unauthenticated",
        "repository-forbidden",
        "actions-runs-forbidden",
        "actions-artifacts-forbidden",
        "attestation-forbidden"
    )) {
        Assert-Condition -Condition (
            @($cases | Where-Object { [string]$_.name -ceq $requiredCase }).Count -eq 1
        )
    }

    foreach ($permissionCase in @(
        "actions-runs-unauthorized",
        "actions-runs-forbidden",
        "actions-artifacts-unauthorized",
        "actions-artifacts-forbidden",
        "attestation-unauthorized",
        "attestation-forbidden"
    )) {
        $case = @($cases | Where-Object { [string]$_.name -ceq $permissionCase })[0]
        $override = @($case.overrides)[0]
        Assert-Condition -Condition (
            [string]$override.stderr -match "HTTP\s+(401|403)"
        )
    }

    $maliciousFixture = Copy-JsonObject -Value $fixture
    $maliciousCase = @(
        $maliciousFixture.cases |
            Where-Object { [string]$_.name -ceq "unauthenticated" }
    )
    Assert-Condition -Condition ($maliciousCase.Count -eq 1)
    $maliciousOverride = @($maliciousCase[0].overrides)[0]
    $maliciousOverride.stderr = @(
        "gh auth status failed"
        "Authorization: Bearer $($script:Canary)"
        "oauth_token=$($script:Canary)"
        "hosts.yml: $($script:Canary)"
    ) -join "`n"
    $maliciousFixturePath = Join-Path $script:TempRoot "malicious-gh-preflight.json"
    Write-JsonFile -Path $maliciousFixturePath -Value $maliciousFixture

    $maliciousResult = Invoke-Preflight `
        -Fixture $maliciousFixturePath `
        -Case "unauthenticated"
    Assert-BlockedCase -Result $maliciousResult -ExpectedCode "GH_AUTH_REQUIRED"
    Assert-Condition -Condition (
        [string]$maliciousResult.Output -cnotmatch [regex]::Escape($script:Canary)
    )

    $source = [System.IO.File]::ReadAllText($script:Preflight)
    Assert-Condition -Condition ($source -match '2\.49\.0')
    Assert-Condition -Condition ($source -match 'yinshaohua/GPTEasy')
    Assert-Condition -Condition ($source -match 'attestation", "verify", "--help')
    Assert-Condition -Condition ($source -match 'actions/runs\?per_page=1')
    Assert-Condition -Condition ($source -match 'actions/artifacts\?per_page=1')
    Assert-Condition -Condition ($source -match 'attestations/\$\(\$script:AttestationProbeDigest\)')
    Assert-Condition -Condition ($source -notmatch "(?i)--show-token")
    Assert-Condition -Condition ($source -notmatch '(?i)"auth",\s*"token"')

    Write-Output (
        "gh evidence preflight self-test passed: 404 permission probe and {0} fail-closed cases" -f
        $cases.Count
    )
    exit 0
} catch {
    Write-Output "gh evidence preflight self-test failed; sensitive gh output is not emitted."
    exit 1
} finally {
    if (Test-Path -LiteralPath $script:TempRoot -PathType Container) {
        Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
