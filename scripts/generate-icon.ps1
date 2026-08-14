[CmdletBinding()]
param(
    [string]$SourcePath,
    [string]$OutputDirectory,
    [string]$PublicDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($SourcePath)) {
    $SourcePath = Join-Path $PSScriptRoot "..\src-tauri\icons\icon.svg"
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot "..\src-tauri\icons"
}
if ([string]::IsNullOrWhiteSpace($PublicDirectory)) {
    $PublicDirectory = Join-Path $PSScriptRoot "..\public"
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$source = (Resolve-Path -LiteralPath $SourcePath).Path
$output = [System.IO.Path]::GetFullPath($OutputDirectory)
$public = [System.IO.Path]::GetFullPath($PublicDirectory)
$temporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$temporaryOutput = Join-Path $temporaryRoot ("gpteasy-icons-" + [guid]::NewGuid().ToString("N"))
$desktopFiles = @(
    "32x32.png",
    "128x128.png",
    "128x128@2x.png",
    "icon.icns",
    "icon.ico",
    "icon.png"
)

New-Item -ItemType Directory -Force -Path $output, $public, $temporaryOutput | Out-Null

try {
    Push-Location $repositoryRoot
    try {
        & npm run tauri -- icon $source --output $temporaryOutput
        if ($LASTEXITCODE -ne 0) {
            throw "Tauri icon generation failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }

    foreach ($fileName in $desktopFiles) {
        $generated = Join-Path $temporaryOutput $fileName
        if (-not (Test-Path -LiteralPath $generated -PathType Leaf)) {
            throw "Tauri did not generate the expected icon: $fileName"
        }
        Copy-Item -LiteralPath $generated -Destination (Join-Path $output $fileName) -Force
    }

    Copy-Item -LiteralPath (Join-Path $temporaryOutput "icon.png") `
        -Destination (Join-Path $public "icon.png") -Force
}
finally {
    if (Test-Path -LiteralPath $temporaryOutput) {
        $resolvedTemporaryOutput = (Resolve-Path -LiteralPath $temporaryOutput).Path
        if (-not $resolvedTemporaryOutput.StartsWith(
            $temporaryRoot,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            throw "Refusing to remove a temporary icon directory outside the system temp path."
        }
        Remove-Item -LiteralPath $resolvedTemporaryOutput -Recurse -Force
    }
}

Write-Output "Generated desktop icons in $output and refreshed public/icon.png"
