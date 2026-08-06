[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Allowlist,

    [Parameter(Mandatory = $false)]
    [string]$MetadataFile,

    [Parameter(Mandatory = $false)]
    [string]$NpmExecutable = "npm"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:FixedRegistry = "https://registry.npmjs.org/"
$script:RemovedEnvironmentVariables = @()
$script:TempRoot = $null
$script:LocationPushed = $false
$script:EnvironmentSnapshot = @{}
$script:VerificationSucceeded = $false
$script:ExitCode = 1

function Throw-VerificationFailure {
    throw [System.InvalidOperationException]::new("npm package identity verification failed")
}

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    $actual = @($Object.PSObject.Properties | ForEach-Object { $_.Name })
    $expectedSorted = @($Expected | Sort-Object)
    $actualSorted = @($actual | Sort-Object)
    if (($actualSorted -join "`n") -cne ($expectedSorted -join "`n")) {
        Throw-VerificationFailure
    }
}

function Normalize-Repository {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Repository
    )

    $raw = $null
    if ($Repository -is [string]) {
        $raw = $Repository
    } elseif ($null -ne $Repository -and $null -ne $Repository.PSObject.Properties["url"]) {
        $raw = [string]$Repository.url
    } else {
        Throw-VerificationFailure
    }

    $raw = $raw.Trim()
    if ([string]::IsNullOrWhiteSpace($raw)) {
        Throw-VerificationFailure
    }

    if ($raw -match "^(?i)github:(?<path>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)$") {
        return "https://github.com/$($Matches.path)"
    }
    if ($raw -match "^(?i)git@github\.com:(?<path>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)(?:\.git)?$") {
        $path = $Matches.path -replace "(?i)\.git$", ""
        return "https://github.com/$path"
    }

    $normalized = $raw -replace "^(?i)git\+", ""
    if ($normalized -match "^(?i)git://github\.com/(?<path>.+)$") {
        $normalized = "https://github.com/$($Matches.path)"
    }

    if ($normalized -notmatch "^(?i)https://github\.com/(?<path>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)(?:\.git)?$") {
        Throw-VerificationFailure
    }

    $path = $Matches.path -replace "(?i)\.git$", ""
    return "https://github.com/$path"
}

function Read-Contract {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Throw-VerificationFailure
    }

    try {
        $raw = [System.IO.File]::ReadAllText($Path)
        $document = $raw | ConvertFrom-Json
    } catch {
        Throw-VerificationFailure
    }

    if ($null -eq $document) {
        Throw-VerificationFailure
    }

    Assert-ExactProperties -Object $document -Expected @("schemaVersion", "registry", "packages")
    if ([int]$document.schemaVersion -ne 1) {
        Throw-VerificationFailure
    }
    if ([string]$document.registry -cne $script:FixedRegistry) {
        Throw-VerificationFailure
    }

    $packageItems = @($document.packages)
    if ($packageItems.Count -eq 0) {
        Throw-VerificationFailure
    }

    $normalizedPackages = @()
    $seenNames = @{}
    foreach ($item in $packageItems) {
        if ($null -eq $item) {
            Throw-VerificationFailure
        }
        Assert-ExactProperties -Object $item -Expected @("name", "version", "repository")

        $name = [string]$item.name
        $version = [string]$item.version
        if ([string]::IsNullOrWhiteSpace($name) -or [string]::IsNullOrWhiteSpace($version)) {
            Throw-VerificationFailure
        }
        if ($seenNames.ContainsKey($name)) {
            Throw-VerificationFailure
        }
        $seenNames[$name] = $true

        $normalizedPackages += [pscustomobject]@{
                name       = $name
                version    = $version
                repository = Normalize-Repository -Repository $item.repository
            }
    }

    return [pscustomobject]@{
        schemaVersion = 1
        registry      = $script:FixedRegistry
        packages      = @($normalizedPackages)
    }
}

function Normalize-NpmMetadata {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Metadata
    )

    if ($null -eq $Metadata) {
        Throw-VerificationFailure
    }
    Assert-ExactProperties -Object $Metadata -Expected @("name", "version", "repository")

    $name = [string]$Metadata.name
    $version = [string]$Metadata.version
    if ([string]::IsNullOrWhiteSpace($name) -or [string]::IsNullOrWhiteSpace($version)) {
        Throw-VerificationFailure
    }

    return [pscustomobject]@{
        name       = $name
        version    = $version
        repository = Normalize-Repository -Repository $Metadata.repository
    }
}

function Assert-ContractMatches {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Expected,

        [Parameter(Mandatory = $true)]
        [object]$Actual
    )

    $expectedPackages = @($Expected.packages)
    $actualPackages = @($Actual.packages)
    if ($expectedPackages.Count -ne $actualPackages.Count) {
        Throw-VerificationFailure
    }

    $actualByName = @{}
    foreach ($item in $actualPackages) {
        if ($actualByName.ContainsKey($item.name)) {
            Throw-VerificationFailure
        }
        $actualByName[$item.name] = $item
    }

    foreach ($expectedItem in $expectedPackages) {
        if (-not $actualByName.ContainsKey($expectedItem.name)) {
            Throw-VerificationFailure
        }
        $actualItem = $actualByName[$expectedItem.name]
        if ($actualItem.version -cne $expectedItem.version) {
            Throw-VerificationFailure
        }
        if ($actualItem.repository -cne $expectedItem.repository) {
            Throw-VerificationFailure
        }
    }
}

function Assert-NpmMetadataMatches {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Expected,

        [Parameter(Mandatory = $true)]
        [object]$Actual
    )

    $normalized = Normalize-NpmMetadata -Metadata $Actual
    if ($normalized.name -cne $Expected.name) {
        Throw-VerificationFailure
    }
    if ($normalized.version -cne $Expected.version) {
        Throw-VerificationFailure
    }
    if ($normalized.repository -cne $Expected.repository) {
        Throw-VerificationFailure
    }
}

function Get-NpmMetadata {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Expected,

        [Parameter(Mandatory = $true)]
        [string]$UserConfig,

        [Parameter(Mandatory = $true)]
        [string]$GlobalConfig,

        [Parameter(Mandatory = $true)]
        [string]$WorkingDirectory
    )

    $packageSpec = "$($Expected.name)@$($Expected.version)"
    $stderrPath = Join-Path $script:TempRoot ("npm-stderr-" + [guid]::NewGuid().ToString("N") + ".log")
    $arguments = @(
        "view",
        $packageSpec,
        "name",
        "version",
        "repository",
        "--json",
        "--registry=$($script:FixedRegistry)",
        "--userconfig=$UserConfig",
        "--globalconfig=$GlobalConfig",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        "--loglevel=error"
    )
    try {
        $stdoutLines = @(& $NpmExecutable @arguments 2> $stderrPath)
        $exitCode = $LASTEXITCODE
    } catch {
        Throw-VerificationFailure
    }

    if ($exitCode -ne 0) {
        Throw-VerificationFailure
    }

    $stdout = ($stdoutLines -join "`n").Trim()
    if ([string]::IsNullOrWhiteSpace($stdout)) {
        Throw-VerificationFailure
    }

    try {
        return ($stdout | ConvertFrom-Json)
    } catch {
        Throw-VerificationFailure
    }
}

function Remove-NpmSensitiveEnvironment {
    $processEnvironment = [System.Environment]::GetEnvironmentVariables("Process")
    foreach ($keyObject in $processEnvironment.Keys) {
        $name = [string]$keyObject
        $isNpmSensitive = (
            ($name -match "^(?i)NPM_CONFIG_.*(REGISTRY|AUTH|TOKEN|USERCONFIG|GLOBALCONFIG)") -or
            ($name -match "^(?i)(NPM_TOKEN|NODE_AUTH_TOKEN)$")
        )
        if ($isNpmSensitive) {
            $script:EnvironmentSnapshot[$name] = [System.Environment]::GetEnvironmentVariable($name, "Process")
            [System.Environment]::SetEnvironmentVariable($name, $null, "Process")
            $script:RemovedEnvironmentVariables += $name
        }
    }
}

function Restore-NpmSensitiveEnvironment {
    foreach ($name in $script:EnvironmentSnapshot.Keys) {
        [System.Environment]::SetEnvironmentVariable($name, $script:EnvironmentSnapshot[$name], "Process")
    }
}

function Test-AncestorHasNpmrc {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $current = New-Object System.IO.DirectoryInfo($Path)
    while ($null -ne $current) {
        if (Test-Path -LiteralPath (Join-Path $current.FullName ".npmrc") -PathType Leaf) {
            return $true
        }
        $current = $current.Parent
    }
    return $false
}

function New-IsolatedTempRoot {
    $baseCandidates = @()
    if (-not [string]::IsNullOrWhiteSpace($env:PUBLIC)) {
        $baseCandidates += Join-Path $env:PUBLIC "GPTEasy\Temp"
    }
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramData)) {
        $baseCandidates += Join-Path $env:ProgramData "GPTEasy\Temp"
    }
    if (-not [string]::IsNullOrWhiteSpace($env:SystemRoot)) {
        $baseCandidates += Join-Path $env:SystemRoot "Temp"
    }

    foreach ($base in $baseCandidates) {
        $candidate = $null
        try {
            New-Item -ItemType Directory -Path $base -Force | Out-Null
            if (Test-AncestorHasNpmrc -Path $base) {
                continue
            }
            $candidate = Join-Path $base ("gpteasy-npm-verifier-" + [guid]::NewGuid().ToString("N"))
            New-Item -ItemType Directory -Path $candidate -Force | Out-Null
            $probe = Join-Path $candidate "write-probe"
            [System.IO.File]::WriteAllText($probe, [string]::Empty, (New-Object System.Text.UTF8Encoding($false)))
            Remove-Item -LiteralPath $probe -Force -ErrorAction Stop
            return $candidate
        } catch {
            if ($null -ne $candidate -and (Test-Path -LiteralPath $candidate -PathType Container)) {
                Remove-Item -LiteralPath $candidate -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    Throw-VerificationFailure
}

try {
    $allowlistPath = (Resolve-Path -LiteralPath $Allowlist -ErrorAction Stop).Path
    $expected = Read-Contract -Path $allowlistPath

    if (-not [string]::IsNullOrWhiteSpace($MetadataFile) -and
        -not [string]::IsNullOrWhiteSpace($NpmExecutable) -and
        $NpmExecutable -ne "npm") {
        Throw-VerificationFailure
    }

    if (-not [string]::IsNullOrWhiteSpace($MetadataFile)) {
        $metadataPath = (Resolve-Path -LiteralPath $MetadataFile -ErrorAction Stop).Path
    } else {
        $metadataPath = $null
    }

    $script:TempRoot = New-IsolatedTempRoot
    $userConfig = Join-Path $script:TempRoot "user.npmrc"
    $globalConfig = Join-Path $script:TempRoot "global.npmrc"
    $workingDirectory = Join-Path $script:TempRoot "work"
    New-Item -ItemType Directory -Path $workingDirectory -Force | Out-Null
    [System.IO.File]::WriteAllText($userConfig, [string]::Empty, (New-Object System.Text.UTF8Encoding($false)))
    [System.IO.File]::WriteAllText($globalConfig, [string]::Empty, (New-Object System.Text.UTF8Encoding($false)))

    Push-Location -LiteralPath $workingDirectory
    $script:LocationPushed = $true
    Remove-NpmSensitiveEnvironment

    if ($null -ne $metadataPath) {
        $actualContract = Read-Contract -Path $metadataPath
        Assert-ContractMatches -Expected $expected -Actual $actualContract
    } else {
        foreach ($expectedPackage in @($expected.packages)) {
            $metadata = Get-NpmMetadata `
                -Expected $expectedPackage `
                -UserConfig $userConfig `
                -GlobalConfig $globalConfig `
                -WorkingDirectory $workingDirectory
            Assert-NpmMetadataMatches -Expected $expectedPackage -Actual $metadata
        }
    }

    $script:VerificationSucceeded = $true
    Write-Output ("npm package allowlist verified: {0} package identities" -f @($expected.packages).Count)
    $script:ExitCode = 0
} catch {
    Write-Output "npm package allowlist verification failed; sensitive npm configuration is not emitted."
    $script:ExitCode = 1
} finally {
    try {
        if ($script:LocationPushed) {
            Restore-NpmSensitiveEnvironment
            Pop-Location
        }
    } catch {
    }

    if ($null -ne $script:TempRoot) {
        Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

exit $script:ExitCode
