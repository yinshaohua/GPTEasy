[CmdletBinding()]
param(
    [string]$InstallerPath,
    [string]$CandidateManifestPath,
    [string]$SecretPath,
    [switch]$ConfirmDisposableEnvironment,
    [switch]$RequireAuthenticode
)

$ErrorActionPreference = 'Stop'

if (-not $ConfirmDisposableEnvironment) {
    throw 'Windows UAT requires -ConfirmDisposableEnvironment before any mutable checks run.'
}
if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
    throw 'InstallerPath is required.'
}
if ([string]::IsNullOrWhiteSpace($CandidateManifestPath)) {
    throw 'CandidateManifestPath is required.'
}
if ([string]::IsNullOrWhiteSpace($SecretPath)) {
    $SecretPath = Join-Path $PSScriptRoot '..\.codex\skills\spike-findings-gpteasy\.secrets\provider.json'
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Get-Sha256Text([string]$Value) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Value)
        return ([System.BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
}

function Get-Sha256File([string]$Path) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
        $stream.Dispose()
        $sha256.Dispose()
    }
}

function Get-FileSignature([string]$Path) {
    if ($PSVersionTable.PSEdition -eq 'Desktop') {
        $module = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1'
        Import-Module -Name $module -Force -ErrorAction Stop
    } else {
        Import-Module -Name Microsoft.PowerShell.Security -ErrorAction Stop
    }
    return Get-AuthenticodeSignature -LiteralPath $Path
}

function Test-FileContainsBytes([string]$Path, [byte[]]$Needle) {
    $haystack = [System.IO.File]::ReadAllBytes($Path)
    if ($Needle.Length -eq 0 -or $haystack.Length -lt $Needle.Length) {
        return $false
    }
    for ($offset = 0; $offset -le $haystack.Length - $Needle.Length; $offset++) {
        $matches = $true
        for ($index = 0; $index -lt $Needle.Length; $index++) {
            if ($haystack[$offset + $index] -ne $Needle[$index]) {
                $matches = $false
                break
            }
        }
        if ($matches) {
            return $true
        }
    }
    return $false
}

function Get-InstalledRoots {
    return @(
        (Join-Path $env:LOCALAPPDATA 'Programs\GPTEasy')
        (Join-Path $env:LOCALAPPDATA 'GPTEasy')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Container }
}

function Get-GPTEasyProcesses([string]$ExecutablePath) {
    return @(Get-CimInstance Win32_Process -Filter "Name = 'gpteasy.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath.Equals($ExecutablePath, [StringComparison]::OrdinalIgnoreCase) })
}

function Confirm-UatStep(
    [System.Collections.Generic.List[object]]$Checks,
    [string]$Id,
    [string]$Prompt
) {
    Write-Host ''
    Write-Host $Prompt
    $answer = Read-Host 'Type PASS only after observing the required behavior'
    if ($answer -cne 'PASS') {
        throw "UAT step was not accepted: $Id"
    }
    $Checks.Add([ordered]@{ id = $Id; passed = $true })
}

$isWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
if (-not $isWindowsHost) {
    throw 'Windows UAT requires Windows.'
}
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Windows UAT requires an x64 operating system.'
}
if ([System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') {
    throw 'Windows UAT requires an x64 PowerShell process.'
}
$os = Get-CimInstance Win32_OperatingSystem
if ([int]$os.BuildNumber -lt 19045) {
    throw 'Windows UAT requires Windows build 19045 or newer.'
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$installer = Get-Item -LiteralPath (Resolve-Path -LiteralPath $InstallerPath).Path
if ($installer.Extension -ne '.exe' -or $installer.Name -notlike '*-setup.exe') {
    throw 'InstallerPath must point to a Tauri NSIS setup executable.'
}
$candidateManifestFile = Get-Item -LiteralPath (Resolve-Path -LiteralPath $CandidateManifestPath).Path

$branch = (& git -C $repoRoot branch --show-current | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $branch -ne 'main') {
    throw 'Windows UAT must run from the main branch.'
}
$worktree = (& git -C $repoRoot status --porcelain | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $worktree) {
    throw 'Windows UAT requires a clean worktree.'
}
$commit = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()

$secretFile = Get-Item -LiteralPath (Resolve-Path -LiteralPath $SecretPath).Path
& git -C $repoRoot check-ignore --quiet -- $secretFile.FullName
if ($LASTEXITCODE -ne 0) {
    throw 'The provider secret file must be ignored by Git.'
}
$secret = Get-Content -LiteralPath $secretFile.FullName -Raw | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($secret.base_url) -or
    [string]::IsNullOrWhiteSpace($secret.api_key) -or
    [string]::IsNullOrWhiteSpace($secret.model) -or
    $secret.api_key.Length -lt 8) {
    throw 'The provider secret file must contain non-empty base_url, api_key, and model fields.'
}
$providerUri = [Uri]([string]$secret.base_url)
$providerBuilder = New-Object System.UriBuilder($providerUri)
$providerBuilder.Path = $providerBuilder.Path.TrimEnd('/')
$normalizedBaseUrl = $providerBuilder.Uri.AbsoluteUri
$combinationMaterial = "gpteasy-provider-combination-v1`0$normalizedBaseUrl`0$($secret.model)`0$($secret.api_key)"
$combinationFingerprint = Get-Sha256Text $combinationMaterial

$codexCommand = Get-Command codex -ErrorAction SilentlyContinue
if (-not $codexCommand) {
    throw 'The current supported Codex CLI must be installed.'
}
$codexVersion = (& codex --version 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $codexVersion -notmatch '^codex-cli (\d+\.\d+\.\d+)') {
    throw 'Unable to read a supported Codex CLI version.'
}
if ([version]$Matches[1] -lt [version]'0.147.0') {
    throw 'Windows UAT requires Codex CLI 0.147.0 or newer.'
}
$desktopPackage = @(Get-AppxPackage -Name 'OpenAI.Codex*' -ErrorAction SilentlyContinue)
if ($desktopPackage.Count -ne 1) {
    throw 'Exactly one desktop Codex package must be installed.'
}
if ($desktopPackage[0].Architecture.ToString() -ne 'X64' -or
    $desktopPackage[0].Version -lt [version]'26.803.5235.0') {
    throw 'Windows UAT requires the supported x64 desktop Codex package version.'
}

$dataRoot = Join-Path $env:LOCALAPPDATA 'com.gpteasy.desktop'
$codexConfig = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex\config.toml'
if (Test-Path -LiteralPath $dataRoot) {
    throw 'Disposable UAT requires no pre-existing GPTEasy user data directory.'
}
if (Test-Path -LiteralPath $codexConfig) {
    throw 'Disposable UAT must begin with a missing current-user Codex config.toml.'
}
if (@(Get-InstalledRoots).Count -ne 0) {
    throw 'Disposable UAT requires GPTEasy to be uninstalled before the run.'
}
if (@(Get-StartApps | Where-Object { $_.Name -eq 'GPTEasy' }).Count -ne 0) {
    throw 'Disposable UAT requires no existing GPTEasy Start menu entry.'
}

$treeOutput = (& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot 'scripts\test-release-tree.ps1') -RepositoryRoot $repoRoot 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw 'The release tree gate failed.'
}
$treeReport = $treeOutput | ConvertFrom-Json
if (-not $treeReport.passed) {
    throw 'The release tree report was not clean.'
}

$installerHash = Get-Sha256File $installer.FullName
$signature = Get-FileSignature $installer.FullName
if ($signature.Status -ne 'Valid' -and $signature.Status -ne 'NotSigned') {
    throw "Installer Authenticode status is not acceptable: $($signature.Status)."
}
if ($RequireAuthenticode -and $signature.Status -ne 'Valid') {
    throw 'Formal release UAT requires a valid Authenticode signature.'
}
$candidateManifest = Get-Content -LiteralPath $candidateManifestFile.FullName -Raw | ConvertFrom-Json
$candidateArtifactName = [System.IO.Path]::GetFileName(([string]$candidateManifest.artifact.path).Replace('/', '\'))
if ($candidateManifest.schemaVersion -ne 1 -or
    $candidateManifest.issue -ne 11 -or
    $candidateManifest.gitCommit -ne $commit -or
    $candidateManifest.platform -ne 'windows-x64-current-user' -or
    $candidateArtifactName -ne $installer.Name -or
    $candidateManifest.artifact.sha256 -ne $installerHash -or
    [int64]$candidateManifest.artifact.size -ne $installer.Length -or
    $candidateManifest.artifact.authenticodeStatus -ne $signature.Status.ToString()) {
    throw 'The installer does not match the candidate manifest for the current commit.'
}
$candidateVerification = $candidateManifest.verification
if ($candidateVerification.frontendCheck -ne 'passed' -or
    $candidateVerification.frontendTests -ne 'passed' -or
    $candidateVerification.rustTests -ne 'passed' -or
    $candidateVerification.acceptanceGate -ne 'passed' -or
    $candidateVerification.releaseTree -ne 'passed') {
    throw 'The candidate manifest does not record every required build gate as passed.'
}
$candidateManifestSha256 = Get-Sha256File $candidateManifestFile.FullName
$startMenuBefore = @(Get-StartApps | Where-Object { $_.Name -eq 'GPTEasy' }).Count
$checks = [System.Collections.Generic.List[object]]::new()
$checks.Add([ordered]@{ id = 'release_tree'; passed = $true })

$install = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($install.ExitCode -ne 0) {
    throw "Installer failed with exit code $($install.ExitCode)."
}
Start-Sleep -Seconds 2
$installedRoots = @(Get-InstalledRoots)
if ($installedRoots.Count -ne 1) {
    throw "Expected one current-user install root, found $($installedRoots.Count)."
}
$installRoot = (Resolve-Path -LiteralPath $installedRoots[0]).Path
$localRoot = (Resolve-Path -LiteralPath $env:LOCALAPPDATA).Path
if (-not $installRoot.StartsWith($localRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The installer escaped the current-user LocalAppData directory.'
}
$app = Get-Item -LiteralPath (Join-Path $installRoot 'gpteasy.exe')
$uninstaller = Get-Item -LiteralPath (Join-Path $installRoot 'uninstall.exe')
$startMenuAfterInstall = @(Get-StartApps | Where-Object { $_.Name -eq 'GPTEasy' }).Count
if ($startMenuAfterInstall -le $startMenuBefore) {
    throw 'The installer did not create a current-user Start menu entry.'
}
$checks.Add([ordered]@{ id = 'install_current_user'; passed = $true })

Start-Process -FilePath $app.FullName | Out-Null
Confirm-UatStep $checks 'application_launch' 'Confirm that the installed GPTEasy settings window is visible and usable.'
Confirm-UatStep $checks 'real_provider_validation' 'Using values read privately from provider.json, complete model discovery and the Responses streaming tool-call validation.'
Confirm-UatStep $checks 'provider_save_and_switch' 'Explicitly save the verified provider and apply it to the current-user Codex environment.'
Confirm-UatStep $checks 'pending_restart' 'With an old Codex consumer still running, apply a change and confirm GPTEasy reports pending restart without terminating it.'
Confirm-UatStep $checks 'cli_new_process_read' 'Exit the old CLI, start a new real Codex CLI process, and confirm a real request uses the target provider and credential carrier.'
Confirm-UatStep $checks 'desktop_new_process_read' 'Close the old desktop Codex, start a new desktop Codex process, and confirm a real request uses the target provider and credential carrier.'
Confirm-UatStep $checks 'restore_last_config' 'Use Restore last config and confirm the current-user Codex environment returns to the previous complete state.'
Confirm-UatStep $checks 'external_config_takeover' 'Create a valid external provider config, rescan, review the scope, and explicitly take it over without losing unrelated TOML fields.'
Confirm-UatStep $checks 'managed_conflict' 'Externally damage or alter the managed block and confirm GPTEasy blocks writes until explicit conflict handling.'
Confirm-UatStep $checks 'openai_login_mode' 'Resolve the managed conflict, then switch to OpenAI login mode and confirm GPTEasy does not read, save, or delete the login token.'
Confirm-UatStep $checks 'provider_combination_applied' 'Switch back to the provider from provider.json and confirm it is the current provider.'
Confirm-UatStep $checks 'tray_residency' 'Close the settings window, confirm GPTEasy remains in the tray, reopen settings, then use the tray Exit command.'

Start-Sleep -Seconds 1
if (@(Get-GPTEasyProcesses $app.FullName).Count -ne 0) {
    throw 'GPTEasy is still running; use the tray Exit command before overwrite installation.'
}
$appliedConfig = Get-Content -LiteralPath $codexConfig -Raw
if (-not $appliedConfig.Contains($normalizedBaseUrl) -or
    -not $appliedConfig.Contains([string]$secret.model) -or
    $appliedConfig.Contains([string]$secret.api_key)) {
    throw 'The applied Codex config does not contain the provider metadata or contains the API key.'
}
$credentialsPath = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex\auth.json'
$appliedCredentials = Get-Content -LiteralPath $credentialsPath -Raw | ConvertFrom-Json
if ($appliedCredentials.auth_mode -ne 'apikey' -or
    $appliedCredentials.OPENAI_API_KEY -cne [string]$secret.api_key) {
    throw 'The Codex credential carrier does not contain the provider API key.'
}
$checks.Add([ordered]@{ id = 'provider_combination_match'; passed = $true })
$stateDatabase = Join-Path $dataRoot 'state.sqlite3'
if (-not (Test-Path -LiteralPath $stateDatabase -PathType Leaf)) {
    throw 'The installed application did not create its state database.'
}
$stateHashBeforeOverwrite = Get-Sha256File $stateDatabase

$overwrite = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($overwrite.ExitCode -ne 0) {
    throw "Overwrite installation failed with exit code $($overwrite.ExitCode)."
}
Start-Sleep -Seconds 2
if (-not (Test-Path -LiteralPath $app.FullName -PathType Leaf) -or
    -not (Test-Path -LiteralPath $uninstaller.FullName -PathType Leaf)) {
    throw 'Overwrite installation removed the application or uninstaller.'
}
$stateHashAfterOverwrite = Get-Sha256File $stateDatabase
if ($stateHashAfterOverwrite -ne $stateHashBeforeOverwrite) {
    throw 'Overwrite installation changed GPTEasy user data.'
}
$checks.Add([ordered]@{ id = 'overwrite_install'; passed = $true })

Start-Process -FilePath $app.FullName | Out-Null
Confirm-UatStep $checks 'overwrite_launch' 'Confirm the overwritten installation starts and retains the provider catalog and environment state, then exit from the tray.'
Start-Sleep -Seconds 1
if (@(Get-GPTEasyProcesses $app.FullName).Count -ne 0) {
    throw 'GPTEasy is still running; use the tray Exit command before uninstalling.'
}
$stateHashBeforeUninstall = Get-Sha256File $stateDatabase

$uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($uninstall.ExitCode -ne 0) {
    throw "Uninstaller failed with exit code $($uninstall.ExitCode)."
}
Start-Sleep -Seconds 2
if (Test-Path -LiteralPath $installRoot) {
    throw 'The application install directory remains after uninstall.'
}
if (@(Get-StartApps | Where-Object { $_.Name -eq 'GPTEasy' }).Count -ne 0) {
    throw 'The Start menu entry remains after uninstall.'
}
if (-not (Test-Path -LiteralPath $stateDatabase -PathType Leaf)) {
    throw 'Uninstall removed GPTEasy user data.'
}
$stateHashAfterUninstall = Get-Sha256File $stateDatabase
if ($stateHashAfterUninstall -ne $stateHashBeforeUninstall) {
    throw 'Uninstall changed GPTEasy user data.'
}
$checks.Add([ordered]@{ id = 'uninstall'; passed = $true })
$checks.Add([ordered]@{ id = 'data_retention'; passed = $true })

$sessionRoot = Join-Path $repoRoot "src-tauri\target\uat\$((Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ'))"
New-Item -ItemType Directory -Path $sessionRoot -Force | Out-Null
$pendingEvidencePath = Join-Path $sessionRoot 'evidence.pending.json'
$evidencePath = Join-Path $sessionRoot 'evidence.json'
$checks.Add([ordered]@{ id = 'credential_leak_scan'; passed = $true })
$evidence = [ordered]@{
    schemaVersion = 1
    issue = 11
    evidenceOrigin = 'interactive-windows-uat'
    completedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    gitCommit = $commit
    candidateManifestSha256 = $candidateManifestSha256
    platform = [ordered]@{
        os = 'windows'
        architecture = 'x64'
        build = [int]$os.BuildNumber
    }
    codexCliVersion = $codexVersion
    desktopCodexVersion = $desktopPackage[0].Version.ToString()
    providerCombinationFingerprint = $combinationFingerprint
    artifact = [ordered]@{
        fileName = $installer.Name
        sha256 = $installerHash
        size = $installer.Length
        authenticodeStatus = $signature.Status.ToString()
    }
    checks = @($checks)
}
$json = $evidence | ConvertTo-Json -Depth 10
if ($json.Contains([string]$secret.api_key)) {
    throw 'UAT evidence contained the provider API key; evidence was not written.'
}
Write-Utf8NoBom $pendingEvidencePath $json
$apiKeyBytes = [System.Text.Encoding]::UTF8.GetBytes([string]$secret.api_key)
$leaked = Get-ChildItem -LiteralPath $sessionRoot -Recurse -File | Where-Object {
    Test-FileContainsBytes $_.FullName $apiKeyBytes
} | Select-Object -First 1
if ($leaked) {
    Remove-Item -LiteralPath $pendingEvidencePath -Force -ErrorAction SilentlyContinue
    throw 'UAT output contained the provider API key; evidence was not retained.'
}
Move-Item -LiteralPath $pendingEvidencePath -Destination $evidencePath
Write-Output $json
