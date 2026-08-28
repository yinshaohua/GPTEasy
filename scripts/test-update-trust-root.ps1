[CmdletBinding()]
param(
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}
$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$errors = [System.Collections.Generic.List[string]]::new()
function Add-TrustError([string]$Message) { $errors.Add($Message) }

function Test-UpdaterPrivateKeyText([string]$Value) {
    $header = @([regex]::Split($Value, '\r?\n'))[0]
    return $header.StartsWith('untrusted comment: ', [StringComparison]::Ordinal) -and
        $header -match '\b(encrypted )?secret key\b' -and
        $header -notmatch '\bsignature from\b'
}

try {
    $distribution = Get-Content -LiteralPath (Join-Path $root 'scripts/gitee-distribution.json') -Raw | ConvertFrom-Json
} catch {
    $distribution = $null
    Add-TrustError 'Gitee distribution contract is missing or invalid.'
}
if ($null -ne $distribution) {
    $apiUri = $null
    $rawUri = $null
    if ($distribution.schemaVersion -ne 1 -or $distribution.issue -ne 55 -or
        -not [Uri]::TryCreate([string]$distribution.apiBaseUrl, [UriKind]::Absolute, [ref]$apiUri) -or $apiUri.Scheme -cne 'https' -or
        -not [Uri]::TryCreate([string]$distribution.rawBaseUrl, [UriKind]::Absolute, [ref]$rawUri) -or $rawUri.Scheme -cne 'https' -or
        [string]$distribution.formalManifestPath -notmatch '^[^/\s]+\.md$' -or
        -not ([string]$distribution.smokeManifestPrefix).StartsWith('smoke/', [StringComparison]::Ordinal) -or
        $distribution.platform -cne 'windows-x86_64' -or
        $distribution.repositoryVariable -cne 'GITEE_REPOSITORY' -or
        $distribution.branchVariable -cne 'GITEE_DEFAULT_BRANCH' -or
        $distribution.tokenSecret -cne 'GITEE_TOKEN') {
        Add-TrustError 'Gitee distribution contract identity or public protocol settings are invalid.'
    }
}

try {
    $config = Get-Content -LiteralPath (Join-Path $root 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
} catch {
    $config = $null
    Add-TrustError 'Tauri configuration is missing or invalid.'
}

$endpoint = ''
$publicKey = ''
$expectedEndpoint = 'https://gitee.com/ericshaohua/gpteasy-releases/raw/main/latest.md'
if ($null -ne $config) {
    if ($config.bundle.createUpdaterArtifacts -ne $true) {
        Add-TrustError 'Tauri updater artifact creation must be enabled.'
    }
    $endpoints = @($config.plugins.updater.endpoints)
    if ($endpoints.Count -ne 1) {
        Add-TrustError 'The application must contain exactly one updater endpoint.'
    } else {
        $endpoint = [string]$endpoints[0]
        if ($endpoint -cne $expectedEndpoint) {
            Add-TrustError 'The updater endpoint must be the canonical Gitee Raw manifest URL.'
        }
    }
    $publicKey = [string]$config.plugins.updater.pubkey
    try {
        $decodedKey = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($publicKey))
        $keyLines = @($decodedKey -split "`n" | ForEach-Object { $_.TrimEnd("`r") })
        if ($keyLines.Count -lt 2 -or
            -not $keyLines[0].StartsWith('untrusted comment: ', [StringComparison]::Ordinal) -or
            $keyLines[0] -notmatch '\bminisign public key\b' -or
            $keyLines[1] -notmatch '^RW[A-Za-z0-9+/]{54}$') {
            Add-TrustError 'The updater public key is not a complete Tauri minisign public key.'
        }
    } catch {
        Add-TrustError 'The updater public key is not valid base64.'
    }
}

$workflowPath = Join-Path $root '.github/workflows/gitee-smoke.yml'
$syncWorkflowPath = Join-Path $root '.github/workflows/gitee-sync.yml'
$smokePath = Join-Path $root 'scripts/smoke-gitee-release.sh'
$syncPath = Join-Path $root 'scripts/sync-gitee-release.mjs'
$wizardPath = Join-Path $root 'scripts/setup-gitee-distribution.sh'
try { $workflow = [System.IO.File]::ReadAllText($workflowPath) } catch { $workflow = ''; Add-TrustError 'Gitee smoke workflow is missing.' }
try { $syncWorkflow = [System.IO.File]::ReadAllText($syncWorkflowPath) } catch { $syncWorkflow = ''; Add-TrustError 'Gitee sync workflow is missing.' }
try { $smoke = [System.IO.File]::ReadAllText($smokePath) } catch { $smoke = ''; Add-TrustError 'Gitee smoke command is missing.' }
try { $sync = [System.IO.File]::ReadAllText($syncPath) } catch { $sync = ''; Add-TrustError 'Gitee sync command is missing.' }
try { $wizard = [System.IO.File]::ReadAllText($wizardPath) } catch { $wizard = ''; Add-TrustError 'Gitee setup wizard is missing.' }

if (-not $workflow.Contains('GITEE_TOKEN: ${{ secrets.GITEE_TOKEN }}')) {
    Add-TrustError 'Gitee Token must come from the GITEE_TOKEN Actions secret.'
}
if (-not $workflow.Contains('GITEE_REPOSITORY: ${{ vars.GITEE_REPOSITORY }}')) {
    Add-TrustError 'Gitee repository must come from a public Actions variable.'
}
if (-not $syncWorkflow.Contains('types: [published]') -or
    -not $syncWorkflow.Contains('GITEE_TOKEN: ${{ secrets.GITEE_TOKEN }}') -or
    -not $syncWorkflow.Contains('GITEE_REPOSITORY: ${{ vars.GITEE_REPOSITORY }}') -or
    $syncWorkflow -match '(?i)(npm run (build|tauri)|cargo build|tauri build)') {
    Add-TrustError 'Gitee formal sync must consume published Release assets without rebuilding.'
}
if (-not $sync.Contains('Authorization: `Bearer ${configuration.giteeToken}`') -or
    $sync -match '(?i)(access_token=|PRIVATE-TOKEN)') {
    Add-TrustError 'Gitee formal sync authentication must use the Bearer header only.'
}
if (-not $smoke.Contains('Authorization: Bearer $GITEE_TOKEN') -or
    $smoke -match '(?i)(access_token=|PRIVATE-TOKEN)') {
    Add-TrustError 'Gitee API authentication must use the Bearer header only.'
}
if (-not $smoke.Contains('gitee-distribution.json')) {
    Add-TrustError 'Gitee smoke must consume the public distribution contract.'
}
if (-not $sync.Contains('new FormData') -or -not $sync.Contains('new URLSearchParams') -or -not $sync.Contains('numericReleaseId')) {
    Add-TrustError 'Gitee sync must use numeric Release IDs, multipart attachments and form-data content writes.'
}
if (-not $smoke.Contains('--range 0-0') -or -not $smoke.Contains('prerelease=true')) {
    Add-TrustError 'Gitee smoke must use a prerelease and anonymous Range download.'
}
if (-not $smoke.Contains('target_commitish=$GITEE_DEFAULT_BRANCH') -or
    -not $sync.Contains('target_commitish: config.giteeBranch')) {
    Add-TrustError 'Gitee Release creation must target the configured distribution branch.'
}
if (-not $wizard.Contains('ask_secret GITEE_TOKEN') -or
    -not $wizard.Contains('set_secret "$TOKEN_SECRET_NAME"') -or
    -not $wizard.Contains('unset GITEE_TOKEN')) {
    Add-TrustError 'The setup wizard must capture, store, and clear the Gitee Token safely.'
}

$trackedOutput = & git -C $root ls-files -z
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to enumerate tracked files for the update trust root gate.'
}
$trackedFiles = @(
    ($trackedOutput -join "`n") -split "`0" |
        Where-Object { $_ } |
        ForEach-Object { $_.Replace('\', '/') }
)
$privateKeyFiles = @($trackedFiles | Where-Object {
    $_ -match '(?i)(^|/)[^/]*(updater|signing)[^/]*\.(key|pem|p12|pfx)$'
})
foreach ($privateFile in $privateKeyFiles) {
    Add-TrustError "Tracked updater private key file is forbidden: $privateFile."
}

foreach ($relativePath in $trackedFiles) {
    $path = Join-Path $root $relativePath
    try {
        $content = [System.IO.File]::ReadAllText($path)
    } catch {
        continue
    }
    $containsPrivateKey = Test-UpdaterPrivateKeyText $content
    if (-not $containsPrivateKey) {
        try {
            $decodedContent = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($content.Trim()))
            $containsPrivateKey = Test-UpdaterPrivateKeyText $decodedContent
        } catch {}
    }
    if ($containsPrivateKey) {
        Add-TrustError "Tracked file contains an updater private key marker: $relativePath."
    }
    if ($content -match '(?i)TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)?\s*=\s*[A-Za-z0-9+/]{12,}') {
        Add-TrustError "Tracked file contains an inline updater signing secret: $relativePath."
    }
}

$report = [ordered]@{
    passed = $errors.Count -eq 0
    endpoint = $endpoint
    publicKeyConfigured = -not [string]::IsNullOrWhiteSpace($publicKey)
    trackedFileCount = $trackedFiles.Count
    errors = @($errors)
}
$report | ConvertTo-Json -Depth 5
if (-not $report.passed) { exit 1 }
