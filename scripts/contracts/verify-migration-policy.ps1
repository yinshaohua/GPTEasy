[CmdletBinding()]
param(
    [switch]$SelfTest,
    [string]$RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
}

try {
    $root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
    $fixturePath = Join-Path $root 'tests\fixtures\migrations\forbidden-migration-cases.json'
    $fixture = [System.IO.File]::ReadAllText($fixturePath) | ConvertFrom-Json
    if ([int]$fixture.format_version -ne 1) {
        throw 'migration policy fixture format_version must be 1'
    }

    foreach ($case in @($fixture.sql_cases)) {
        Test-SqlMigrationPolicy -Source ([string]$case.source) | Out-Null
    }
    foreach ($case in @($fixture.rust_cases)) {
        Test-RustMigrationPolicy -Source ([string]$case.source) | Out-Null
    }

    throw 'migration policy predicates unexpectedly returned without verification'
}
catch {
    [Console]::Error.WriteLine("migration policy verification failed: $($_.Exception.Message)")
    exit 1
}
