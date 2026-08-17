[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ManifestPath
)

$ErrorActionPreference = 'Stop'
$errors = [System.Collections.Generic.List[string]]::new()
function Add-ManifestError([string]$Message) { $errors.Add($Message) }

try {
    $manifest = Get-Content -LiteralPath (Resolve-Path -LiteralPath $ManifestPath).Path -Raw | ConvertFrom-Json
} catch {
    [ordered]@{ passed = $false; errors = @('Manifest is not valid JSON.') } | ConvertTo-Json -Depth 5
    exit 1
}

$version = [string]$manifest.version
if ($version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
    Add-ManifestError 'Manifest version must be stable SemVer without a prerelease or build suffix.'
}
if ($null -eq $manifest.platforms) {
    Add-ManifestError 'Manifest must contain a platforms object.'
} else {
    $platforms = @($manifest.platforms.PSObject.Properties)
    if ($platforms.Count -ne 1 -or $platforms[0].Name -cne 'windows-x86_64') {
        Add-ManifestError 'Manifest must contain exactly the windows-x86_64 platform entry.'
    }
    foreach ($platform in $platforms) {
        $entry = $platform.Value
        $url = [string]$entry.url
        $signature = [string]$entry.signature
        $parsedUrl = $null
        if (-not [Uri]::TryCreate($url, [UriKind]::Absolute, [ref]$parsedUrl) -or
            $parsedUrl.Scheme -cne 'https') {
            Add-ManifestError "$($platform.Name) URL must be an absolute HTTPS URL."
        }
        if ([string]::IsNullOrWhiteSpace($signature)) {
            Add-ManifestError "$($platform.Name) signature must contain the .sig body."
        } else {
            try {
                $decoded = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($signature))
                if (-not $decoded.StartsWith('untrusted comment: ', [StringComparison]::Ordinal) -or
                    @($decoded -split "`n").Count -lt 4) {
                    Add-ManifestError "$($platform.Name) signature is not a complete Tauri updater signature."
                }
            } catch {
                Add-ManifestError "$($platform.Name) signature is not valid base64."
            }
        }
    }
}

$report = [ordered]@{
    passed = $errors.Count -eq 0
    version = $version
    platforms = if ($null -eq $manifest.platforms) { @() } else { @($manifest.platforms.PSObject.Properties.Name) }
    errors = @($errors)
}
$report | ConvertTo-Json -Depth 5
if (-not $report.passed) { exit 1 }
