[CmdletBinding()]
param(
    [string]$RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Failures = [System.Collections.Generic.List[string]]::new()
$script:Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
}

function Add-Failure {
    param([string]$Message)

    $script:Failures.Add($Message)
}

function Get-Sha256Hex {
    param([byte[]]$Bytes)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash($Bytes)
        return ([System.BitConverter]::ToString($digest)).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Get-FileSha256Hex {
    param([string]$Path)

    return Get-Sha256Hex -Bytes ([System.IO.File]::ReadAllBytes($Path))
}

function Get-BigEndianBytes {
    param(
        [long]$Value,
        [ValidateSet(4, 8)][int]$Width
    )

    if ($Width -eq 4) {
        $bytes = [System.BitConverter]::GetBytes([uint32]$Value)
    }
    else {
        $bytes = [System.BitConverter]::GetBytes([int64]$Value)
    }
    if ([System.BitConverter]::IsLittleEndian) {
        [System.Array]::Reverse($bytes)
    }
    return $bytes
}

function Get-SchemaFingerprint {
    param(
        [long]$ApplicationId,
        [uint32]$Version,
        [string]$Checksum
    )

    $stream = [System.IO.MemoryStream]::new()
    try {
        $domain = $script:Utf8NoBom.GetBytes("gpteasy-schema-fingerprint-v1`0")
        $stream.Write($domain, 0, $domain.Length)
        $applicationBytes = Get-BigEndianBytes -Value $ApplicationId -Width 8
        $stream.Write($applicationBytes, 0, $applicationBytes.Length)
        $versionBytes = Get-BigEndianBytes -Value $Version -Width 4
        $stream.Write($versionBytes, 0, $versionBytes.Length)
        $checksumBytes = $script:Utf8NoBom.GetBytes($Checksum)
        $stream.Write($checksumBytes, 0, $checksumBytes.Length)
        return Get-Sha256Hex -Bytes $stream.ToArray()
    }
    finally {
        $stream.Dispose()
    }
}

function Resolve-RepositoryPath {
    param(
        [string]$Root,
        [string]$RelativePath
    )

    if ([System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "history path must be repository-relative: $RelativePath"
    }
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootFull $RelativePath))
    if (-not ($candidate -eq $rootFull -or $candidate.StartsWith("$rootFull$([System.IO.Path]::DirectorySeparatorChar)", [System.StringComparison]::OrdinalIgnoreCase))) {
        throw "history path escapes repository root: $RelativePath"
    }
    return $candidate
}

function Read-JsonDocument {
    param([string]$Path)

    return [System.IO.File]::ReadAllText($Path, $script:Utf8NoBom) | ConvertFrom-Json
}

function Invoke-GitBytes {
    param(
        [string]$Root,
        [string[]]$Arguments
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = 'git'
    $startInfo.WorkingDirectory = $Root
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $output = [System.IO.MemoryStream]::new()
    try {
        if (-not $process.Start()) {
            throw 'failed to start git'
        }
        $process.StandardOutput.BaseStream.CopyTo($output)
        $errorText = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Bytes = $output.ToArray()
            ErrorText = $errorText
        }
    }
    finally {
        $output.Dispose()
        $process.Dispose()
    }
}

function Get-GitBlobIfPresent {
    param(
        [string]$Root,
        [string]$Revision,
        [string]$Path
    )

    $spec = "${Revision}:$Path"
    $exists = Invoke-GitBytes -Root $Root -Arguments @('cat-file', '-e', $spec)
    if ($exists.ExitCode -ne 0) {
        return $null
    }
    $blob = Invoke-GitBytes -Root $Root -Arguments @('cat-file', 'blob', $spec)
    if ($blob.ExitCode -ne 0) {
        throw "git could not read $spec"
    }
    return ,$blob.Bytes
}

function Assert-LowerSha256 {
    param(
        [string]$Value,
        [string]$Label
    )

    if ($Value -cnotmatch '^[0-9a-f]{64}$') {
        Add-Failure "$Label must be a lowercase SHA-256"
    }
}

function Get-RegistryEntries {
    param(
        [string]$Source,
        [long]$ExpectedApplicationId
    )

    $applicationMatch = [regex]::Match($Source, 'pub\s+const\s+APPLICATION_ID\s*:\s*i64\s*=\s*0x(?<hex>[0-9A-Fa-f_]+)\s*;')
    if (-not $applicationMatch.Success) {
        Add-Failure 'migration registry APPLICATION_ID declaration is missing or ambiguous'
    }
    else {
        $observedApplicationId = [System.Convert]::ToInt64($applicationMatch.Groups['hex'].Value.Replace('_', ''), 16)
        if ($observedApplicationId -ne $ExpectedApplicationId) {
            Add-Failure 'migration registry APPLICATION_ID differs from history lock'
        }
    }

    $registryMatch = [regex]::Match(
        $Source,
        '(?s)pub\s+const\s+MIGRATIONS\s*:\s*&\[Migration\]\s*=\s*&\[(?<body>.*?)\];'
    )
    if (-not $registryMatch.Success) {
        Add-Failure 'migration registry declaration is missing or ambiguous'
        return @()
    }

    $entryPattern = [regex]::new(
        '(?sx)Migration\s*\{\s*' +
        'version\s*:\s*(?<version>\d+)\s*,\s*' +
        'name\s*:\s*"(?<name>[^"]+)"\s*,\s*' +
        'sql\s*:\s*include_str!\("(?<sql>[^"]+)"\)\s*,\s*' +
        'checksum\s*:\s*"(?<checksum>[0-9a-f]{64})"\s*,\s*' +
        'schema_fingerprint\s*:\s*"(?<fingerprint>[0-9a-f]{64})"\s*,?\s*\}'
    )
    $entries = @($entryPattern.Matches($registryMatch.Groups['body'].Value) | ForEach-Object {
        [pscustomobject]@{
            version = [uint32]$_.Groups['version'].Value
            name = $_.Groups['name'].Value
            sql_file = $_.Groups['sql'].Value
            checksum = $_.Groups['checksum'].Value
            schema_fingerprint = $_.Groups['fingerprint'].Value
        }
    })
    $unparsed = $entryPattern.Replace($registryMatch.Groups['body'].Value, '')
    if ($unparsed -notmatch '^\s*$') {
        Add-Failure 'migration registry contains unparsed or unsupported entries'
    }
    return $entries
}

try {
    $root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
    $lockPath = Resolve-RepositoryPath -Root $root -RelativePath 'tests/fixtures/databases/history-lock.json'
    $manifestPath = Resolve-RepositoryPath -Root $root -RelativePath 'tests/fixtures/databases/manifest.json'
    $registryPath = Resolve-RepositoryPath -Root $root -RelativePath 'src-tauri/src/state/migrations/mod.rs'
    foreach ($required in @($lockPath, $manifestPath, $registryPath)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "required migration history input is missing: $required"
        }
    }

    $lock = Read-JsonDocument -Path $lockPath
    $manifest = Read-JsonDocument -Path $manifestPath
    if ([int]$lock.format_version -ne 1) {
        Add-Failure 'history lock format_version must be 1'
    }
    if ([int]$manifest.format_version -ne 1) {
        Add-Failure 'fixture manifest format_version must be 1'
    }
    if (@($lock.migrations).Count -eq 0 -or @($lock.fixtures).Count -eq 0) {
        Add-Failure 'history lock migrations and fixtures must both be non-empty'
    }
    if (@($manifest.fixtures).Count -eq 0) {
        Add-Failure 'fixture manifest must be non-empty'
    }

    $applicationId = [long]$lock.application_id
    $registrySource = [System.IO.File]::ReadAllText($registryPath, $script:Utf8NoBom)
    $registryEntries = @(Get-RegistryEntries -Source $registrySource -ExpectedApplicationId $applicationId)
    if ($registryEntries.Count -ne @($lock.migrations).Count) {
        Add-Failure 'history lock migration count differs from production registry'
    }

    $expectedVersion = 1
    foreach ($migration in @($lock.migrations)) {
        $version = [int]$migration.version
        if ($version -ne $expectedVersion) {
            Add-Failure "history lock migration versions must be continuous from one (found $version, expected $expectedVersion)"
        }
        $expectedVersion++
        Assert-LowerSha256 -Value ([string]$migration.sql_sha256) -Label "migration $version sql_sha256"
        Assert-LowerSha256 -Value ([string]$migration.schema_fingerprint) -Label "migration $version schema_fingerprint"

        $sqlPath = Resolve-RepositoryPath -Root $root -RelativePath ([string]$migration.sql_path)
        if (-not (Test-Path -LiteralPath $sqlPath -PathType Leaf)) {
            Add-Failure "migration $version SQL file is missing"
            continue
        }
        $sqlDigest = Get-FileSha256Hex -Path $sqlPath
        if ($sqlDigest -cne [string]$migration.sql_sha256) {
            Add-Failure "migration $version SQL digest differs from history lock"
        }
        $expectedFingerprint = Get-SchemaFingerprint -ApplicationId $applicationId -Version $version -Checksum $sqlDigest
        if ($expectedFingerprint -cne [string]$migration.schema_fingerprint) {
            Add-Failure "migration $version schema fingerprint differs from its locked identity"
        }

        $registry = @($registryEntries | Where-Object { $_.version -eq $version })
        if ($registry.Count -ne 1) {
            Add-Failure "migration $version is missing or duplicated in production registry"
            continue
        }
        $expectedSqlFile = Split-Path -Leaf ([string]$migration.sql_path)
        if ($registry[0].name -cne [string]$migration.name -or
            $registry[0].sql_file -cne $expectedSqlFile -or
            $registry[0].checksum -cne $sqlDigest -or
            $registry[0].schema_fingerprint -cne [string]$migration.schema_fingerprint) {
            Add-Failure "migration $version production registry identity differs from history lock"
        }
    }

    if (@($lock.fixtures).Count -ne @($manifest.fixtures).Count) {
        Add-Failure 'history lock fixture count differs from manifest'
    }
    foreach ($fixture in @($manifest.fixtures)) {
        $fixtureId = [string]$fixture.fixture_id
        $lockedFixtures = @($lock.fixtures | Where-Object { $_.fixture_id -ceq $fixtureId })
        if ($lockedFixtures.Count -ne 1) {
            Add-Failure "fixture $fixtureId is missing or duplicated in history lock"
            continue
        }
        $locked = $lockedFixtures[0]
        foreach ($field in @('database_path', 'file_sha256', 'logical_digest_sha256', 'application_id', 'user_version', 'schema_fingerprint')) {
            if ([string]$locked.$field -cne [string]$fixture.$field) {
                Add-Failure "fixture $fixtureId field $field differs between manifest and history lock"
            }
        }
        Assert-LowerSha256 -Value ([string]$locked.file_sha256) -Label "fixture $fixtureId file_sha256"
        Assert-LowerSha256 -Value ([string]$locked.logical_digest_sha256) -Label "fixture $fixtureId logical_digest_sha256"
        $databasePath = Resolve-RepositoryPath -Root $root -RelativePath ("tests/fixtures/databases/" + [string]$locked.database_path)
        if (-not (Test-Path -LiteralPath $databasePath -PathType Leaf)) {
            Add-Failure "fixture $fixtureId database file is missing"
        }
        elseif ((Get-FileSha256Hex -Path $databasePath) -cne [string]$locked.file_sha256) {
            Add-Failure "fixture $fixtureId database digest differs from history lock"
        }
        $migration = @($lock.migrations | Where-Object { [int]$_.version -eq [int]$locked.user_version })
        if ($migration.Count -ne 1 -or
            [long]$locked.application_id -ne $applicationId -or
            [string]$locked.schema_fingerprint -cne [string]$migration[0].schema_fingerprint) {
            Add-Failure "fixture $fixtureId identity does not resolve to exactly one locked migration"
        }
    }

    $tagResult = Invoke-GitBytes -Root $root -Arguments @('tag', '--merged', 'HEAD', '--list', 'v*')
    if ($tagResult.ExitCode -ne 0) {
        throw 'git could not enumerate merged release tags'
    }
    $tags = @($script:Utf8NoBom.GetString($tagResult.Bytes).Split("`n", [System.StringSplitOptions]::RemoveEmptyEntries) | ForEach-Object { $_.Trim() })
    foreach ($tag in $tags) {
        foreach ($migration in @($lock.migrations)) {
            $tagSql = Get-GitBlobIfPresent -Root $root -Revision $tag -Path ([string]$migration.sql_path)
            if ($null -ne $tagSql -and (Get-Sha256Hex -Bytes $tagSql) -cne [string]$migration.sql_sha256) {
                Add-Failure "release tag $tag rewrites migration $($migration.version) SQL"
            }
        }
        foreach ($fixture in @($lock.fixtures)) {
            $tagFixturePath = "tests/fixtures/databases/$([string]$fixture.database_path)"
            $tagDatabase = Get-GitBlobIfPresent -Root $root -Revision $tag -Path $tagFixturePath
            if ($null -ne $tagDatabase -and (Get-Sha256Hex -Bytes $tagDatabase) -cne [string]$fixture.file_sha256) {
                Add-Failure "release tag $tag rewrites fixture $([string]$fixture.fixture_id) bytes"
            }
        }
        $tagManifestBytes = Get-GitBlobIfPresent -Root $root -Revision $tag -Path 'tests/fixtures/databases/manifest.json'
        if ($null -ne $tagManifestBytes) {
            $tagManifest = $script:Utf8NoBom.GetString($tagManifestBytes) | ConvertFrom-Json
            foreach ($tagFixture in @($tagManifest.fixtures)) {
                $locked = @($lock.fixtures | Where-Object { $_.fixture_id -ceq [string]$tagFixture.fixture_id })
                if ($locked.Count -eq 1 -and [int]$locked[0].user_version -eq [int]$tagFixture.user_version) {
                    foreach ($field in @('file_sha256', 'logical_digest_sha256', 'application_id', 'user_version', 'schema_fingerprint')) {
                        if ([string]$locked[0].$field -cne [string]$tagFixture.$field) {
                            Add-Failure "release tag $tag rewrites fixture $([string]$tagFixture.fixture_id) field $field"
                        }
                    }
                }
            }
        }
    }

    if ($script:Failures.Count -gt 0) {
        foreach ($failure in $script:Failures) {
            [Console]::Error.WriteLine("migration history verification failed: $failure")
        }
        exit 1
    }

    [pscustomobject]@{
        outcome = 'passed'
        migration_count = @($lock.migrations).Count
        fixture_count = @($lock.fixtures).Count
        merged_release_tag_count = $tags.Count
    } | ConvertTo-Json -Compress
}
catch {
    [Console]::Error.WriteLine("migration history verification failed: $($_.Exception.Message)")
    exit 1
}
