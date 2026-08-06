[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$Repository = "yinshaohua/GPTEasy",

    [Parameter(Mandatory = $false)]
    [string]$MinimumVersion = "2.49.0",

    [Parameter(Mandatory = $false)]
    [string]$GhExecutable = "gh",

    [Parameter(Mandatory = $false)]
    [string]$GhFixture,

    [Parameter(Mandatory = $false)]
    [string]$FixtureCase = "positive",

    [Parameter(Mandatory = $false)]
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:FixedRepository = "yinshaohua/GPTEasy"
$script:FixedMinimumVersion = "2.49.0"
$script:GitHubHostname = "github.com"
$script:ApiVersion = "2022-11-28"
$script:AttestationProbeDigest = "sha256:" + ("0" * 64)
$script:TempRoot = $null
$script:ActiveTranscript = $null
$script:Checks = $null
$script:DetectedVersion = ""
$script:TestOnly = $false

function Throw-PreflightBlocked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Check,

        [Parameter(Mandatory = $true)]
        [string]$Code
    )

    $exception = [System.InvalidOperationException]::new("gh evidence preflight blocked")
    $exception.Data["PreflightCheck"] = $Check
    $exception.Data["PreflightCode"] = $Code
    throw $exception
}

function Throw-FixtureInvalid {
    Throw-PreflightBlocked -Check "fixture" -Code "GH_FIXTURE_INVALID"
}

function Add-Check {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [ValidateSet("passed", "blocked")]
        [string]$Outcome,

        [Parameter(Mandatory = $true)]
        [string]$Code
    )

    $script:Checks.Add([ordered]@{
        name = $Name
        outcome = $Outcome
        code = $Code
    })
}

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    if ($null -eq $Object) {
        Throw-FixtureInvalid
    }
    $actual = @($Object.PSObject.Properties | ForEach-Object { $_.Name } | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if (($actual -join "`n") -cne ($expectedSorted -join "`n")) {
        Throw-FixtureInvalid
    }
}

function Read-JsonFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Throw-FixtureInvalid
    }
    try {
        return [System.IO.File]::ReadAllText($Path) | ConvertFrom-Json
    } catch {
        Throw-FixtureInvalid
    }
}

function Copy-JsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return (($Value | ConvertTo-Json -Depth 20 -Compress) | ConvertFrom-Json)
}

function Assert-StringSequence {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Actual,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    $actualStrings = @($Actual | ForEach-Object { [string]$_ })
    if (($actualStrings -join "`n") -cne ($Expected -join "`n")) {
        Throw-FixtureInvalid
    }
}

function Read-FixtureContract {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $fixture = Read-JsonFile -Path $Path
    Assert-ExactProperties -Object $fixture -Expected @(
        "schema_version",
        "repository",
        "minimum_version",
        "attestation_probe_digest",
        "positive_control",
        "cases"
    )
    if ([int]$fixture.schema_version -ne 1 -or
        [string]$fixture.repository -cne $script:FixedRepository -or
        [string]$fixture.minimum_version -cne $script:FixedMinimumVersion -or
        [string]$fixture.attestation_probe_digest -cne $script:AttestationProbeDigest) {
        Throw-FixtureInvalid
    }

    Assert-ExactProperties -Object $fixture.positive_control -Expected @(
        "name",
        "expected_outcome",
        "expected_error_code",
        "commands"
    )
    if ([string]$fixture.positive_control.expected_outcome -cne "passed" -or
        -not [string]::IsNullOrEmpty([string]$fixture.positive_control.expected_error_code)) {
        Throw-FixtureInvalid
    }

    $commandIds = New-Object System.Collections.Generic.HashSet[string]
    foreach ($command in @($fixture.positive_control.commands)) {
        Assert-ExactProperties -Object $command -Expected @(
            "id",
            "arguments",
            "exit_code",
            "stdout",
            "stderr"
        )
        $id = [string]$command.id
        if ([string]::IsNullOrWhiteSpace($id) -or -not $commandIds.Add($id)) {
            Throw-FixtureInvalid
        }
        if ([int]$command.exit_code -lt 0) {
            Throw-FixtureInvalid
        }
        foreach ($argument in @($command.arguments)) {
            if ([string]::IsNullOrWhiteSpace([string]$argument)) {
                Throw-FixtureInvalid
            }
        }
    }

    $expectedCommandIds = @(
        "version",
        "attestation_help",
        "auth_status",
        "repository_read",
        "actions_runs_read",
        "actions_artifacts_read",
        "attestation_read"
    )
    if ((@($commandIds | Sort-Object) -join "`n") -cne (@($expectedCommandIds | Sort-Object) -join "`n")) {
        Throw-FixtureInvalid
    }

    $caseNames = New-Object System.Collections.Generic.HashSet[string]
    foreach ($case in @($fixture.cases)) {
        Assert-ExactProperties -Object $case -Expected @(
            "name",
            "expected_outcome",
            "expected_error_code",
            "overrides"
        )
        $caseName = [string]$case.name
        if ([string]::IsNullOrWhiteSpace($caseName) -or
            -not $caseNames.Add($caseName) -or
            [string]$case.expected_outcome -cne "blocked" -or
            [string]::IsNullOrWhiteSpace([string]$case.expected_error_code)) {
            Throw-FixtureInvalid
        }
        foreach ($override in @($case.overrides)) {
            Assert-ExactProperties -Object $override -Expected @(
                "id",
                "exit_code",
                "stdout",
                "stderr"
            )
            if (-not $commandIds.Contains([string]$override.id) -or [int]$override.exit_code -lt 0) {
                Throw-FixtureInvalid
            }
        }
    }
    if ($caseNames.Count -lt 10) {
        Throw-FixtureInvalid
    }

    return $fixture
}

function Get-FixtureTranscript {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Fixture,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $positive = Copy-JsonObject -Value $Fixture.positive_control
    if ($Name -ceq "positive") {
        return $positive
    }

    $matches = @($Fixture.cases | Where-Object { [string]$_.name -ceq $Name })
    if ($matches.Count -ne 1) {
        Throw-FixtureInvalid
    }
    $case = $matches[0]
    foreach ($override in @($case.overrides)) {
        $commands = @($positive.commands | Where-Object { [string]$_.id -ceq [string]$override.id })
        if ($commands.Count -ne 1) {
            Throw-FixtureInvalid
        }
        $commands[0].exit_code = [int]$override.exit_code
        $commands[0].stdout = [string]$override.stdout
        $commands[0].stderr = [string]$override.stderr
    }
    $positive.name = [string]$case.name
    $positive.expected_outcome = [string]$case.expected_outcome
    $positive.expected_error_code = [string]$case.expected_error_code
    return $positive
}

function Invoke-GhCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Id,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    if ($null -ne $script:ActiveTranscript) {
        $matches = @(
            $script:ActiveTranscript.commands |
                Where-Object { [string]$_.id -ceq $Id }
        )
        if ($matches.Count -ne 1) {
            Throw-FixtureInvalid
        }
        $command = $matches[0]
        Assert-StringSequence -Actual @($command.arguments) -Expected $Arguments
        return [pscustomobject]@{
            ExitCode = [int]$command.exit_code
            Output = [string]$command.stdout
            Error = [string]$command.stderr
        }
    }

    if ($null -eq $script:TempRoot) {
        Throw-PreflightBlocked -Check $Id -Code "GH_EXECUTABLE_UNAVAILABLE"
    }
    $errorPath = Join-Path $script:TempRoot ("gh-error-" + [guid]::NewGuid().ToString("N") + ".txt")
    try {
        $outputLines = @(& $GhExecutable @Arguments 2> $errorPath)
        $exitCode = [int]$LASTEXITCODE
        $errorText = ""
        if (Test-Path -LiteralPath $errorPath -PathType Leaf) {
            $errorText = [System.IO.File]::ReadAllText($errorPath)
        }
        return [pscustomobject]@{
            ExitCode = $exitCode
            Output = ($outputLines -join "`n")
            Error = $errorText
        }
    } catch {
        Throw-PreflightBlocked -Check $Id -Code "GH_EXECUTABLE_UNAVAILABLE"
    } finally {
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}

function Get-HttpStatus {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Result
    )

    $combined = ([string]$Result.Error) + "`n" + ([string]$Result.Output)
    if ($combined -match "(?i)HTTP\s+(?<status>\d{3})") {
        return [int]$Matches.status
    }
    return 0
}

function Read-AllowlistedJson {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Result,

        [Parameter(Mandatory = $true)]
        [string]$Check,

        [Parameter(Mandatory = $true)]
        [string]$FailureCode
    )

    try {
        return ([string]$Result.Output | ConvertFrom-Json)
    } catch {
        Throw-PreflightBlocked -Check $Check -Code $FailureCode
    }
}

function Assert-NonNegativeCount {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value,

        [Parameter(Mandatory = $true)]
        [string]$Check,

        [Parameter(Mandatory = $true)]
        [string]$FailureCode
    )

    try {
        $count = [int64]$Value
    } catch {
        Throw-PreflightBlocked -Check $Check -Code $FailureCode
    }
    if ($count -lt 0) {
        Throw-PreflightBlocked -Check $Check -Code $FailureCode
    }
}

function Get-ApiArguments {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Endpoint,

        [Parameter(Mandatory = $true)]
        [string]$Jq
    )

    return @(
        "api",
        $Endpoint,
        "--method",
        "GET",
        "--header",
        "Accept: application/vnd.github+json",
        "--header",
        "X-GitHub-Api-Version: $($script:ApiVersion)",
        "--jq",
        $Jq
    )
}

function Test-VersionAtLeast {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Actual,

        [Parameter(Mandatory = $true)]
        [string]$Minimum
    )

    try {
        $actualVersion = [System.Version]::Parse($Actual)
        $minimumVersion = [System.Version]::Parse($Minimum)
        return $actualVersion.CompareTo($minimumVersion) -ge 0
    } catch {
        return $false
    }
}

function Invoke-PreflightEvaluation {
    param(
        [Parameter(Mandatory = $false)]
        [AllowNull()]
        [object]$Transcript
    )

    $script:ActiveTranscript = $Transcript
    $script:TestOnly = $null -ne $Transcript
    $script:Checks = New-Object System.Collections.Generic.List[object]
    $script:DetectedVersion = ""
    $blockingReasons = New-Object System.Collections.Generic.List[string]
    $outcome = "blocked"
    $exitCode = 3

    try {
        if ($Repository -cne $script:FixedRepository -or
            $MinimumVersion -cne $script:FixedMinimumVersion) {
            Throw-PreflightBlocked -Check "policy" -Code "GH_POLICY_INVALID"
        }

        $versionResult = Invoke-GhCommand -Id "version" -Arguments @("version")
        if ($versionResult.ExitCode -ne 0) {
            Throw-PreflightBlocked -Check "gh_version" -Code "GH_VERSION_COMMAND_FAILED"
        }
        if ([string]$versionResult.Output -notmatch "(?im)^gh version (?<version>\d+\.\d+\.\d+)(?:-[^\s]+)?") {
            Throw-PreflightBlocked -Check "gh_version" -Code "GH_VERSION_INVALID"
        }
        $script:DetectedVersion = [string]$Matches.version
        if (-not (Test-VersionAtLeast -Actual $script:DetectedVersion -Minimum $script:FixedMinimumVersion)) {
            Throw-PreflightBlocked -Check "gh_version" -Code "GH_VERSION_TOO_OLD"
        }
        Add-Check -Name "gh_version" -Outcome "passed" -Code "GH_VERSION_OK"

        $attestationHelp = Invoke-GhCommand `
            -Id "attestation_help" `
            -Arguments @("attestation", "verify", "--help")
        if ($attestationHelp.ExitCode -ne 0 -or
            [string]$attestationHelp.Output -notmatch "(?i)gh attestation verify") {
            Throw-PreflightBlocked `
                -Check "gh_attestation_command" `
                -Code "GH_ATTESTATION_COMMAND_UNAVAILABLE"
        }
        Add-Check `
            -Name "gh_attestation_command" `
            -Outcome "passed" `
            -Code "GH_ATTESTATION_COMMAND_OK"

        $authResult = Invoke-GhCommand `
            -Id "auth_status" `
            -Arguments @("auth", "status", "--hostname", $script:GitHubHostname)
        if ($authResult.ExitCode -ne 0) {
            Throw-PreflightBlocked -Check "gh_auth" -Code "GH_AUTH_REQUIRED"
        }
        Add-Check -Name "gh_auth" -Outcome "passed" -Code "GH_AUTH_OK"

        $repositoryResult = Invoke-GhCommand `
            -Id "repository_read" `
            -Arguments (Get-ApiArguments `
                -Endpoint "repos/$($script:FixedRepository)" `
                -Jq "{full_name: .full_name}")
        if ($repositoryResult.ExitCode -ne 0) {
            Throw-PreflightBlocked `
                -Check "repository_read" `
                -Code "GH_REPOSITORY_READ_BLOCKED"
        }
        $repositoryDocument = Read-AllowlistedJson `
            -Result $repositoryResult `
            -Check "repository_read" `
            -FailureCode "GH_REPOSITORY_RESPONSE_INVALID"
        if ([string]$repositoryDocument.full_name -cne $script:FixedRepository) {
            Throw-PreflightBlocked `
                -Check "repository_read" `
                -Code "GH_REPOSITORY_RESPONSE_INVALID"
        }
        Add-Check `
            -Name "repository_read" `
            -Outcome "passed" `
            -Code "GH_REPOSITORY_READ_OK"

        $runsResult = Invoke-GhCommand `
            -Id "actions_runs_read" `
            -Arguments (Get-ApiArguments `
                -Endpoint "repos/$($script:FixedRepository)/actions/runs?per_page=1" `
                -Jq "{total_count: .total_count}")
        if ($runsResult.ExitCode -ne 0) {
            Throw-PreflightBlocked `
                -Check "actions_runs_read" `
                -Code "GH_ACTIONS_RUNS_READ_BLOCKED"
        }
        $runsDocument = Read-AllowlistedJson `
            -Result $runsResult `
            -Check "actions_runs_read" `
            -FailureCode "GH_ACTIONS_RUNS_RESPONSE_INVALID"
        Assert-NonNegativeCount `
            -Value $runsDocument.total_count `
            -Check "actions_runs_read" `
            -FailureCode "GH_ACTIONS_RUNS_RESPONSE_INVALID"
        Add-Check `
            -Name "actions_runs_read" `
            -Outcome "passed" `
            -Code "GH_ACTIONS_RUNS_READ_OK"

        $artifactsResult = Invoke-GhCommand `
            -Id "actions_artifacts_read" `
            -Arguments (Get-ApiArguments `
                -Endpoint "repos/$($script:FixedRepository)/actions/artifacts?per_page=1" `
                -Jq "{total_count: .total_count}")
        if ($artifactsResult.ExitCode -ne 0) {
            Throw-PreflightBlocked `
                -Check "actions_artifacts_read" `
                -Code "GH_ACTIONS_ARTIFACTS_READ_BLOCKED"
        }
        $artifactsDocument = Read-AllowlistedJson `
            -Result $artifactsResult `
            -Check "actions_artifacts_read" `
            -FailureCode "GH_ACTIONS_ARTIFACTS_RESPONSE_INVALID"
        Assert-NonNegativeCount `
            -Value $artifactsDocument.total_count `
            -Check "actions_artifacts_read" `
            -FailureCode "GH_ACTIONS_ARTIFACTS_RESPONSE_INVALID"
        Add-Check `
            -Name "actions_artifacts_read" `
            -Outcome "passed" `
            -Code "GH_ACTIONS_ARTIFACTS_READ_OK"

        $attestationResult = Invoke-GhCommand `
            -Id "attestation_read" `
            -Arguments (Get-ApiArguments `
                -Endpoint "repos/$($script:FixedRepository)/attestations/$($script:AttestationProbeDigest)" `
                -Jq "{attestation_count: (.attestations | length)}")
        if ($attestationResult.ExitCode -eq 0) {
            $attestationDocument = Read-AllowlistedJson `
                -Result $attestationResult `
                -Check "attestation_read" `
                -FailureCode "GH_ATTESTATION_RESPONSE_INVALID"
            Assert-NonNegativeCount `
                -Value $attestationDocument.attestation_count `
                -Check "attestation_read" `
                -FailureCode "GH_ATTESTATION_RESPONSE_INVALID"
            Add-Check `
                -Name "attestation_read" `
                -Outcome "passed" `
                -Code "GH_ATTESTATION_READ_OK"
        } else {
            $httpStatus = Get-HttpStatus -Result $attestationResult
            if ($httpStatus -ne 404) {
                Throw-PreflightBlocked `
                    -Check "attestation_read" `
                    -Code "GH_ATTESTATION_READ_BLOCKED"
            }
            Add-Check `
                -Name "attestation_read" `
                -Outcome "passed" `
                -Code "GH_ATTESTATION_NOT_FOUND_AUTHORIZED"
        }

        $outcome = "passed"
        $exitCode = 0
    } catch {
        $check = [string]$_.Exception.Data["PreflightCheck"]
        $code = [string]$_.Exception.Data["PreflightCode"]
        if ([string]::IsNullOrWhiteSpace($check)) {
            $check = "preflight"
        }
        if ([string]::IsNullOrWhiteSpace($code)) {
            $code = "GH_PREFLIGHT_INTERNAL_ERROR"
        }
        Add-Check -Name $check -Outcome "blocked" -Code $code
        $blockingReasons.Add($code)
    }

    return [ordered]@{
        schema_version = 1
        repository = $script:FixedRepository
        minimum_version = $script:FixedMinimumVersion
        detected_version = $script:DetectedVersion
        outcome = $outcome
        exit_code = $exitCode
        strict_gate_eligible = ($outcome -ceq "passed" -and -not $script:TestOnly)
        test_only = $script:TestOnly
        artifact_verified = $false
        checks = @($script:Checks.ToArray())
        blocking_reasons = @($blockingReasons.ToArray())
    }
}

function Invoke-SelfTest {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Fixture
    )

    $caseCount = 0
    $positive = Get-FixtureTranscript -Fixture $Fixture -Name "positive"
    $positiveResult = Invoke-PreflightEvaluation -Transcript $positive
    if ([string]$positiveResult.outcome -cne "passed" -or
        [int]$positiveResult.exit_code -ne 0 -or
        [bool]$positiveResult.strict_gate_eligible -or
        -not [bool]$positiveResult.test_only -or
        [bool]$positiveResult.artifact_verified -or
        [string]$positiveResult.checks[-1].code -cne "GH_ATTESTATION_NOT_FOUND_AUTHORIZED") {
        Throw-FixtureInvalid
    }

    foreach ($case in @($Fixture.cases)) {
        $transcript = Get-FixtureTranscript -Fixture $Fixture -Name ([string]$case.name)
        $result = Invoke-PreflightEvaluation -Transcript $transcript
        if ([string]$result.outcome -cne [string]$case.expected_outcome -or
            [int]$result.exit_code -ne 3 -or
            [bool]$result.strict_gate_eligible -or
            -not [bool]$result.test_only -or
            @($result.blocking_reasons).Count -ne 1 -or
            [string]$result.blocking_reasons[0] -cne [string]$case.expected_error_code) {
            Throw-FixtureInvalid
        }
        $caseCount++
    }

    return [ordered]@{
        schema_version = 1
        outcome = "passed"
        exit_code = 0
        strict_gate_eligible = $false
        test_only = $true
        artifact_verified = $false
        checks = @(
            [ordered]@{
                name = "fixture_contract"
                outcome = "passed"
                code = "GH_FIXTURE_CONTRACT_OK"
            },
            [ordered]@{
                name = "preflight_cases"
                outcome = "passed"
                code = "GH_PREFLIGHT_CASES_OK"
            }
        )
        case_count = $caseCount
        blocking_reasons = @()
    }
}

$result = $null
$exitCode = 3
try {
    if ($Repository -cne $script:FixedRepository -or
        $MinimumVersion -cne $script:FixedMinimumVersion) {
        $result = [ordered]@{
            schema_version = 1
            repository = $script:FixedRepository
            minimum_version = $script:FixedMinimumVersion
            detected_version = ""
            outcome = "blocked"
            exit_code = 64
            strict_gate_eligible = $false
            test_only = $false
            artifact_verified = $false
            checks = @(
                [ordered]@{
                    name = "policy"
                    outcome = "blocked"
                    code = "GH_POLICY_INVALID"
                }
            )
            blocking_reasons = @("GH_POLICY_INVALID")
        }
        $exitCode = 64
    } elseif ($SelfTest) {
        $fixturePath = if ([string]::IsNullOrWhiteSpace($GhFixture)) {
            Join-Path (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path `
                "tests\fixtures\contracts\gh-preflight-cases.json"
        } else {
            (Resolve-Path -LiteralPath $GhFixture -ErrorAction Stop).Path
        }
        $fixture = Read-FixtureContract -Path $fixturePath
        $result = Invoke-SelfTest -Fixture $fixture
        $exitCode = 0
    } elseif (-not [string]::IsNullOrWhiteSpace($GhFixture)) {
        $fixturePath = (Resolve-Path -LiteralPath $GhFixture -ErrorAction Stop).Path
        $fixture = Read-FixtureContract -Path $fixturePath
        $transcript = Get-FixtureTranscript -Fixture $fixture -Name $FixtureCase
        $result = Invoke-PreflightEvaluation -Transcript $transcript
        $exitCode = [int]$result.exit_code
    } else {
        $script:TempRoot = Join-Path `
            ([System.IO.Path]::GetTempPath()) `
            ("gpteasy-gh-preflight-" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null
        $result = Invoke-PreflightEvaluation
        $exitCode = [int]$result.exit_code
    }
} catch {
    $result = [ordered]@{
        schema_version = 1
        repository = $script:FixedRepository
        minimum_version = $script:FixedMinimumVersion
        detected_version = ""
        outcome = "blocked"
        exit_code = 3
        strict_gate_eligible = $false
        test_only = (-not [string]::IsNullOrWhiteSpace($GhFixture) -or $SelfTest)
        artifact_verified = $false
        checks = @(
            [ordered]@{
                name = "fixture"
                outcome = "blocked"
                code = "GH_FIXTURE_INVALID"
            }
        )
        blocking_reasons = @("GH_FIXTURE_INVALID")
    }
    $exitCode = 3
} finally {
    if ($null -ne $script:TempRoot -and
        (Test-Path -LiteralPath $script:TempRoot -PathType Container)) {
        Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

[Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 10))
exit $exitCode
