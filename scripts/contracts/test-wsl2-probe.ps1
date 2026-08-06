[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:Probe = Join-Path $PSScriptRoot "probe-wsl2.ps1"
$script:PowerShellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
$script:TempRoot = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    ("gpteasy-wsl2-probe-test-" + [guid]::NewGuid().ToString("N"))

function Throw-TestFailure {
    throw [System.InvalidOperationException]::new("WSL2 probe self-test failed")
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
    $actual = @($Object.PSObject.Properties.Name | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    Assert-Condition -Condition (($actual -join "`n") -ceq ($expectedSorted -join "`n"))
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

    Write-Utf8NoBom -Path $Path -Content (($Value | ConvertTo-Json -Depth 30) + "`n")
}

function Copy-JsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return (($Value | ConvertTo-Json -Depth 30 -Compress) | ConvertFrom-Json)
}

function New-Command {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,

        [Parameter(Mandatory = $true)]
        [string[]]$OutputLines
    )

    return [ordered]@{
        arguments = @($Arguments)
        exit_code = 0
        stdout_lines = @($OutputLines)
    }
}

function New-PositiveCase {
    return [ordered]@{
        commands = @(
            (New-Command `
                -Arguments @("--version") `
                -OutputLines @("WSL version: 2.5.7.0")),
            (New-Command `
                -Arguments @("--list", "--quiet") `
                -OutputLines @("Ubuntu", "Duplicate")),
            (New-Command `
                -Arguments @("--list", "--running", "--quiet") `
                -OutputLines @("Ubuntu")),
            (New-Command `
                -Arguments @("--list", "--running", "--quiet") `
                -OutputLines @("Ubuntu"))
        )
        registrations = @(
            [ordered]@{
                registration_id = "11111111-1111-1111-1111-111111111111"
                display_name = "Ubuntu"
                default_uid = 1000
            },
            [ordered]@{
                registration_id = "22222222-2222-2222-2222-222222222222"
                display_name = "Duplicate"
                default_uid = 1001
            },
            [ordered]@{
                registration_id = "33333333-3333-3333-3333-333333333333"
                display_name = "Duplicate"
                default_uid = 1002
            }
        )
    }
}

function New-FixtureDocument {
    $positive = New-PositiveCase
    $runningDrift = Copy-JsonObject -Value $positive
    $runningDrift.commands[3].stdout_lines = @("Ubuntu", "Duplicate")

    $guestCommand = Copy-JsonObject -Value $positive
    $guestCommand.commands[2].arguments = @(
        "--distribution",
        "Ubuntu",
        "--",
        "id",
        "-un"
    )

    return [ordered]@{
        schema_version = 1
        cases = [ordered]@{
            positive = $positive
            running_drift = $runningDrift
            guest_command = $guestCommand
        }
    }
}

function Invoke-ProbeCase {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FixturePath,

        [Parameter(Mandatory = $true)]
        [string]$Case
    )

    $lines = @(
        & $script:PowerShellExecutable `
            -NoProfile `
            -ExecutionPolicy Bypass `
            -File $script:Probe `
            -FixturePath $FixturePath `
            -FixtureCase $Case 2>&1
    )
    $exitCode = [int]$LASTEXITCODE
    $output = ($lines -join "`n").Trim()
    Assert-Condition -Condition (-not [string]::IsNullOrWhiteSpace($output))
    Assert-Condition -Condition ($output -notmatch "(?i)id\s+-un")
    Assert-Condition -Condition ($output -notmatch "(?i)--distribution")
    try {
        $document = $output | ConvertFrom-Json
    } catch {
        Throw-TestFailure
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = $output
        Document = $document
    }
}

try {
    Assert-Condition -Condition (Test-Path -LiteralPath $script:Probe -PathType Leaf)
    New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null
    $fixturePath = Join-Path $script:TempRoot "wsl2-probe-fixture.json"
    Write-JsonFile -Path $fixturePath -Value (New-FixtureDocument)

    $positive = Invoke-ProbeCase -FixturePath $fixturePath -Case "positive"
    Assert-Condition -Condition ($positive.ExitCode -eq 0)
    Assert-Condition -Condition ([string]$positive.Document.outcome -ceq "passed")
    Assert-Condition -Condition ([bool]$positive.Document.test_only)
    Assert-Condition -Condition (-not [bool]$positive.Document.strict_gate_eligible)
    Assert-Condition -Condition ([int]$positive.Document.guest_command_count -eq 0)
    Assert-Condition -Condition (
        (@($positive.Document.running_before) -join "`n") -ceq "Ubuntu"
    )
    Assert-Condition -Condition (
        (@($positive.Document.running_after) -join "`n") -ceq "Ubuntu"
    )
    Assert-ExactProperties -Object $positive.Document -Expected @(
        "schema_version",
        "probe",
        "outcome",
        "exit_code",
        "strict_gate_eligible",
        "test_only",
        "wsl_version_sha256",
        "all_names",
        "running_before",
        "running_after",
        "guest_command_count",
        "registrations",
        "checks",
        "blocking_reasons"
    )
    foreach ($registration in @($positive.Document.registrations)) {
        Assert-ExactProperties -Object $registration -Expected @(
            "registration_id",
            "display_name",
            "default_uid",
            "is_running",
            "command_target_resolvable"
        )
    }

    $ubuntu = @(
        $positive.Document.registrations |
            Where-Object { [string]$_.display_name -ceq "Ubuntu" }
    )
    Assert-Condition -Condition ($ubuntu.Count -eq 1)
    Assert-Condition -Condition ([bool]$ubuntu[0].command_target_resolvable)
    Assert-Condition -Condition ([bool]$ubuntu[0].is_running)

    $duplicates = @(
        $positive.Document.registrations |
            Where-Object { [string]$_.display_name -ceq "Duplicate" }
    )
    Assert-Condition -Condition ($duplicates.Count -eq 2)
    foreach ($duplicate in $duplicates) {
        Assert-Condition -Condition (-not [bool]$duplicate.command_target_resolvable)
    }

    $drift = Invoke-ProbeCase -FixturePath $fixturePath -Case "running_drift"
    Assert-Condition -Condition ($drift.ExitCode -eq 5)
    Assert-Condition -Condition ([string]$drift.Document.outcome -ceq "failed")
    Assert-Condition -Condition (
        [string]$drift.Document.blocking_reasons[0] -ceq
        "WSL_RUNNING_SET_CHANGED"
    )

    $guest = Invoke-ProbeCase -FixturePath $fixturePath -Case "guest_command"
    Assert-Condition -Condition ($guest.ExitCode -eq 5)
    Assert-Condition -Condition ([string]$guest.Document.outcome -ceq "failed")
    Assert-Condition -Condition ([int]$guest.Document.guest_command_count -eq 1)
    Assert-Condition -Condition (
        [string]$guest.Document.blocking_reasons[0] -ceq
        "WSL_GUEST_COMMAND_FORBIDDEN"
    )

    $source = [System.IO.File]::ReadAllText($script:Probe)
    Assert-Condition -Condition (
        $source -match "StandardOutputEncoding\s*=\s*\[System\.Text\.Encoding\]::Unicode"
    )
    Assert-Condition -Condition (
        $source -match [regex]::Escape(
            "HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss"
        )
    )
    Assert-Condition -Condition ($source -match '"--version"')
    Assert-Condition -Condition ($source -match '"--list",\s*"--quiet"')
    Assert-Condition -Condition (
        $source -match '"--list",\s*"--running",\s*"--quiet"'
    )
    Assert-Condition -Condition ($source -notmatch "(?i)Invoke-Command")
    Assert-Condition -Condition ($source -notmatch "(?i)Set-ItemProperty")
    Assert-Condition -Condition ($source -notmatch "(?i)New-ItemProperty")
    Assert-Condition -Condition ($source -notmatch "(?i)Remove-ItemProperty")

    Write-Output (
        "WSL2 probe self-test passed: running set preserved, guest commands rejected, " +
        "and duplicate display names fail closed"
    )
    exit 0
} catch {
    Write-Output "WSL2 probe self-test failed; WSL command output is not emitted."
    exit 1
} finally {
    if (Test-Path -LiteralPath $script:TempRoot -PathType Container) {
        $resolvedTempRoot = (Resolve-Path -LiteralPath $script:TempRoot).Path
        $systemTempRoot = [System.IO.Path]::GetFullPath(
            [System.IO.Path]::GetTempPath()
        ).TrimEnd("\")
        if ($resolvedTempRoot.StartsWith(
            $systemTempRoot + "\",
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            Remove-Item -LiteralPath $resolvedTempRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
