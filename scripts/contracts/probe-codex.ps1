[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("official_cli", "bundled_host")]
    [string]$Role,

    [Parameter(Mandatory = $true)]
    [string]$DisposableUserProfile,

    [Parameter(Mandatory = $true)]
    [string]$WorkingDirectory,

    [string]$CodexExecutable,

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
$script:ExpectedModel = "gpteasy-contract-model-01-11"
$script:ExpectedProvider = "gpteasy_contract"
$script:ExpectedCarrier = "direct_bearer"

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

function Test-Sha256 {
    param(
        [AllowNull()]
        [object]$Value
    )

    return ([string]$Value -cmatch "^[0-9a-f]{64}$")
}

function ConvertTo-CanonicalJson {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return ($Value | ConvertTo-Json -Compress -Depth 30)
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

function New-EmptyObservation {
    return [ordered]@{
        role = $Role
        executable_present = $false
        version = $null
        binary_sha256 = $null
        schema_sha256 = $null
        config_root_category = "unknown"
        protocol = [ordered]@{
            initialize = $false
            initialized = $false
            config_read = $false
            include_layers = $false
        }
        model_sha256 = $null
        provider_sha256 = $null
        origin_sha256 = $null
        credential_carrier = [ordered]@{
            env_key = $false
            direct_bearer = $false
            missing = $true
        }
        canary_model_match = $false
        canary_provider_match = $false
        shared_user_layer = $false
    }
}

function Read-FixtureObservation {
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
    $cases = Get-PropertyValue -Object $fixture -Name "cases"
    $case = Get-PropertyValue -Object $cases -Name $FixtureCase
    if ($null -eq $case) {
        throw "fixture case does not exist"
    }
    $observation = Get-PropertyValue -Object $case -Name $Role
    if ($null -eq $observation) {
        throw "fixture role does not exist"
    }
    if ([string](Get-PropertyValue -Object $observation -Name "role") -cne $Role) {
        throw "fixture role does not match"
    }

    return $observation
}

function Quote-ProcessArgument {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value
    )

    return '"' + $Value.Replace('"', '\"') + '"'
}

function Invoke-ProcessCapture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [Parameter(Mandatory = $true)]
        [string]$Arguments,

        [Parameter(Mandatory = $true)]
        [string]$CurrentDirectory
    )

    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = $Arguments
    $info.WorkingDirectory = $CurrentDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) {
            throw "process did not start"
        }
        $stdout = $process.StandardOutput.ReadToEnd()
        $null = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        return [pscustomobject]@{
            ExitCode = [int]$process.ExitCode
            Stdout = [string]$stdout
        }
    } finally {
        $process.Dispose()
    }
}

function Get-SchemaDigest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SchemaRoot
    )

    $files = @(
        Get-ChildItem -LiteralPath $SchemaRoot -Recurse -File -Filter "*.json" |
            Sort-Object FullName
    )
    if ($files.Count -eq 0) {
        throw "schema generator produced no JSON files"
    }

    $entries = foreach ($file in $files) {
        $relative = $file.FullName.Substring($SchemaRoot.Length).TrimStart("\", "/")
        $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "{0}`n{1}" -f ($relative -replace "\\", "/"), $digest
    }
    return Get-Sha256String -Value ($entries -join "`n")
}

function Send-AppServerMessage {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.StreamWriter]$Writer,

        [Parameter(Mandatory = $true)]
        [object]$Message
    )

    $Writer.WriteLine((ConvertTo-CanonicalJson -Value $Message))
    $Writer.Flush()
}

function Read-AppServerResponse {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.StreamReader]$Reader,

        [Parameter(Mandatory = $true)]
        [System.Diagnostics.Process]$Process,

        [Parameter(Mandatory = $true)]
        [int]$Id,

        [int]$TimeoutSeconds = 20
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $remaining = [int][Math]::Max(
            1,
            ($deadline - [DateTime]::UtcNow).TotalMilliseconds
        )
        $task = $Reader.ReadLineAsync()
        if (-not $task.Wait($remaining)) {
            throw "app-server response timed out"
        }
        $line = $task.Result
        if ($null -eq $line) {
            throw "app-server closed stdout"
        }
        try {
            $message = $line | ConvertFrom-Json
        } catch {
            continue
        }
        $messageId = Get-PropertyValue -Object $message -Name "id"
        if ($null -ne $messageId -and [int]$messageId -eq $Id) {
            return $message
        }
        if ($Process.HasExited) {
            throw "app-server exited before response"
        }
    }
    throw "app-server response timed out"
}

function Get-OriginType {
    param(
        [AllowNull()]
        [object]$Origin
    )

    $name = Get-PropertyValue -Object $Origin -Name "name"
    return [string](Get-PropertyValue -Object $name -Name "type")
}

function Get-LiveObservation {
    $observation = New-EmptyObservation
    if ([string]::IsNullOrWhiteSpace($CodexExecutable) -or
        -not (Test-Path -LiteralPath $CodexExecutable -PathType Leaf)) {
        return $observation
    }

    $profileRoot = [System.IO.Path]::GetFullPath($DisposableUserProfile).TrimEnd("\", "/")
    $currentProfile = [System.IO.Path]::GetFullPath(
        [Environment]::GetFolderPath("UserProfile")
    ).TrimEnd("\", "/")
    if (-not $profileRoot.Equals(
        $currentProfile,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "live probe requires the current disposable OS user profile"
    }
    if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
        throw "working directory does not exist"
    }
    $configPath = Join-Path $profileRoot ".codex\config.toml"
    if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
        throw "canary user config does not exist"
    }

    $observation.executable_present = $true
    $versionResult = Invoke-ProcessCapture `
        -Executable $CodexExecutable `
        -Arguments "--version" `
        -CurrentDirectory $WorkingDirectory
    if ($versionResult.ExitCode -ne 0) {
        throw "Codex version probe failed"
    }
    $versionMatch = [regex]::Match(
        $versionResult.Stdout,
        "(?m)(?:codex-cli\s+)?(?<version>[0-9]+\.[0-9]+\.[0-9]+)"
    )
    if (-not $versionMatch.Success) {
        throw "Codex version was not recognized"
    }
    $observation.version = $versionMatch.Groups["version"].Value
    $observation.binary_sha256 = (
        Get-FileHash -LiteralPath $CodexExecutable -Algorithm SHA256
    ).Hash.ToLowerInvariant()

    $schemaRoot = Join-Path `
        ([System.IO.Path]::GetTempPath()) `
        ("gpteasy-codex-schema-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $schemaRoot -Force | Out-Null
    try {
        $schemaArguments = "app-server generate-json-schema --out " +
            (Quote-ProcessArgument -Value $schemaRoot)
        $schemaResult = Invoke-ProcessCapture `
            -Executable $CodexExecutable `
            -Arguments $schemaArguments `
            -CurrentDirectory $WorkingDirectory
        if ($schemaResult.ExitCode -ne 0) {
            throw "Codex schema generation failed"
        }
        $observation.schema_sha256 = Get-SchemaDigest -SchemaRoot $schemaRoot
    } finally {
        if (Test-Path -LiteralPath $schemaRoot -PathType Container) {
            $resolvedSchemaRoot = (Resolve-Path -LiteralPath $schemaRoot).Path
            $tempRoot = [System.IO.Path]::GetFullPath(
                [System.IO.Path]::GetTempPath()
            ).TrimEnd("\")
            if ($resolvedSchemaRoot.StartsWith(
                $tempRoot + "\",
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
                Remove-Item -LiteralPath $resolvedSchemaRoot -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $CodexExecutable
    $info.Arguments = if ($Role -ceq "bundled_host") {
        '-c "features.code_mode_host=true" app-server --analytics-default-enabled'
    } else {
        "app-server"
    }
    $info.WorkingDirectory = $WorkingDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $null = $info.EnvironmentVariables.Remove("CODEX_HOME")
    $info.EnvironmentVariables["USERPROFILE"] = $profileRoot
    $info.EnvironmentVariables["HOME"] = $profileRoot

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) {
            throw "app-server did not start"
        }
        $process.BeginErrorReadLine()

        $initialize = [ordered]@{
            id = 1
            method = "initialize"
            params = [ordered]@{
                clientInfo = [ordered]@{
                    name = "gpteasy-contract-probe"
                    version = "1.0.0"
                }
                capabilities = [ordered]@{
                    experimentalApi = $true
                }
            }
        }
        Send-AppServerMessage -Writer $process.StandardInput -Message $initialize
        $initializeResponse = Read-AppServerResponse `
            -Reader $process.StandardOutput `
            -Process $process `
            -Id 1
        $initializeResult = Get-PropertyValue -Object $initializeResponse -Name "result"
        if ($null -eq $initializeResult) {
            throw "initialize response has no result"
        }
        $observation.protocol.initialize = $true

        $initialized = [ordered]@{
            method = "initialized"
            params = [ordered]@{}
        }
        Send-AppServerMessage -Writer $process.StandardInput -Message $initialized
        $observation.protocol.initialized = $true

        $configRead = [ordered]@{
            id = 2
            method = "config/read"
            params = [ordered]@{
                cwd = $WorkingDirectory
                includeLayers = $true
            }
        }
        Send-AppServerMessage -Writer $process.StandardInput -Message $configRead
        $configResponse = Read-AppServerResponse `
            -Reader $process.StandardOutput `
            -Process $process `
            -Id 2
        $configResult = Get-PropertyValue -Object $configResponse -Name "result"
        if ($null -eq $configResult) {
            throw "config/read response has no result"
        }
        $observation.protocol.config_read = $true
        $observation.protocol.include_layers = $true

        $reportedCodexHome = [string](
            Get-PropertyValue -Object $initializeResult -Name "codexHome"
        )
        $expectedCodexHome = [System.IO.Path]::GetFullPath(
            (Join-Path $profileRoot ".codex")
        ).TrimEnd("\", "/")
        $reportedCodexHomeFull = if ([string]::IsNullOrWhiteSpace($reportedCodexHome)) {
            ""
        } else {
            [System.IO.Path]::GetFullPath($reportedCodexHome).TrimEnd("\", "/")
        }
        $observation.config_root_category = if (
            $reportedCodexHomeFull.Equals(
                $expectedCodexHome,
                [System.StringComparison]::OrdinalIgnoreCase
            )
        ) {
            "default_user"
        } else {
            "custom"
        }

        $config = Get-PropertyValue -Object $configResult -Name "config"
        $model = [string](Get-PropertyValue -Object $config -Name "model")
        $providerKey = [string](
            Get-PropertyValue -Object $config -Name "model_provider"
        )
        $providers = Get-PropertyValue -Object $config -Name "model_providers"
        $provider = Get-PropertyValue -Object $providers -Name $providerKey
        $envKeyPresent = -not [string]::IsNullOrWhiteSpace(
            [string](Get-PropertyValue -Object $provider -Name "env_key")
        )
        $directBearerPresent = -not [string]::IsNullOrWhiteSpace(
            [string](
                Get-PropertyValue -Object $provider -Name "experimental_bearer_token"
            )
        )
        $observation.credential_carrier = [ordered]@{
            env_key = $envKeyPresent
            direct_bearer = $directBearerPresent
            missing = (-not $envKeyPresent -and -not $directBearerPresent)
        }
        $observation.model_sha256 = Get-Sha256String -Value $model
        $providerSummary = [ordered]@{
            provider_key = $providerKey
            name = [string](Get-PropertyValue -Object $provider -Name "name")
            base_url = [string](Get-PropertyValue -Object $provider -Name "base_url")
            wire_api = [string](Get-PropertyValue -Object $provider -Name "wire_api")
            requires_openai_auth = ConvertTo-Boolean (
                Get-PropertyValue -Object $provider -Name "requires_openai_auth"
            )
            supports_websockets = ConvertTo-Boolean (
                Get-PropertyValue -Object $provider -Name "supports_websockets"
            )
            carrier = $observation.credential_carrier
        }
        $observation.provider_sha256 = Get-Sha256String -Value (
            ConvertTo-CanonicalJson -Value $providerSummary
        )

        $origins = Get-PropertyValue -Object $configResult -Name "origins"
        $modelOrigin = Get-OriginType (
            Get-PropertyValue -Object $origins -Name "model"
        )
        $providerOrigin = Get-OriginType (
            Get-PropertyValue -Object $origins -Name "model_provider"
        )
        $layerTypes = @(
            @(Get-PropertyValue -Object $configResult -Name "layers") |
                ForEach-Object { Get-OriginType -Origin $_ }
        )
        $originSummary = [ordered]@{
            model = $modelOrigin
            provider = $providerOrigin
            layers = @($layerTypes)
        }
        $observation.origin_sha256 = Get-Sha256String -Value (
            ConvertTo-CanonicalJson -Value $originSummary
        )
        $observation.canary_model_match = ($model -ceq $script:ExpectedModel)
        $observation.canary_provider_match = (
            $providerKey -ceq $script:ExpectedProvider
        )
        $observation.shared_user_layer = (
            $observation.config_root_category -ceq "default_user" -and
            $modelOrigin -ceq "user" -and
            $providerOrigin -ceq "user" -and
            $layerTypes -contains "user"
        )
    } finally {
        if (-not $process.HasExited) {
            try {
                $process.Kill()
                $process.WaitForExit(5000)
            } catch {
            }
        }
        $process.Dispose()
    }

    return $observation
}

function Build-Result {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Observation,

        [Parameter(Mandatory = $true)]
        [bool]$TestOnly
    )

    $protocol = Get-PropertyValue -Object $Observation -Name "protocol"
    $carrier = Get-PropertyValue -Object $Observation -Name "credential_carrier"
    $checks = @(
        New-Check `
            -Name "executable_present" `
            -Passed (ConvertTo-Boolean (
                Get-PropertyValue -Object $Observation -Name "executable_present"
            )) `
            -FailureCode "CODEX_EXECUTABLE_MISSING"
        New-Check `
            -Name "version_exact" `
            -Passed (
                [string](Get-PropertyValue -Object $Observation -Name "version") -ceq
                $ExpectedVersion
            ) `
            -FailureCode "CODEX_VERSION_MISMATCH"
        New-Check `
            -Name "binary_digest" `
            -Passed (Test-Sha256 (
                Get-PropertyValue -Object $Observation -Name "binary_sha256"
            )) `
            -FailureCode "CODEX_BINARY_DIGEST_INVALID"
        New-Check `
            -Name "schema_digest" `
            -Passed (Test-Sha256 (
                Get-PropertyValue -Object $Observation -Name "schema_sha256"
            )) `
            -FailureCode "CODEX_SCHEMA_DIGEST_INVALID"
        New-Check `
            -Name "app_server_protocol" `
            -Passed (
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $protocol -Name "initialize"
                )) -and
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $protocol -Name "initialized"
                )) -and
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $protocol -Name "config_read"
                )) -and
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $protocol -Name "include_layers"
                ))
            ) `
            -FailureCode "CODEX_APP_SERVER_PROTOCOL_INCOMPLETE"
        New-Check `
            -Name "default_config_root" `
            -Passed (
                [string](
                    Get-PropertyValue -Object $Observation -Name "config_root_category"
                ) -ceq "default_user"
            ) `
            -FailureCode "CODEX_CONFIG_ROOT_NOT_DEFAULT"
        New-Check `
            -Name "canary_model" `
            -Passed (ConvertTo-Boolean (
                Get-PropertyValue -Object $Observation -Name "canary_model_match"
            )) `
            -FailureCode "CODEX_CANARY_MODEL_MISMATCH"
        New-Check `
            -Name "canary_provider" `
            -Passed (ConvertTo-Boolean (
                Get-PropertyValue -Object $Observation -Name "canary_provider_match"
            )) `
            -FailureCode "CODEX_CANARY_PROVIDER_MISMATCH"
        New-Check `
            -Name "summary_digests" `
            -Passed (
                (Test-Sha256 (
                    Get-PropertyValue -Object $Observation -Name "model_sha256"
                )) -and
                (Test-Sha256 (
                    Get-PropertyValue -Object $Observation -Name "provider_sha256"
                )) -and
                (Test-Sha256 (
                    Get-PropertyValue -Object $Observation -Name "origin_sha256"
                ))
            ) `
            -FailureCode "CODEX_SUMMARY_DIGEST_INVALID"
        New-Check `
            -Name "credential_carrier" `
            -Passed (
                -not (ConvertTo-Boolean (
                    Get-PropertyValue -Object $carrier -Name "env_key"
                )) -and
                (ConvertTo-Boolean (
                    Get-PropertyValue -Object $carrier -Name "direct_bearer"
                )) -and
                -not (ConvertTo-Boolean (
                    Get-PropertyValue -Object $carrier -Name "missing"
                ))
            ) `
            -FailureCode "CODEX_CREDENTIAL_CARRIER_MISMATCH"
        New-Check `
            -Name "shared_user_layer" `
            -Passed (ConvertTo-Boolean (
                Get-PropertyValue -Object $Observation -Name "shared_user_layer"
            )) `
            -FailureCode "CODEX_SHARED_USER_LAYER_MISSING"
    )
    $failed = @($checks | Where-Object { $_.outcome -ceq "failed" })
    $codes = @($failed | ForEach-Object { [string]$_.code })

    $exitCode = $script:ExitCodes.Completed
    $outcome = "passed"
    if ($codes -contains "CODEX_EXECUTABLE_MISSING") {
        $exitCode = $script:ExitCodes.StrictPrerequisiteBlocked
        $outcome = "blocked"
    } elseif ($codes.Count -gt 0) {
        $securityCodes = @(
            "CODEX_CONFIG_ROOT_NOT_DEFAULT",
            "CODEX_CANARY_MODEL_MISMATCH",
            "CODEX_CANARY_PROVIDER_MISMATCH",
            "CODEX_CREDENTIAL_CARRIER_MISMATCH",
            "CODEX_SHARED_USER_LAYER_MISSING"
        )
        $exitCode = if (@($codes | Where-Object { $_ -in $securityCodes }).Count -gt 0) {
            $script:ExitCodes.SecurityBoundaryFailed
        } else {
            $script:ExitCodes.AssertionFailed
        }
        $outcome = "failed"
    }

    return [ordered]@{
        schema_version = 1
        probe = "codex-app-server-config-read"
        role = $Role
        outcome = $outcome
        exit_code = $exitCode
        strict_gate_eligible = ($exitCode -eq 0 -and -not $TestOnly)
        test_only = $TestOnly
        expected_version = $ExpectedVersion
        version = Get-PropertyValue -Object $Observation -Name "version"
        binary_sha256 = Get-PropertyValue -Object $Observation -Name "binary_sha256"
        schema_sha256 = Get-PropertyValue -Object $Observation -Name "schema_sha256"
        config_root_category = Get-PropertyValue `
            -Object $Observation `
            -Name "config_root_category"
        protocol = [ordered]@{
            initialize = ConvertTo-Boolean (
                Get-PropertyValue -Object $protocol -Name "initialize"
            )
            initialized = ConvertTo-Boolean (
                Get-PropertyValue -Object $protocol -Name "initialized"
            )
            config_read = ConvertTo-Boolean (
                Get-PropertyValue -Object $protocol -Name "config_read"
            )
            include_layers = ConvertTo-Boolean (
                Get-PropertyValue -Object $protocol -Name "include_layers"
            )
        }
        model_sha256 = Get-PropertyValue -Object $Observation -Name "model_sha256"
        provider_sha256 = Get-PropertyValue `
            -Object $Observation `
            -Name "provider_sha256"
        origin_sha256 = Get-PropertyValue -Object $Observation -Name "origin_sha256"
        credential_carrier = [ordered]@{
            env_key = ConvertTo-Boolean (
                Get-PropertyValue -Object $carrier -Name "env_key"
            )
            direct_bearer = ConvertTo-Boolean (
                Get-PropertyValue -Object $carrier -Name "direct_bearer"
            )
            missing = ConvertTo-Boolean (
                Get-PropertyValue -Object $carrier -Name "missing"
            )
        }
        shared_user_layer = ConvertTo-Boolean (
            Get-PropertyValue -Object $Observation -Name "shared_user_layer"
        )
        checks = @($checks)
        blocking_reasons = @($codes)
    }
}

$testOnly = -not [string]::IsNullOrWhiteSpace($FixturePath)
try {
    $observation = if ($testOnly) {
        Read-FixtureObservation
    } else {
        Get-LiveObservation
    }
    $result = Build-Result -Observation $observation -TestOnly $testOnly
} catch {
    $result = [ordered]@{
        schema_version = 1
        probe = "codex-app-server-config-read"
        role = $Role
        outcome = "blocked"
        exit_code = $script:ExitCodes.StrictPrerequisiteBlocked
        strict_gate_eligible = $false
        test_only = $testOnly
        expected_version = $ExpectedVersion
        version = $null
        binary_sha256 = $null
        schema_sha256 = $null
        config_root_category = "unknown"
        protocol = [ordered]@{
            initialize = $false
            initialized = $false
            config_read = $false
            include_layers = $false
        }
        model_sha256 = $null
        provider_sha256 = $null
        origin_sha256 = $null
        credential_carrier = [ordered]@{
            env_key = $false
            direct_bearer = $false
            missing = $true
        }
        shared_user_layer = $false
        checks = @(
            [ordered]@{
                name = "probe_execution"
                outcome = "blocked"
                code = "CODEX_PROBE_UNAVAILABLE"
            }
        )
        blocking_reasons = @("CODEX_PROBE_UNAVAILABLE")
    }
}

[Console]::Out.WriteLine(($result | ConvertTo-Json -Compress -Depth 30))
exit [int]$result.exit_code
