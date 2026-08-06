[CmdletBinding()]
param(
    [string]$WslExecutable = "wsl.exe",

    [string]$FixturePath,

    [string]$FixtureCase
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:ExitCodes = @{
    Completed = 0
    AssertionFailed = 2
    StrictPrerequisiteBlocked = 3
    SecurityBoundaryFailed = 5
}
$script:LxssRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss"
$script:ExpectedCommandSequence = @(
    @("--version"),
    @("--list", "--quiet"),
    @("--list", "--running", "--quiet"),
    @("--list", "--running", "--quiet")
)

function Get-PropertyValue {
    param(
        [AllowNull()]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ($null -eq $Object) {
        return $null
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function ConvertTo-Boolean {
    param(
        [AllowNull()]
        [object]$Value
    )

    if ($null -eq $Value) {
        return $false
    }
    return [System.Convert]::ToBoolean($Value)
}

function Get-Sha256String {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value
    )

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Value)
        return (
            $sha256.ComputeHash($bytes) |
                ForEach-Object { $_.ToString("x2") }
        ) -join ""
    } finally {
        $sha256.Dispose()
    }
}

function ConvertTo-NormalizedLines {
    param(
        [AllowNull()]
        [object[]]$Lines
    )

    if ($null -eq $Lines) {
        return @()
    }
    return @(
        $Lines |
            ForEach-Object {
                ([string]$_).Replace([string][char]0, "").Trim()
            } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    )
}

function Test-ArgumentSequenceEqual {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Actual,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    return (($Actual -join "`n") -ceq ($Expected -join "`n"))
}

function Test-ReadOnlyWslArguments {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    foreach ($allowed in $script:ExpectedCommandSequence) {
        if (Test-ArgumentSequenceEqual -Actual $Arguments -Expected @($allowed)) {
            return $true
        }
    }
    return $false
}

function Test-GuestCommandArguments {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    foreach ($argument in $Arguments) {
        if ($argument -in @("-d", "--distribution", "-e", "--exec", "--cd", "-u", "--user")) {
            return $true
        }
    }
    return (-not (Test-ReadOnlyWslArguments -Arguments $Arguments))
}

function Invoke-LiveWslCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    if (-not (Test-ReadOnlyWslArguments -Arguments $Arguments)) {
        throw "non-read-only WSL arguments are forbidden"
    }

    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = $Arguments -join " "
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [System.Text.Encoding]::Unicode
    $info.StandardErrorEncoding = [System.Text.Encoding]::Unicode
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) {
            throw "WSL command did not start"
        }
        $stdout = $process.StandardOutput.ReadToEnd()
        $null = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        return [pscustomobject]@{
            Arguments = @($Arguments)
            ExitCode = [int]$process.ExitCode
            OutputLines = ConvertTo-NormalizedLines -Lines ($stdout -split "`r?`n")
        }
    } finally {
        $process.Dispose()
    }
}

function Read-LiveRegistrations {
    if (-not (Test-Path -LiteralPath $script:LxssRegistryPath)) {
        return @()
    }

    $registrations = foreach ($key in @(
        Get-ChildItem -LiteralPath $script:LxssRegistryPath -ErrorAction Stop
    )) {
        $registrationId = [guid]::Empty
        $validId = [guid]::TryParse([string]$key.PSChildName, [ref]$registrationId)
        $properties = Get-ItemProperty `
            -LiteralPath $key.PSPath `
            -Name "DistributionName", "DefaultUid" `
            -ErrorAction SilentlyContinue
        [pscustomobject]@{
            RegistrationId = if ($validId) {
                $registrationId.ToString("D").ToLowerInvariant()
            } else {
                $null
            }
            DisplayName = [string](
                Get-PropertyValue -Object $properties -Name "DistributionName"
            )
            DefaultUid = Get-PropertyValue -Object $properties -Name "DefaultUid"
            RegistrationIdValid = $validId
        }
    }
    return @($registrations)
}

function Read-FixtureCase {
    if ([string]::IsNullOrWhiteSpace($FixturePath) -or
        [string]::IsNullOrWhiteSpace($FixtureCase)) {
        throw "fixture path and case are both required"
    }
    if (-not (Test-Path -LiteralPath $FixturePath -PathType Leaf)) {
        throw "fixture does not exist"
    }

    $fixture = [System.IO.File]::ReadAllText($FixturePath) | ConvertFrom-Json
    if ([int](Get-PropertyValue -Object $fixture -Name "schema_version") -ne 1) {
        throw "fixture schema is unsupported"
    }
    $case = Get-PropertyValue `
        -Object (Get-PropertyValue -Object $fixture -Name "cases") `
        -Name $FixtureCase
    if ($null -eq $case) {
        throw "fixture case does not exist"
    }
    return $case
}

function Convert-FixtureCommands {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Fixture
    )

    return @(
        @(Get-PropertyValue -Object $Fixture -Name "commands") |
            ForEach-Object {
                [pscustomobject]@{
                    Arguments = @(
                        Get-PropertyValue -Object $_ -Name "arguments" |
                            ForEach-Object { [string]$_ }
                    )
                    ExitCode = [int](
                        Get-PropertyValue -Object $_ -Name "exit_code"
                    )
                    OutputLines = ConvertTo-NormalizedLines -Lines @(
                        Get-PropertyValue -Object $_ -Name "stdout_lines"
                    )
                }
            }
    )
}

function Convert-FixtureRegistrations {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Fixture
    )

    return @(
        @(Get-PropertyValue -Object $Fixture -Name "registrations") |
            ForEach-Object {
                $registrationId = [guid]::Empty
                $validId = [guid]::TryParse(
                    [string](
                        Get-PropertyValue -Object $_ -Name "registration_id"
                    ),
                    [ref]$registrationId
                )
                [pscustomobject]@{
                    RegistrationId = if ($validId) {
                        $registrationId.ToString("D").ToLowerInvariant()
                    } else {
                        $null
                    }
                    DisplayName = [string](
                        Get-PropertyValue -Object $_ -Name "display_name"
                    )
                    DefaultUid = Get-PropertyValue -Object $_ -Name "default_uid"
                    RegistrationIdValid = $validId
                }
            }
    )
}

function Test-NamePresent {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Names,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    foreach ($candidate in $Names) {
        if ($candidate.Equals(
            $Name,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            return $true
        }
    }
    return $false
}

function Get-NameCount {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Names,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $count = 0
    foreach ($candidate in $Names) {
        if ($candidate.Equals(
            $Name,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            $count++
        }
    }
    return $count
}

function Test-NameSetsEqual {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Left,

        [Parameter(Mandatory = $true)]
        [string[]]$Right
    )

    $leftCanonical = @($Left | ForEach-Object { $_.ToUpperInvariant() } | Sort-Object)
    $rightCanonical = @($Right | ForEach-Object { $_.ToUpperInvariant() } | Sort-Object)
    return (($leftCanonical -join "`n") -ceq ($rightCanonical -join "`n"))
}

function New-Check {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [bool]$Passed,

        [Parameter(Mandatory = $true)]
        [string]$FailureCode
    )

    return [ordered]@{
        name = $Name
        outcome = if ($Passed) { "passed" } else { "failed" }
        code = if ($Passed) { "OK" } else { $FailureCode }
    }
}

function Build-Result {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Commands,

        [Parameter(Mandatory = $true)]
        [object[]]$Registrations,

        [Parameter(Mandatory = $true)]
        [bool]$TestOnly
    )

    $guestCommandCount = @(
        $Commands |
            Where-Object { Test-GuestCommandArguments -Arguments @($_.Arguments) }
    ).Count
    $sequenceValid = (@($Commands).Count -eq $script:ExpectedCommandSequence.Count)
    if ($sequenceValid) {
        for ($index = 0; $index -lt $script:ExpectedCommandSequence.Count; $index++) {
            if (-not (Test-ArgumentSequenceEqual `
                -Actual @($Commands[$index].Arguments) `
                -Expected @($script:ExpectedCommandSequence[$index]))) {
                $sequenceValid = $false
                break
            }
        }
    }
    $commandsPassed = (
        @($Commands).Count -eq $script:ExpectedCommandSequence.Count -and
        @($Commands | Where-Object { [int]$_.ExitCode -ne 0 }).Count -eq 0
    )

    $versionLines = if (@($Commands).Count -ge 1) {
        @($Commands[0].OutputLines)
    } else {
        @()
    }
    $allNames = if (@($Commands).Count -ge 2) {
        @(ConvertTo-NormalizedLines -Lines @($Commands[1].OutputLines) | Sort-Object)
    } else {
        @()
    }
    $runningBefore = if (@($Commands).Count -ge 3) {
        @(ConvertTo-NormalizedLines -Lines @($Commands[2].OutputLines) | Sort-Object)
    } else {
        @()
    }
    $runningAfter = if (@($Commands).Count -ge 4) {
        @(ConvertTo-NormalizedLines -Lines @($Commands[3].OutputLines) | Sort-Object)
    } else {
        @()
    }
    $runningStable = Test-NameSetsEqual `
        -Left @($runningBefore) `
        -Right @($runningAfter)

    $registrationNames = @(
        $Registrations |
            ForEach-Object { [string]$_.DisplayName }
    )
    $registrationOutput = @(
        $Registrations |
            Sort-Object RegistrationId |
            ForEach-Object {
                $displayName = [string]$_.DisplayName
                $resolvable = (
                    -not [string]::IsNullOrWhiteSpace($displayName) -and
                    (Get-NameCount `
                        -Names @($registrationNames) `
                        -Name $displayName) -eq 1 -and
                    (Get-NameCount -Names @($allNames) -Name $displayName) -eq 1
                )
                [ordered]@{
                    registration_id = $_.RegistrationId
                    display_name = $displayName
                    default_uid = $_.DefaultUid
                    is_running = Test-NamePresent `
                        -Names @($runningBefore) `
                        -Name $displayName
                    command_target_resolvable = $resolvable
                }
            }
    )
    $registrationIdsValid = (
        @($Registrations | Where-Object { -not $_.RegistrationIdValid }).Count -eq 0
    )
    $duplicatesFailClosed = $true
    foreach ($registration in $registrationOutput) {
        if ((Get-NameCount `
            -Names @($registrationNames) `
            -Name ([string]$registration.display_name)) -gt 1 -and
            [bool]$registration.command_target_resolvable) {
            $duplicatesFailClosed = $false
        }
    }

    $checks = @(
        New-Check `
            -Name "read_only_command_sequence" `
            -Passed ($sequenceValid -and $guestCommandCount -eq 0) `
            -FailureCode "WSL_GUEST_COMMAND_FORBIDDEN"
        New-Check `
            -Name "wsl_commands_completed" `
            -Passed $commandsPassed `
            -FailureCode "WSL_READ_ONLY_COMMAND_FAILED"
        New-Check `
            -Name "version_summary" `
            -Passed (@($versionLines).Count -gt 0) `
            -FailureCode "WSL_VERSION_UNAVAILABLE"
        New-Check `
            -Name "running_set_stable" `
            -Passed $runningStable `
            -FailureCode "WSL_RUNNING_SET_CHANGED"
        New-Check `
            -Name "registration_ids" `
            -Passed $registrationIdsValid `
            -FailureCode "WSL_REGISTRATION_ID_INVALID"
        New-Check `
            -Name "duplicate_names_fail_closed" `
            -Passed $duplicatesFailClosed `
            -FailureCode "WSL_DUPLICATE_NAME_RESOLVED"
    )
    $failedCodes = @(
        $checks |
            Where-Object { $_.outcome -ceq "failed" } |
            ForEach-Object { [string]$_.code }
    )

    $outcome = "passed"
    $exitCode = $script:ExitCodes.Completed
    $blockingReasons = @()
    if ($guestCommandCount -gt 0 -or -not $sequenceValid) {
        $outcome = "failed"
        $exitCode = $script:ExitCodes.SecurityBoundaryFailed
        $blockingReasons = @("WSL_GUEST_COMMAND_FORBIDDEN")
    } elseif (-not $commandsPassed -or @($versionLines).Count -eq 0) {
        $outcome = "blocked"
        $exitCode = $script:ExitCodes.StrictPrerequisiteBlocked
        $blockingReasons = @("WSL_READ_ONLY_COMMAND_FAILED")
    } elseif (-not $runningStable) {
        $outcome = "failed"
        $exitCode = $script:ExitCodes.SecurityBoundaryFailed
        $blockingReasons = @("WSL_RUNNING_SET_CHANGED")
    } elseif (-not $registrationIdsValid -or -not $duplicatesFailClosed) {
        $outcome = "failed"
        $exitCode = $script:ExitCodes.SecurityBoundaryFailed
        $blockingReasons = @($failedCodes)
    }

    return [ordered]@{
        schema_version = 1
        probe = "wsl2-host-contract"
        outcome = $outcome
        exit_code = $exitCode
        strict_gate_eligible = ($exitCode -eq 0 -and -not $TestOnly)
        test_only = $TestOnly
        wsl_version_sha256 = if (@($versionLines).Count -gt 0) {
            Get-Sha256String -Value ($versionLines -join "`n")
        } else {
            $null
        }
        all_names = @($allNames)
        running_before = @($runningBefore)
        running_after = @($runningAfter)
        guest_command_count = $guestCommandCount
        registrations = @($registrationOutput)
        checks = @($checks)
        blocking_reasons = @($blockingReasons)
    }
}

$testOnly = -not [string]::IsNullOrWhiteSpace($FixturePath)
try {
    if ($testOnly) {
        $fixture = Read-FixtureCase
        $commands = @(Convert-FixtureCommands -Fixture $fixture)
        $registrations = @(Convert-FixtureRegistrations -Fixture $fixture)
    } else {
        $resolvedWsl = Get-Command $WslExecutable -ErrorAction SilentlyContinue
        if ($null -eq $resolvedWsl) {
            $commands = @()
            $registrations = @()
        } else {
            $commands = @(
                Invoke-LiveWslCommand `
                    -Executable $resolvedWsl.Source `
                    -Arguments @("--version")
                Invoke-LiveWslCommand `
                    -Executable $resolvedWsl.Source `
                    -Arguments @("--list", "--quiet")
                Invoke-LiveWslCommand `
                    -Executable $resolvedWsl.Source `
                    -Arguments @("--list", "--running", "--quiet")
                Invoke-LiveWslCommand `
                    -Executable $resolvedWsl.Source `
                    -Arguments @("--list", "--running", "--quiet")
            )
            $registrations = @(Read-LiveRegistrations)
        }
    }
    $result = Build-Result `
        -Commands @($commands) `
        -Registrations @($registrations) `
        -TestOnly $testOnly
} catch {
    $result = [ordered]@{
        schema_version = 1
        probe = "wsl2-host-contract"
        outcome = "blocked"
        exit_code = $script:ExitCodes.StrictPrerequisiteBlocked
        strict_gate_eligible = $false
        test_only = $testOnly
        wsl_version_sha256 = $null
        all_names = @()
        running_before = @()
        running_after = @()
        guest_command_count = 0
        registrations = @()
        checks = @(
            [ordered]@{
                name = "probe_execution"
                outcome = "blocked"
                code = "WSL_PROBE_UNAVAILABLE"
            }
        )
        blocking_reasons = @("WSL_PROBE_UNAVAILABLE")
    }
}

[Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 30))
exit [int]$result.exit_code
