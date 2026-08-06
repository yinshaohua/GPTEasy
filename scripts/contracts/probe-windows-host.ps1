[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DisposableUserProfile,

    [Parameter(Mandatory = $true)]
    [string]$WorkingDirectory,

    [string]$OfficialCliExecutable,

    [string]$FixturePath,

    [string]$FixtureCase,

    [string]$ExpectedVersion = "0.146.1"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:ExitCodes = @{
    Completed = 0
    AssertionFailed = 2
    StrictPrerequisiteBlocked = 3
    SecurityBoundaryFailed = 5
}
$script:PowerShellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
$script:CodexProbe = Join-Path $PSScriptRoot "probe-codex.ps1"
$script:AllowedPackageName = "OpenAI.Codex"
$script:AllowedPackageFamilyName = "OpenAI.Codex_2p2nqsd0c76g0"
$script:BundledCodexRelativePath = "app\resources\codex.exe"
$script:Canary = "GPTEASY-CONTRACT-CANARY-NONSECRET-01-11"

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

function Get-Sha256File {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    return (
        Get-FileHash -LiteralPath $Path -Algorithm SHA256
    ).Hash.ToLowerInvariant()
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

function Resolve-HostIdentity {
    param(
        [AllowNull()]
        [object]$Fixture
    )

    if ($null -ne $Fixture) {
        $identity = Get-PropertyValue -Object $Fixture -Name "host_identity"
        if ($null -eq $identity) {
            throw "fixture host identity is missing"
        }
        return [pscustomobject]@{
            PackageName = [string](
                Get-PropertyValue -Object $identity -Name "package_name"
            )
            PackageFamilyName = [string](
                Get-PropertyValue -Object $identity -Name "package_family_name"
            )
            PackageVersion = [string](
                Get-PropertyValue -Object $identity -Name "package_version"
            )
            InstallRootCategory = [string](
                Get-PropertyValue -Object $identity -Name "install_root_category"
            )
            ExecutablePresent = ConvertTo-Boolean (
                Get-PropertyValue -Object $identity -Name "executable_present"
            )
            Executable = $null
        }
    }

    $packages = @(
        Get-AppxPackage -Name $script:AllowedPackageName -ErrorAction Stop |
            Where-Object {
                [string]$_.PackageFamilyName -ceq $script:AllowedPackageFamilyName
            }
    )
    if ($packages.Count -ne 1) {
        return [pscustomobject]@{
            PackageName = $script:AllowedPackageName
            PackageFamilyName = $script:AllowedPackageFamilyName
            PackageVersion = $null
            InstallRootCategory = "windows_appx"
            ExecutablePresent = $false
            Executable = $null
        }
    }
    $package = $packages[0]
    $executable = Join-Path $package.InstallLocation $script:BundledCodexRelativePath
    return [pscustomobject]@{
        PackageName = [string]$package.Name
        PackageFamilyName = [string]$package.PackageFamilyName
        PackageVersion = [string]$package.Version
        InstallRootCategory = "windows_appx"
        ExecutablePresent = (Test-Path -LiteralPath $executable -PathType Leaf)
        Executable = $executable
    }
}

function New-HostIdentityOutput {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Identity
    )

    return [ordered]@{
        package_name = $Identity.PackageName
        package_family_name = $Identity.PackageFamilyName
        package_version = $Identity.PackageVersion
        install_root_category = $Identity.InstallRootCategory
        executable_present = [bool]$Identity.ExecutablePresent
    }
}

function Test-HostIdentityAllowlist {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Identity
    )

    return (
        [string]$Identity.PackageName -ceq $script:AllowedPackageName -and
        [string]$Identity.PackageFamilyName -ceq $script:AllowedPackageFamilyName -and
        [string]$Identity.InstallRootCategory -ceq "windows_appx"
    )
}

function Get-CanaryContent {
    return @"
model = "gpteasy-contract-model-01-11"
model_provider = "gpteasy_contract"

[model_providers.gpteasy_contract]
name = "GPTEasy Contract Canary"
base_url = "https://127.0.0.1.invalid/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
experimental_bearer_token = "$($script:Canary)"
"@
}

function Invoke-CodexProbe {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("official_cli", "bundled_host")]
        [string]$Role,

        [AllowNull()]
        [string]$Executable
    )

    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", $script:CodexProbe,
        "-Role", $Role,
        "-DisposableUserProfile", $DisposableUserProfile,
        "-WorkingDirectory", $WorkingDirectory,
        "-ExpectedVersion", $ExpectedVersion
    )
    if (-not [string]::IsNullOrWhiteSpace($FixturePath)) {
        $arguments += @(
            "-FixturePath", $FixturePath,
            "-FixtureCase", $FixtureCase
        )
    } else {
        $arguments += @("-CodexExecutable", $Executable)
    }

    $lines = @(& $script:PowerShellExecutable @arguments 2>&1)
    $exitCode = [int]$LASTEXITCODE
    $output = ($lines -join "`n").Trim()
    try {
        $document = $output | ConvertFrom-Json
    } catch {
        return [pscustomobject]@{
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            Document = $null
        }
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Document = $document
    }
}

function New-ProbeOutput {
    param(
        [AllowNull()]
        [object]$Probe
    )

    if ($null -eq $Probe) {
        return $null
    }
    return [ordered]@{
        outcome = Get-PropertyValue -Object $Probe -Name "outcome"
        exit_code = Get-PropertyValue -Object $Probe -Name "exit_code"
        version = Get-PropertyValue -Object $Probe -Name "version"
        binary_sha256 = Get-PropertyValue -Object $Probe -Name "binary_sha256"
        schema_sha256 = Get-PropertyValue -Object $Probe -Name "schema_sha256"
        config_root_category = Get-PropertyValue `
            -Object $Probe `
            -Name "config_root_category"
        protocol = Get-PropertyValue -Object $Probe -Name "protocol"
        model_sha256 = Get-PropertyValue -Object $Probe -Name "model_sha256"
        provider_sha256 = Get-PropertyValue -Object $Probe -Name "provider_sha256"
        origin_sha256 = Get-PropertyValue -Object $Probe -Name "origin_sha256"
        credential_carrier = Get-PropertyValue `
            -Object $Probe `
            -Name "credential_carrier"
        shared_user_layer = ConvertTo-Boolean (
            Get-PropertyValue -Object $Probe -Name "shared_user_layer"
        )
    }
}

function Test-ObjectJsonEqual {
    param(
        [AllowNull()]
        [object]$Left,

        [AllowNull()]
        [object]$Right
    )

    if ($null -eq $Left -or $null -eq $Right) {
        return $false
    }
    return (
        ($Left | ConvertTo-Json -Compress -Depth 20) -ceq
        ($Right | ConvertTo-Json -Compress -Depth 20)
    )
}

function New-EmptyParity {
    return [ordered]@{
        version = $false
        config_root = $false
        model_digest = $false
        provider_digest = $false
        origin_digest = $false
        credential_carrier = $false
        shared_user_layer = $false
        all = $false
    }
}

function New-Result {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Identity,

        [AllowNull()]
        [object]$CliProbe,

        [AllowNull()]
        [object]$HostProbe,

        [Parameter(Mandatory = $true)]
        [object]$Parity,

        [Parameter(Mandatory = $true)]
        [string]$Outcome,

        [Parameter(Mandatory = $true)]
        [int]$ExitCode,

        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$Codes,

        [Parameter(Mandatory = $true)]
        [bool]$TestOnly
    )

    $checks = @(
        [ordered]@{
            name = "host_package_allowlisted"
            outcome = if (Test-HostIdentityAllowlist -Identity $Identity) {
                "passed"
            } else {
                "failed"
            }
            code = if (Test-HostIdentityAllowlist -Identity $Identity) {
                "OK"
            } else {
                "HOST_PACKAGE_IDENTITY_MISMATCH"
            }
        },
        [ordered]@{
            name = "host_executable_present"
            outcome = if ($Identity.ExecutablePresent) { "passed" } else { "blocked" }
            code = if ($Identity.ExecutablePresent) { "OK" } else { "HOST_CODEX_MISSING" }
        },
        [ordered]@{
            name = "host_cli_parity"
            outcome = if ($Parity.all) { "passed" } else { "failed" }
            code = if ($Parity.all) { "OK" } else { "HOST_CLI_PARITY_MISMATCH" }
        }
    )

    return [ordered]@{
        schema_version = 1
        probe = "windows-host-codex-parity"
        outcome = $Outcome
        exit_code = $ExitCode
        strict_gate_eligible = ($ExitCode -eq 0 -and -not $TestOnly)
        test_only = $TestOnly
        expected_version = $ExpectedVersion
        host_identity = New-HostIdentityOutput -Identity $Identity
        official_cli = New-ProbeOutput -Probe $CliProbe
        bundled_host = New-ProbeOutput -Probe $HostProbe
        parity = $Parity
        checks = @($checks)
        blocking_reasons = @($Codes)
    }
}

$testOnly = -not [string]::IsNullOrWhiteSpace($FixturePath)
$fixture = $null
$identity = [pscustomobject]@{
    PackageName = $script:AllowedPackageName
    PackageFamilyName = $script:AllowedPackageFamilyName
    PackageVersion = $null
    InstallRootCategory = "windows_appx"
    ExecutablePresent = $false
    Executable = $null
}
$cliDocument = $null
$hostDocument = $null
$parity = New-EmptyParity
$result = $null
$canaryPath = $null
$canaryDigest = $null

try {
    if (-not (Test-Path -LiteralPath $script:CodexProbe -PathType Leaf)) {
        throw "Codex probe does not exist"
    }
    $fixture = if ($testOnly) { Read-FixtureCase } else { $null }
    $identity = Resolve-HostIdentity -Fixture $fixture

    if (-not (Test-HostIdentityAllowlist -Identity $identity)) {
        $result = New-Result `
            -Identity $identity `
            -CliProbe $null `
            -HostProbe $null `
            -Parity $parity `
            -Outcome "failed" `
            -ExitCode $script:ExitCodes.SecurityBoundaryFailed `
            -Codes @("HOST_PACKAGE_IDENTITY_MISMATCH") `
            -TestOnly $testOnly
    } elseif (-not $identity.ExecutablePresent) {
        $result = New-Result `
            -Identity $identity `
            -CliProbe $null `
            -HostProbe $null `
            -Parity $parity `
            -Outcome "blocked" `
            -ExitCode $script:ExitCodes.StrictPrerequisiteBlocked `
            -Codes @("HOST_CODEX_MISSING") `
            -TestOnly $testOnly
    } else {
        $profileRoot = [System.IO.Path]::GetFullPath(
            $DisposableUserProfile
        ).TrimEnd("\", "/")
        $cwdRoot = [System.IO.Path]::GetFullPath($WorkingDirectory).TrimEnd("\", "/")
        if (-not $cwdRoot.StartsWith(
            $profileRoot + "\",
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            throw "working directory is outside the disposable profile"
        }
        if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
            throw "working directory does not exist"
        }
        if (-not $testOnly) {
            $currentProfile = [System.IO.Path]::GetFullPath(
                [Environment]::GetFolderPath("UserProfile")
            ).TrimEnd("\", "/")
            if (-not $profileRoot.Equals(
                $currentProfile,
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
                throw "live probe requires the current disposable OS user profile"
            }
            if ([string]::IsNullOrWhiteSpace($OfficialCliExecutable) -or
                -not (Test-Path -LiteralPath $OfficialCliExecutable -PathType Leaf)) {
                throw "official CLI executable is missing"
            }
        }

        $codexRoot = Join-Path $profileRoot ".codex"
        $canaryPath = Join-Path $codexRoot "config.toml"
        $canaryPathFull = [System.IO.Path]::GetFullPath($canaryPath)
        if (-not $canaryPathFull.StartsWith(
            $profileRoot + "\",
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            throw "canary path is outside the disposable profile"
        }
        if (Test-Path -LiteralPath $canaryPath) {
            throw "disposable profile already contains Codex config"
        }
        New-Item -ItemType Directory -Path $codexRoot -Force | Out-Null
        Write-Utf8NoBom -Path $canaryPath -Content ((Get-CanaryContent) + "`n")
        $canaryDigest = Get-Sha256File -Path $canaryPath

        $cliResult = Invoke-CodexProbe `
            -Role "official_cli" `
            -Executable $OfficialCliExecutable
        $hostResult = Invoke-CodexProbe `
            -Role "bundled_host" `
            -Executable $identity.Executable
        $cliDocument = $cliResult.Document
        $hostDocument = $hostResult.Document

        if ($null -ne $cliDocument -and $null -ne $hostDocument) {
            $parity.version = (
                [string](Get-PropertyValue -Object $cliDocument -Name "version") -ceq
                [string](Get-PropertyValue -Object $hostDocument -Name "version") -and
                [string](Get-PropertyValue -Object $cliDocument -Name "version") -ceq
                $ExpectedVersion
            )
            $parity.config_root = (
                [string](
                    Get-PropertyValue -Object $cliDocument -Name "config_root_category"
                ) -ceq
                [string](
                    Get-PropertyValue -Object $hostDocument -Name "config_root_category"
                ) -and
                [string](
                    Get-PropertyValue -Object $cliDocument -Name "config_root_category"
                ) -ceq "default_user"
            )
            $parity.model_digest = (
                [string](
                    Get-PropertyValue -Object $cliDocument -Name "model_sha256"
                ) -ceq
                [string](
                    Get-PropertyValue -Object $hostDocument -Name "model_sha256"
                )
            )
            $parity.provider_digest = (
                [string](
                    Get-PropertyValue -Object $cliDocument -Name "provider_sha256"
                ) -ceq
                [string](
                    Get-PropertyValue -Object $hostDocument -Name "provider_sha256"
                )
            )
            $parity.origin_digest = (
                [string](
                    Get-PropertyValue -Object $cliDocument -Name "origin_sha256"
                ) -ceq
                [string](
                    Get-PropertyValue -Object $hostDocument -Name "origin_sha256"
                )
            )
            $parity.credential_carrier = Test-ObjectJsonEqual `
                -Left (
                    Get-PropertyValue `
                        -Object $cliDocument `
                        -Name "credential_carrier"
                ) `
                -Right (
                    Get-PropertyValue `
                        -Object $hostDocument `
                        -Name "credential_carrier"
                )
            $parity.shared_user_layer = (
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $cliDocument -Name "shared_user_layer"
                )) -and
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $hostDocument -Name "shared_user_layer"
                ))
            )
            $parity.all = (
                $parity.version -and
                $parity.config_root -and
                $parity.model_digest -and
                $parity.provider_digest -and
                $parity.origin_digest -and
                $parity.credential_carrier -and
                $parity.shared_user_layer -and
                [int](Get-PropertyValue -Object $cliDocument -Name "exit_code") -eq 0 -and
                [int](Get-PropertyValue -Object $hostDocument -Name "exit_code") -eq 0
            )
        }

        if ($parity.all) {
            $result = New-Result `
                -Identity $identity `
                -CliProbe $cliDocument `
                -HostProbe $hostDocument `
                -Parity $parity `
                -Outcome "passed" `
                -ExitCode $script:ExitCodes.Completed `
                -Codes @() `
                -TestOnly $testOnly
        } else {
            $exitCode = if (-not $parity.version) {
                $script:ExitCodes.AssertionFailed
            } else {
                $script:ExitCodes.SecurityBoundaryFailed
            }
            $result = New-Result `
                -Identity $identity `
                -CliProbe $cliDocument `
                -HostProbe $hostDocument `
                -Parity $parity `
                -Outcome "failed" `
                -ExitCode $exitCode `
                -Codes @("HOST_CLI_PARITY_MISMATCH") `
                -TestOnly $testOnly
        }
    }
} catch {
    $result = New-Result `
        -Identity $identity `
        -CliProbe $cliDocument `
        -HostProbe $hostDocument `
        -Parity $parity `
        -Outcome "blocked" `
        -ExitCode $script:ExitCodes.StrictPrerequisiteBlocked `
        -Codes @("WINDOWS_HOST_PROBE_UNAVAILABLE") `
        -TestOnly $testOnly
} finally {
    if ($null -ne $canaryPath -and
        $null -ne $canaryDigest -and
        (Test-Path -LiteralPath $canaryPath -PathType Leaf)) {
        try {
            if ((Get-Sha256File -Path $canaryPath) -ceq $canaryDigest) {
                Remove-Item -LiteralPath $canaryPath -Force
            }
        } catch {
        }
    }
}

[Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 30))
exit [int]$result.exit_code
