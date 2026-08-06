[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:ProbeCodex = Join-Path $PSScriptRoot "probe-codex.ps1"
$script:ProbeHost = Join-Path $PSScriptRoot "probe-windows-host.ps1"
$script:PowerShellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
$script:TempRoot = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    ("gpteasy-windows-contract-test-" + [guid]::NewGuid().ToString("N"))
$script:Canary = "GPTEASY-CONTRACT-CANARY-NONSECRET-01-11"

function Throw-TestFailure {
    throw [System.InvalidOperationException]::new(
        "Windows contract probe self-test failed"
    )
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

function New-ProbeFixture {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet("official_cli", "bundled_host")]
        [string]$Role,

        [Parameter(Mandatory = $true)]
        [string]$BinaryDigest
    )

    return [ordered]@{
        role = $Role
        executable_present = $true
        version = "0.146.1"
        binary_sha256 = $BinaryDigest
        schema_sha256 = ("b" * 64)
        config_root_category = "default_user"
        protocol = [ordered]@{
            initialize = $true
            initialized = $true
            config_read = $true
            include_layers = $true
        }
        model_sha256 = ("c" * 64)
        provider_sha256 = ("d" * 64)
        origin_sha256 = ("e" * 64)
        credential_carrier = [ordered]@{
            env_key = $false
            direct_bearer = $true
            missing = $false
        }
        canary_model_match = $true
        canary_provider_match = $true
        shared_user_layer = $true
    }
}

function New-PositiveCase {
    return [ordered]@{
        host_identity = [ordered]@{
            package_name = "OpenAI.Codex"
            package_family_name = "OpenAI.Codex_2p2nqsd0c76g0"
            package_version = "26.730.8199.0"
            install_root_category = "windows_appx"
            executable_present = $true
        }
        official_cli = New-ProbeFixture `
            -Role "official_cli" `
            -BinaryDigest ("1" * 64)
        bundled_host = New-ProbeFixture `
            -Role "bundled_host" `
            -BinaryDigest ("2" * 64)
    }
}

function New-FixtureDocument {
    $positive = New-PositiveCase
    $hostMissing = Copy-JsonObject -Value $positive
    $hostMissing.host_identity.executable_present = $false

    $versionMismatch = Copy-JsonObject -Value $positive
    $versionMismatch.bundled_host.version = "0.146.0"

    $rootMismatch = Copy-JsonObject -Value $positive
    $rootMismatch.bundled_host.config_root_category = "custom"

    $providerMismatch = Copy-JsonObject -Value $positive
    $providerMismatch.bundled_host.provider_sha256 = ("f" * 64)

    $originMismatch = Copy-JsonObject -Value $positive
    $originMismatch.bundled_host.origin_sha256 = ("0" * 64)

    $carrierMismatch = Copy-JsonObject -Value $positive
    $carrierMismatch.bundled_host.credential_carrier.env_key = $true
    $carrierMismatch.bundled_host.credential_carrier.direct_bearer = $false

    return [ordered]@{
        schema_version = 1
        cases = [ordered]@{
            positive = $positive
            host_missing = $hostMissing
            version_mismatch = $versionMismatch
            root_mismatch = $rootMismatch
            provider_mismatch = $providerMismatch
            origin_mismatch = $originMismatch
            carrier_mismatch = $carrierMismatch
        }
    }
}

function Invoke-JsonScript {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Script,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $lines = @(
        & $script:PowerShellExecutable `
            -NoProfile `
            -ExecutionPolicy Bypass `
            -File $Script `
            @Arguments 2>&1
    )
    $exitCode = [int]$LASTEXITCODE
    $output = ($lines -join "`n").Trim()
    Assert-Condition -Condition (-not [string]::IsNullOrWhiteSpace($output))
    Assert-Condition -Condition ($output -cnotmatch [regex]::Escape($script:Canary))
    Assert-Condition -Condition ($output -notmatch "(?i)authorization\s*:")
    Assert-Condition -Condition ($output -notmatch "(?i)bearer\s+")
    Assert-Condition -Condition ($output -notmatch "(?i)command[_ ]?line")
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

function Invoke-HostCase {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FixturePath,

        [Parameter(Mandatory = $true)]
        [string]$Case
    )

    $profile = Join-Path $script:TempRoot ("profile-" + $Case)
    $cwd = Join-Path $profile "cwd"
    New-Item -ItemType Directory -Path $cwd -Force | Out-Null
    $result = Invoke-JsonScript `
        -Script $script:ProbeHost `
        -Arguments @(
            "-DisposableUserProfile", $profile,
            "-WorkingDirectory", $cwd,
            "-FixturePath", $FixturePath,
            "-FixtureCase", $Case
        )
    Assert-Condition -Condition (
        -not (Test-Path -LiteralPath (Join-Path $profile ".codex\config.toml"))
    )
    return $result
}

try {
    Assert-Condition -Condition (Test-Path -LiteralPath $script:ProbeCodex -PathType Leaf)
    Assert-Condition -Condition (Test-Path -LiteralPath $script:ProbeHost -PathType Leaf)
    New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null

    $fixturePath = Join-Path $script:TempRoot "windows-contract-fixture.json"
    Write-JsonFile -Path $fixturePath -Value (New-FixtureDocument)

    $probeProfile = Join-Path $script:TempRoot "probe-profile"
    $probeCwd = Join-Path $probeProfile "cwd"
    New-Item -ItemType Directory -Path $probeCwd -Force | Out-Null
    $probeResult = Invoke-JsonScript `
        -Script $script:ProbeCodex `
        -Arguments @(
            "-Role", "official_cli",
            "-DisposableUserProfile", $probeProfile,
            "-WorkingDirectory", $probeCwd,
            "-FixturePath", $fixturePath,
            "-FixtureCase", "positive"
        )
    Assert-Condition -Condition ($probeResult.ExitCode -eq 0)
    Assert-ExactProperties -Object $probeResult.Document -Expected @(
        "schema_version",
        "probe",
        "role",
        "outcome",
        "exit_code",
        "strict_gate_eligible",
        "test_only",
        "expected_version",
        "version",
        "binary_sha256",
        "schema_sha256",
        "config_root_category",
        "protocol",
        "model_sha256",
        "provider_sha256",
        "origin_sha256",
        "credential_carrier",
        "shared_user_layer",
        "checks",
        "blocking_reasons"
    )
    Assert-ExactProperties -Object $probeResult.Document.protocol -Expected @(
        "initialize",
        "initialized",
        "config_read",
        "include_layers"
    )
    Assert-ExactProperties -Object $probeResult.Document.credential_carrier -Expected @(
        "env_key",
        "direct_bearer",
        "missing"
    )
    Assert-Condition -Condition ([bool]$probeResult.Document.test_only)
    Assert-Condition -Condition (-not [bool]$probeResult.Document.strict_gate_eligible)
    Assert-Condition -Condition ([bool]$probeResult.Document.shared_user_layer)

    $positive = Invoke-HostCase -FixturePath $fixturePath -Case "positive"
    Assert-Condition -Condition ($positive.ExitCode -eq 0)
    Assert-Condition -Condition ([string]$positive.Document.outcome -ceq "passed")
    Assert-Condition -Condition ([bool]$positive.Document.test_only)
    Assert-Condition -Condition (-not [bool]$positive.Document.strict_gate_eligible)
    Assert-Condition -Condition ([bool]$positive.Document.parity.all)
    Assert-Condition -Condition ([bool]$positive.Document.parity.shared_user_layer)
    Assert-ExactProperties -Object $positive.Document -Expected @(
        "schema_version",
        "probe",
        "outcome",
        "exit_code",
        "strict_gate_eligible",
        "test_only",
        "expected_version",
        "host_identity",
        "official_cli",
        "bundled_host",
        "parity",
        "checks",
        "blocking_reasons"
    )
    Assert-ExactProperties -Object $positive.Document.parity -Expected @(
        "version",
        "config_root",
        "model_digest",
        "provider_digest",
        "origin_digest",
        "credential_carrier",
        "shared_user_layer",
        "all"
    )

    $hostMissing = Invoke-HostCase -FixturePath $fixturePath -Case "host_missing"
    Assert-Condition -Condition ($hostMissing.ExitCode -eq 3)
    Assert-Condition -Condition ([string]$hostMissing.Document.outcome -ceq "blocked")

    foreach ($case in @(
        "version_mismatch",
        "root_mismatch",
        "provider_mismatch",
        "origin_mismatch",
        "carrier_mismatch"
    )) {
        $result = Invoke-HostCase -FixturePath $fixturePath -Case $case
        Assert-Condition -Condition ($result.ExitCode -ne 0)
        Assert-Condition -Condition ([string]$result.Document.outcome -ceq "failed")
        Assert-Condition -Condition (-not [bool]$result.Document.parity.all)
    }

    $codexSource = [System.IO.File]::ReadAllText($script:ProbeCodex)
    $hostSource = [System.IO.File]::ReadAllText($script:ProbeHost)
    Assert-Condition -Condition ($codexSource -match "0\.146\.1")
    Assert-Condition -Condition ($codexSource -match "generate-json-schema")
    Assert-Condition -Condition ($codexSource -match 'method\s*=\s*"initialize"')
    Assert-Condition -Condition ($codexSource -match 'method\s*=\s*"initialized"')
    Assert-Condition -Condition ($codexSource -match 'method\s*=\s*"config/read"')
    Assert-Condition -Condition ($codexSource -match "includeLayers")
    Assert-Condition -Condition ($hostSource -match "OpenAI\.Codex_2p2nqsd0c76g0")
    Assert-Condition -Condition ($hostSource -match [regex]::Escape($script:Canary))
    Assert-Condition -Condition ($hostSource -match "shared_user_layer")
    Assert-Condition -Condition ($hostSource -notmatch "(?i)Get-CimInstance.+CommandLine")

    Write-Output (
        "Windows contract probe self-test passed: exact Codex protocol, host identity, " +
        "shared user-layer parity, and 6 fail-closed cases"
    )
    exit 0
} catch {
    Write-Output "Windows contract probe self-test failed; raw configuration and process output are not emitted."
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
