[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = $PSScriptRoot
$repoRoot = (Resolve-Path -LiteralPath (Join-Path (Split-Path -Parent $scriptRoot) "..")).Path
$verifier = Join-Path $scriptRoot "verify-npm-package-allowlist.ps1"
$fixture = Join-Path $repoRoot "tests/fixtures/contracts/npm-package-allowlist.json"
$powershellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
$script:TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("gpteasy-npm-allowlist-test-" + [guid]::NewGuid().ToString("N"))
$script:EnvironmentSnapshot = @{}
$script:Canary = "GPTEASY-TOKEN-CANARY-7E1B5A"

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Content
    )

    [System.IO.File]::WriteAllText($Path, $Content, (New-Object System.Text.UTF8Encoding($false)))
}

function Assert-Condition {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition
    )

    if (-not $Condition) {
        throw [System.InvalidOperationException]::new("npm allowlist self-test assertion failed")
    }
}

function Assert-ExitCode {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Result,

        [Parameter(Mandatory = $true)]
        [int]$Expected
    )

    Assert-Condition -Condition ([int]$Result.ExitCode -eq $Expected)
}

function Assert-CleanText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    Assert-Condition -Condition ($Text -notmatch [regex]::Escape($script:Canary))
    Assert-Condition -Condition ($Text -notmatch "(?i)authorization")
    Assert-Condition -Condition ($Text -notmatch "(?i)npmrc")
}

function Invoke-Verifier {
    param(
        [Parameter(Mandatory = $true)]
        [string]$AllowlistPath,

        [Parameter(Mandatory = $false)]
        [string]$MetadataPath,

        [Parameter(Mandatory = $false)]
        [string]$NpmPath
    )

    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        $verifier,
        "-Allowlist",
        $AllowlistPath
    )
    if (-not [string]::IsNullOrWhiteSpace($MetadataPath)) {
        $arguments += @("-MetadataFile", $MetadataPath)
    }
    if (-not [string]::IsNullOrWhiteSpace($NpmPath)) {
        $arguments += @("-NpmExecutable", $NpmPath)
    }

    $outputLines = @(& $powershellExecutable @arguments 2>&1)
    $exitCode = $LASTEXITCODE
    return [pscustomobject]@{
        ExitCode = $exitCode
        Output   = ($outputLines | Out-String)
    }
}

function Write-Contract {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Contract,

        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $json = $Contract | ConvertTo-Json -Depth 10
    Write-Utf8NoBom -Path $Path -Content ($json + "`n")
}

function Read-Contract {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    return (Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json)
}

function Set-TestEnvironment {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $false)]
        [AllowNull()]
        [string]$Value
    )

    if (-not $script:EnvironmentSnapshot.ContainsKey($Name)) {
        $script:EnvironmentSnapshot[$Name] = [System.Environment]::GetEnvironmentVariable($Name, "Process")
    }
    [System.Environment]::SetEnvironmentVariable($Name, $Value, "Process")
}

function Restore-TestEnvironment {
    foreach ($name in $script:EnvironmentSnapshot.Keys) {
        [System.Environment]::SetEnvironmentVariable($name, $script:EnvironmentSnapshot[$name], "Process")
    }
}

try {
    Assert-Condition -Condition (Test-Path -LiteralPath $verifier -PathType Leaf)
    Assert-Condition -Condition (Test-Path -LiteralPath $fixture -PathType Leaf)

    New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null
    $metadataDirectory = Join-Path $script:TempRoot "metadata"
    $maliciousProject = Join-Path $script:TempRoot "malicious-project"
    New-Item -ItemType Directory -Path $metadataDirectory -Force | Out-Null
    New-Item -ItemType Directory -Path $maliciousProject -Force | Out-Null

    $stubScript = Join-Path $script:TempRoot "npm-stub.ps1"
    $stubCommand = Join-Path $script:TempRoot "npm-stub.cmd"
    $stubLog = Join-Path $script:TempRoot "npm-stub.log"
    $stubMetadata = Join-Path $script:TempRoot "stub-metadata.json"
    $maliciousUserConfig = Join-Path $maliciousProject "user.npmrc"
    $maliciousGlobalConfig = Join-Path $maliciousProject "global.npmrc"
    $maliciousProjectConfig = Join-Path $maliciousProject ".npmrc"

    Copy-Item -LiteralPath $fixture -Destination $stubMetadata -Force
    Write-Utf8NoBom -Path $stubCommand -Content "@echo off`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"%~dp0npm-stub.ps1`" %*`nexit /b %ERRORLEVEL%`n"
    Write-Utf8NoBom -Path $stubScript -Content @'
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Arguments
)

$logPath = $env:GPTEASY_NPM_STUB_LOG
$metadataPath = $env:GPTEASY_NPM_STUB_METADATA
$record = [System.Collections.Generic.List[string]]::new()
$record.Add(($Arguments -join " "))

if ($env:GPTEASY_NPM_STUB_FAIL -eq "1") {
    $record.Add("forcedFailure=true")
    [System.IO.File]::AppendAllText($logPath, (($record -join ";") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
    [Console]::Error.WriteLine("stub query failed")
    exit 17
}

$registryArgument = @($Arguments | Where-Object { $_ -like "--registry=*" }) | Select-Object -First 1
$userConfigArgument = @($Arguments | Where-Object { $_ -like "--userconfig=*" }) | Select-Object -First 1
$globalConfigArgument = @($Arguments | Where-Object { $_ -like "--globalconfig=*" }) | Select-Object -First 1
$userConfigPath = ([string]$userConfigArgument).Substring(13)
$globalConfigPath = ([string]$globalConfigArgument).Substring(15)
$cwdHasNpmrc = Test-Path -LiteralPath (Join-Path (Get-Location) ".npmrc") -PathType Leaf
$userConfigEmpty = [System.IO.File]::Exists($userConfigPath) -and ((Get-Item -LiteralPath $userConfigPath).Length -eq 0)
$globalConfigEmpty = [System.IO.File]::Exists($globalConfigPath) -and ((Get-Item -LiteralPath $globalConfigPath).Length -eq 0)
$sensitiveEnvironmentPresent = (
    -not [string]::IsNullOrEmpty([System.Environment]::GetEnvironmentVariable("NPM_CONFIG_REGISTRY", "Process")) -or
    -not [string]::IsNullOrEmpty([System.Environment]::GetEnvironmentVariable("NPM_TOKEN", "Process")) -or
    -not [string]::IsNullOrEmpty([System.Environment]::GetEnvironmentVariable("NODE_AUTH_TOKEN", "Process"))
)
$record.Add("registry=$registryArgument")
$record.Add("cwdHasNpmrc=$cwdHasNpmrc")
$record.Add("userConfigEmpty=$userConfigEmpty")
$record.Add("globalConfigEmpty=$globalConfigEmpty")
$record.Add("sensitiveEnvironmentPresent=$sensitiveEnvironmentPresent")

$contract = Get-Content -Raw -LiteralPath $metadataPath | ConvertFrom-Json
$packageSpec = [string]$Arguments[1]
$entry = @($contract.packages | Where-Object { "$($_.name)@$($_.version)" -ceq $packageSpec }) | Select-Object -First 1
if ($null -eq $entry) {
    [System.IO.File]::AppendAllText($logPath, (($record -join ";") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
    [Console]::Error.WriteLine("stub package lookup failed")
    exit 17
}

[System.IO.File]::AppendAllText($logPath, (($record -join ";") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
[pscustomobject]@{
    name       = $entry.name
    version    = $entry.version
    repository = [pscustomobject]@{
        type = "git"
        url  = "git+https://github.com/$(([string]$entry.repository).Substring(19)).git"
    }
} | ConvertTo-Json -Compress
exit 0
'@

    Set-TestEnvironment -Name "GPTEASY_NPM_STUB_LOG" -Value $stubLog
    Set-TestEnvironment -Name "GPTEASY_NPM_STUB_METADATA" -Value $stubMetadata
    Set-TestEnvironment -Name "GPTEASY_NPM_STUB_FAIL" -Value $null

    Write-Utf8NoBom -Path $maliciousProjectConfig -Content "registry=https://private.example.invalid/`n//private.example.invalid/:_authToken=$($script:Canary)`n"
    Write-Utf8NoBom -Path $maliciousUserConfig -Content "//private.example.invalid/:_authToken=$($script:Canary)`n"
    Write-Utf8NoBom -Path $maliciousGlobalConfig -Content "registry=https://private.example.invalid/`n"
    Set-TestEnvironment -Name "NPM_CONFIG_REGISTRY" -Value "https://private.example.invalid/"
    Set-TestEnvironment -Name "NPM_CONFIG_USERCONFIG" -Value $maliciousUserConfig
    Set-TestEnvironment -Name "NPM_CONFIG_GLOBALCONFIG" -Value $maliciousGlobalConfig
    Set-TestEnvironment -Name "NPM_TOKEN" -Value $script:Canary
    Set-TestEnvironment -Name "NODE_AUTH_TOKEN" -Value $script:Canary

    $beforePackageState = (& git -C $repoRoot status --short -- package.json package-lock.json node_modules | Out-String).Trim()

    Push-Location -LiteralPath $maliciousProject
    try {
        $positiveQuery = Invoke-Verifier -AllowlistPath $fixture -NpmPath $stubCommand
    } finally {
        Pop-Location
    }
    Assert-ExitCode -Result $positiveQuery -Expected 0
    Assert-CleanText -Text $positiveQuery.Output

    $requests = @(Get-Content -LiteralPath $stubLog)
    Assert-Condition -Condition ($requests.Count -eq 7)
    foreach ($request in $requests) {
        Assert-Condition -Condition ($request -match "registry=--registry=https://registry\.npmjs\.org/")
        Assert-Condition -Condition ($request -match "cwdHasNpmrc=False")
        Assert-Condition -Condition ($request -match "userConfigEmpty=True")
        Assert-Condition -Condition ($request -match "globalConfigEmpty=True")
        Assert-Condition -Condition ($request -match "sensitiveEnvironmentPresent=False")
        Assert-Condition -Condition ($request -notmatch [regex]::Escape($script:Canary))
        Assert-Condition -Condition ($request -notmatch "private\.example\.invalid")
    }

    $failureLogLength = $requests.Count
    Set-TestEnvironment -Name "GPTEASY_NPM_STUB_FAIL" -Value "1"
    $queryFailure = Invoke-Verifier -AllowlistPath $fixture -NpmPath $stubCommand
    Assert-ExitCode -Result $queryFailure -Expected 1
    Assert-CleanText -Text $queryFailure.Output
    Set-TestEnvironment -Name "GPTEASY_NPM_STUB_FAIL" -Value $null

    $positiveMetadata = Join-Path $metadataDirectory "positive.json"
    Copy-Item -LiteralPath $fixture -Destination $positiveMetadata -Force
    $positiveFixture = Invoke-Verifier -AllowlistPath $fixture -MetadataPath $positiveMetadata
    Assert-ExitCode -Result $positiveFixture -Expected 0
    Assert-CleanText -Text $positiveFixture.Output

    $mutations = @(
        [pscustomobject]@{ Name = "wrong-name"; Action = { param($c) $c.packages[0].name = "wrong-package" } },
        [pscustomobject]@{ Name = "wrong-version"; Action = { param($c) $c.packages[0].version = "0.0.0" } },
        [pscustomobject]@{ Name = "wrong-repository"; Action = { param($c) $c.packages[0].repository = "https://github.com/example/forged" } },
        [pscustomobject]@{ Name = "missing-field"; Action = { param($c) $c.packages[0].PSObject.Properties.Remove("repository") } },
        [pscustomobject]@{ Name = "duplicate-entry"; Action = { param($c) $c.packages = @($c.packages) + $c.packages[0] } },
        [pscustomobject]@{ Name = "extra-entry"; Action = { param($c) $c.packages = @($c.packages) + [pscustomobject]@{ name = "extra-package"; version = "1.0.0"; repository = "https://github.com/example/extra-package" } } }
    )
    foreach ($mutation in $mutations) {
        $contract = Read-Contract -Path $fixture
        & $mutation.Action $contract
        $mutationPath = Join-Path $metadataDirectory ($mutation.Name + ".json")
        Write-Contract -Contract $contract -Path $mutationPath
        $mutationResult = Invoke-Verifier -AllowlistPath $fixture -MetadataPath $mutationPath
        Assert-ExitCode -Result $mutationResult -Expected 1
        Assert-CleanText -Text $mutationResult.Output
    }

    $afterPackageState = (& git -C $repoRoot status --short -- package.json package-lock.json node_modules | Out-String).Trim()
    Assert-Condition -Condition ($beforePackageState -ceq $afterPackageState)
    Assert-Condition -Condition ($failureLogLength -eq 7)

    foreach ($generatedFile in @(Get-ChildItem -LiteralPath $script:TempRoot -Recurse -File)) {
        if ($generatedFile.FullName -eq $maliciousProjectConfig -or
            $generatedFile.FullName -eq $maliciousUserConfig -or
            $generatedFile.FullName -eq $maliciousGlobalConfig) {
            continue
        }
        $generatedText = [System.IO.File]::ReadAllText($generatedFile.FullName)
        Assert-Condition -Condition ($generatedText -notmatch [regex]::Escape($script:Canary))
    }

    $verifierSource = [System.IO.File]::ReadAllText($verifier)
    Assert-Condition -Condition ($verifierSource -notmatch "(?im)\bnpm\s+(install|ci|update)\b")

    Write-Output "npm package allowlist self-test passed: isolation, positive control, six identity mutations, and query failure"
    exit 0
} catch {
    Write-Output "npm package allowlist self-test failed; sensitive npm configuration is not emitted."
    exit 1
} finally {
    Restore-TestEnvironment
    if (Test-Path -LiteralPath $script:TempRoot) {
        Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
