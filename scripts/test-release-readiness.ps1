[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Acceptance', 'Release')]
    [string]$Mode,
    [Parameter(Mandatory = $true)]
    [string]$EvidencePath,
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [Parameter(Mandatory = $true)]
    [string]$CandidateManifestPath,
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}

$releaseContract = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot 'windows-release-contract.json')) | ConvertFrom-Json
$requiredChecks = @($releaseContract.requiredUatChecks | ForEach-Object { [string]$_.id })

$errors = [System.Collections.Generic.List[string]]::new()
function Add-GateError([string]$Message) {
    $errors.Add($Message)
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

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$evidenceFile = Get-Item -LiteralPath (Resolve-Path -LiteralPath $EvidencePath).Path
$installer = Get-Item -LiteralPath (Resolve-Path -LiteralPath $InstallerPath).Path
$candidateManifestFile = Get-Item -LiteralPath (Resolve-Path -LiteralPath $CandidateManifestPath).Path
$evidence = Get-Content -LiteralPath $evidenceFile.FullName -Raw | ConvertFrom-Json
$candidateManifest = Get-Content -LiteralPath $candidateManifestFile.FullName -Raw | ConvertFrom-Json

if ($evidence.schemaVersion -ne 1) {
    Add-GateError 'Evidence schemaVersion must be 1.'
}
if ($evidence.issue -ne $releaseContract.issue) {
    Add-GateError 'Evidence must belong to Issue #28.'
}
if ($evidence.evidenceOrigin -eq 'synthetic-test') {
    Add-GateError 'Synthetic evidence cannot satisfy the Windows UAT gate.'
} elseif ($evidence.evidenceOrigin -ne 'interactive-windows-uat') {
    Add-GateError 'Evidence origin must be the interactive Windows UAT runner.'
}
try {
    [void][DateTimeOffset]::Parse([string]$evidence.completedAtUtc)
} catch {
    Add-GateError 'Evidence completedAtUtc is invalid.'
}

$head = (& git -C $root rev-parse HEAD | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $evidence.gitCommit -ne $head) {
    Add-GateError 'Evidence gitCommit does not match the current HEAD.'
}
if ($candidateManifest.schemaVersion -ne 1 -or $candidateManifest.issue -ne $releaseContract.issue) {
    Add-GateError 'The candidate manifest schema or issue is invalid.'
}
if ($candidateManifest.gitCommit -ne $head) {
    Add-GateError 'The candidate manifest gitCommit does not match the current HEAD.'
}
if ($candidateManifest.platform -ne 'windows-x64-current-user') {
    Add-GateError 'The candidate manifest platform is invalid.'
}
$candidateVerification = $candidateManifest.verification
if ($candidateVerification.frontendCheck -ne 'passed' -or
    $candidateVerification.frontendTests -ne 'passed' -or
    $candidateVerification.layoutTests -ne 'passed' -or
    $candidateVerification.rustTests -ne 'passed' -or
    $candidateVerification.acceptanceGate -ne 'passed' -or
    $candidateVerification.releaseTree -ne 'passed' -or
    $candidateVerification.releaseContract -ne 'passed') {
    Add-GateError 'The candidate manifest does not record all required build gates as passed.'
}
$candidateManifestSha256 = Get-Sha256File $candidateManifestFile.FullName
if ($evidence.candidateManifestSha256 -ne $candidateManifestSha256) {
    Add-GateError 'Evidence is not bound to this candidate manifest.'
}
if ($evidence.platform.os -ne 'windows' -or
    $evidence.platform.architecture -ne 'x64' -or
    [int]$evidence.platform.build -lt 19045) {
    Add-GateError 'Evidence platform must be Windows x64 build 19045 or newer.'
}
$cliVersionMatch = [regex]::Match([string]$evidence.codexCliVersion, '^codex-cli (\d+\.\d+\.\d+)')
if (-not $cliVersionMatch.Success -or
    [version]$cliVersionMatch.Groups[1].Value -lt [version]'0.147.0') {
    Add-GateError 'Evidence must record a supported Codex CLI version.'
}
if ([string]$evidence.providerCombinationFingerprint -notmatch '^[0-9a-f]{64}$' -or
    [string]$evidence.providerCombinationFingerprint -match '^0{64}$') {
    Add-GateError 'Evidence provider combination fingerprint is invalid.'
}

$seenChecks = @{}
foreach ($check in @($evidence.checks)) {
    $id = [string]$check.id
    if ([string]::IsNullOrWhiteSpace($id)) {
        Add-GateError 'Evidence contains a check without an id.'
        continue
    }
    if ($seenChecks.ContainsKey($id)) {
        Add-GateError "Evidence contains a duplicate check: $id."
        continue
    }
    $seenChecks[$id] = $check.passed -eq $true
}
foreach ($required in $requiredChecks) {
    if (-not $seenChecks.ContainsKey($required) -or -not $seenChecks[$required]) {
        Add-GateError "Required UAT check is missing or failed: $required."
    }
}

$actualHash = Get-Sha256File $installer.FullName
$candidateArtifactName = [System.IO.Path]::GetFileName(([string]$candidateManifest.artifact.path).Replace('/', '\'))
if ($candidateArtifactName -ne $installer.Name -or
    $candidateManifest.artifact.sha256 -ne $actualHash -or
    [int64]$candidateManifest.artifact.size -ne $installer.Length) {
    Add-GateError 'The candidate manifest artifact does not match the installer.'
}
if ($evidence.artifact.fileName -ne $installer.Name) {
    Add-GateError 'Evidence artifact fileName does not match the installer.'
}
if ($evidence.artifact.sha256 -ne $actualHash) {
    Add-GateError 'Evidence artifact SHA-256 does not match the installer.'
}
if ([int64]$evidence.artifact.size -ne $installer.Length) {
    Add-GateError 'Evidence artifact size does not match the installer.'
}

$signature = Get-FileSignature $installer.FullName
$signatureStatus = $signature.Status.ToString()
if ($evidence.artifact.authenticodeStatus -ne $signatureStatus) {
    Add-GateError 'Evidence Authenticode status does not match the installer.'
}
if ($candidateManifest.artifact.authenticodeStatus -ne $signatureStatus) {
    Add-GateError 'The candidate manifest Authenticode status does not match the installer.'
}
if ($signatureStatus -ne 'Valid' -and $signatureStatus -ne 'NotSigned') {
    Add-GateError "Installer Authenticode status is not acceptable: $signatureStatus."
}
if ($Mode -eq 'Release' -and $signatureStatus -ne 'Valid') {
    Add-GateError 'Formal release requires a valid Authenticode signature.'
}

$treeOutput = (& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'test-release-tree.ps1') -RepositoryRoot $root 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    Add-GateError 'Release tree gate failed.'
} else {
    try {
        $treeReport = $treeOutput | ConvertFrom-Json
        if (-not $treeReport.passed) {
            Add-GateError 'Release tree report was not clean.'
        }
    } catch {
        Add-GateError 'Release tree report was invalid.'
    }
}

$contractOutput = (& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'test-release-contract.ps1') -RepositoryRoot $root 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    Add-GateError 'Release contract gate failed.'
} else {
    try {
        $contractReport = $contractOutput | ConvertFrom-Json
        if (-not $contractReport.passed -or @($contractReport.contradictions).Count -ne 0) {
            Add-GateError 'Release contract report contains contradictions.'
        }
    } catch {
        Add-GateError 'Release contract report was invalid.'
    }
}

$report = [ordered]@{
    passed = $errors.Count -eq 0
    mode = $Mode
    issue = [int]$releaseContract.issue
    gitCommit = $head
    artifactSha256 = $actualHash
    authenticodeStatus = $signatureStatus
    errors = @($errors)
}
$report | ConvertTo-Json -Depth 5
if ($errors.Count -ne 0) {
    exit 1
}
