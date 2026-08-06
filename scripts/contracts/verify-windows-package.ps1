param(
    [string]$FixturePath,
    [string]$PackagePath,
    [ValidateSet("x64", "arm64")]
    [string]$TargetArchitecture = "x64",
    [string]$InstallEvidencePath,
    [string]$LifecycleEvidencePath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:ExitCodes = @{
    Completed = 0
    StrictPrerequisiteBlocked = 3
    SecurityBoundaryFailed = 5
}

function Get-Sha256File {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function New-Check {
    param(
        [string]$Name,
        [bool]$Passed,
        [string]$FailureCode
    )

    return [ordered]@{
        name = $Name
        outcome = if ($Passed) { "passed" } else { "failed" }
        code = if ($Passed) { $null } else { $FailureCode }
    }
}

function Get-ExpectedMachine {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Architecture
    )

    if ($Architecture -ceq "x64") {
        return "AMD64"
    }
    return "ARM64"
}

function Get-PeMachine {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "package is not a PE file"
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "PE signature is missing"
        }
        $machine = $reader.ReadUInt16()
        switch ($machine) {
            0x8664 { return "AMD64" }
            0xAA64 { return "ARM64" }
            default { return ("0x{0:X4}" -f $machine) }
        }
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Read-JsonFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "evidence file does not exist"
    }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-ExactValue {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Actual,
        [Parameter(Mandatory = $true)]
        [object]$Expected,
        [Parameter(Mandatory = $true)]
        [string]$Code
    )

    if ([string]$Actual -cne [string]$Expected) {
        throw $Code
    }
}

function Test-CurrentUserPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        return $false
    }
    $fullPath = [IO.Path]::GetFullPath($Path).TrimEnd("\")
    $localAppData = [IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd("\")
    return $fullPath.StartsWith($localAppData + "\", [StringComparison]::OrdinalIgnoreCase)
}

function Test-Lifecycle {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Lifecycle,
        [bool]$FixtureMode
    )

    $checks = @(
        New-Check `
            -Name "runner_ephemeral" `
            -Passed ([bool]$Lifecycle.runner_lifecycle.ephemeral) `
            -FailureCode "WINDOWS_RUNNER_NOT_EPHEMERAL"
        New-Check `
            -Name "dedicated_job" `
            -Passed ([bool]$Lifecycle.runner_lifecycle.dedicated_job) `
            -FailureCode "WINDOWS_RUNNER_NOT_DEDICATED"
        New-Check `
            -Name "account_created_for_job" `
            -Passed ([bool]$Lifecycle.account_lifecycle.created_for_job) `
            -FailureCode "WINDOWS_ACCOUNT_NOT_JOB_SCOPED"
        New-Check `
            -Name "profile_created_for_job" `
            -Passed ([bool]$Lifecycle.account_lifecycle.profile_created_for_job) `
            -FailureCode "WINDOWS_PROFILE_NOT_JOB_SCOPED"
        New-Check `
            -Name "cleanup_attempted" `
            -Passed ([bool]$Lifecycle.account_lifecycle.cleanup_attempted) `
            -FailureCode "WINDOWS_LIFECYCLE_CLEANUP_MISSING"
        New-Check `
            -Name "cleanup_attested" `
            -Passed ([bool]$Lifecycle.account_lifecycle.cleanup_attested) `
            -FailureCode "WINDOWS_LIFECYCLE_NOT_ATTESTED"
        New-Check `
            -Name "cleanup_succeeded" `
            -Passed ([bool]$Lifecycle.account_lifecycle.cleanup_succeeded) `
            -FailureCode "WINDOWS_LIFECYCLE_CLEANUP_FAILED"
        New-Check `
            -Name "account_absent" `
            -Passed ([bool]$Lifecycle.account_lifecycle.account_absent_after_cleanup) `
            -FailureCode "WINDOWS_ACCOUNT_REMAINS"
        New-Check `
            -Name "profile_absent" `
            -Passed ([bool]$Lifecycle.account_lifecycle.profile_absent_after_cleanup) `
            -FailureCode "WINDOWS_PROFILE_REMAINS"
    )
    $failed = @($checks | Where-Object { $_.outcome -ceq "failed" })
    return [pscustomobject]@{
        Checks = @($checks)
        Passed = @($failed).Count -eq 0
        FailureCodes = @($failed | ForEach-Object { [string]$_.code })
        FixtureMode = $FixtureMode
    }
}

function Test-InstallEvidence {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Package,
        [bool]$FixtureMode
    )

    $checks = @(
        New-Check `
            -Name "current_user_install" `
            -Passed ([string]$Package.install.scope -ceq "currentUser") `
            -FailureCode "WINDOWS_PACKAGE_NOT_CURRENT_USER"
        New-Check `
            -Name "localappdata_install_root" `
            -Passed ([string]$Package.install.root_kind -ceq "LOCALAPPDATA") `
            -FailureCode "WINDOWS_PACKAGE_NOT_CURRENT_USER"
        New-Check `
            -Name "path_smoke" `
            -Passed (
                [string]$Package.path_smoke.outcome -ceq "passed" -and
                [string]$Package.path_smoke.root_kind -ceq "app_local_data_dir" -and
                [bool]$Package.path_smoke.reopened -and
                [bool]$Package.path_smoke.absolute_path_redacted
            ) `
            -FailureCode "WINDOWS_PATH_SMOKE_FAILED"
        New-Check `
            -Name "marker_correlation" `
            -Passed ([bool]$Package.marker_correlated) `
            -FailureCode "WINDOWS_MARKER_NOT_CORRELATED"
    )
    $failed = @($checks | Where-Object { $_.outcome -ceq "failed" })
    return [pscustomobject]@{
        Checks = @($checks)
        Passed = @($failed).Count -eq 0
        FailureCodes = @($failed | ForEach-Object { [string]$_.code })
    }
}

function Test-Package {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Document,
        [bool]$FixtureMode
    )

    $targetArchitecture = if ($FixtureMode) {
        [string]$Document.target_architecture
    } else {
        $TargetArchitecture
    }
    $expectedMachine = Get-ExpectedMachine -Architecture $targetArchitecture
    $package = $Document.package
    $checks = New-Object System.Collections.Generic.List[object]

    $signaturePassed = [string]$package.authenticode.status -ceq "Valid"
    $checks.Add((New-Check `
        -Name "authenticode" `
        -Passed $signaturePassed `
        -FailureCode "WINDOWS_PACKAGE_UNSIGNED"))

    $machine = if ($FixtureMode) {
        [string]$package.pe.machine
    } else {
        Get-PeMachine -Path $PackagePath
    }
    $checks.Add((New-Check `
        -Name "pe_architecture" `
        -Passed ($machine -ceq $expectedMachine) `
        -FailureCode "WINDOWS_PACKAGE_ARCH_MISMATCH"))

    $install = Test-InstallEvidence -Package $package -FixtureMode $FixtureMode
    foreach ($check in @($install.Checks)) {
        $checks.Add($check)
    }

    $lifecycle = Test-Lifecycle `
        -Lifecycle $Document.lifecycle `
        -FixtureMode $FixtureMode
    foreach ($check in @($lifecycle.Checks)) {
        $checks.Add($check)
    }

    $failed = @($checks | Where-Object { $_.outcome -ceq "failed" })
    $blockingReasons = @($failed | ForEach-Object { [string]$_.code } | Select-Object -Unique)
    $passed = @($failed).Count -eq 0
    return [ordered]@{
        schema_version = 1
        probe = "windows-package-contract"
        outcome = if ($passed) { "passed" } else { "failed" }
        exit_code = if ($passed) {
            $script:ExitCodes.Completed
        } else {
            $script:ExitCodes.SecurityBoundaryFailed
        }
        strict_gate_eligible = $passed -and -not $FixtureMode
        test_only = $FixtureMode
        target_architecture = $targetArchitecture
        pe_machine = $machine
        package_sha256 = if ($FixtureMode) {
            [string]$package.sha256
        } else {
            Get-Sha256File -Path $PackagePath
        }
        checks = [object[]]$checks.ToArray()
        blocking_reasons = @($blockingReasons)
    }
}

function Test-LivePreconditions {
    if ([string]::IsNullOrWhiteSpace($PackagePath) -or
        [string]::IsNullOrWhiteSpace($InstallEvidencePath) -or
        [string]::IsNullOrWhiteSpace($LifecycleEvidencePath)) {
        throw "live package verification requires PackagePath, InstallEvidencePath and LifecycleEvidencePath"
    }
    if (-not (Test-Path -LiteralPath $PackagePath -PathType Leaf)) {
        throw "package artifact does not exist"
    }
    if (-not (Test-CurrentUserPath -Path $InstallEvidencePath)) {
        throw "install evidence must be held under current-user LOCALAPPDATA"
    }
}

$fixtureMode = -not [string]::IsNullOrWhiteSpace($FixturePath)
try {
    if ($fixtureMode) {
        $document = Read-JsonFile -Path $FixturePath
        if (-not [bool]$document.fixture_mode) {
            throw "fixture_mode must be true for injected package evidence"
        }
    } else {
        Test-LivePreconditions
        $document = [pscustomobject]@{
            target_architecture = $TargetArchitecture
            package = (Read-JsonFile -Path $InstallEvidencePath)
            lifecycle = (Read-JsonFile -Path $LifecycleEvidencePath)
        }
        $signature = Get-AuthenticodeSignature -FilePath $PackagePath
        $document.package | Add-Member -NotePropertyName authenticode -NotePropertyValue @{
            status = [string]$signature.Status
        }
        $document.package | Add-Member -NotePropertyName pe -NotePropertyValue @{
            machine = Get-PeMachine -Path $PackagePath
        }
        $document.package | Add-Member -NotePropertyName sha256 -NotePropertyValue (
            Get-Sha256File -Path $PackagePath
        )
    }

    $result = Test-Package -Document $document -FixtureMode $fixtureMode
    [Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 30))
    exit [int]$result.exit_code
} catch {
    $blocked = [ordered]@{
        schema_version = 1
        probe = "windows-package-contract"
        outcome = if ($fixtureMode) { "failed" } else { "blocked" }
        exit_code = if ($fixtureMode) {
            $script:ExitCodes.SecurityBoundaryFailed
        } else {
            $script:ExitCodes.StrictPrerequisiteBlocked
        }
        strict_gate_eligible = $false
        test_only = $fixtureMode
        blocking_reasons = @(
            if ($_.Exception.Message -match "Authenticode|signature") {
                "WINDOWS_PACKAGE_UNSIGNED"
            } else {
                "WINDOWS_PACKAGE_VERIFICATION_UNAVAILABLE"
            }
        )
        error = [string]$_.Exception.Message
    }
    [Console]::Out.WriteLine(($blocked | ConvertTo-Json -Compress -Depth 30))
    exit [int]$blocked.exit_code
}
