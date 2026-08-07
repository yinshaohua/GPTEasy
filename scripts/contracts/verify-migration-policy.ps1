[CmdletBinding()]
param(
    [switch]$SelfTest,
    [string]$RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
}

function Resolve-RepositoryPath {
    param(
        [string]$Root,
        [string]$RelativePath
    )

    if ([System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "migration policy path must be repository-relative: $RelativePath"
    }
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootFull $RelativePath))
    if (-not ($candidate -eq $rootFull -or $candidate.StartsWith("$rootFull$([System.IO.Path]::DirectorySeparatorChar)", [System.StringComparison]::OrdinalIgnoreCase))) {
        throw "migration policy path escapes repository root: $RelativePath"
    }
    return $candidate
}

function Add-SqlToken {
    param(
        [System.Collections.Generic.List[string]]$Tokens,
        [string]$Value
    )

    if (-not [string]::IsNullOrWhiteSpace($Value)) {
        $Tokens.Add($Value.ToLowerInvariant())
    }
}

function Get-SqlPolicyViolations {
    param([string]$Source)

    $tokens = [System.Collections.Generic.List[string]]::new()
    $violations = [System.Collections.Generic.List[string]]::new()
    $index = 0
    $state = 'code'
    $quoted = [System.Text.StringBuilder]::new()

    while ($index -lt $Source.Length) {
        $character = $Source[$index]
        $next = if ($index + 1 -lt $Source.Length) { $Source[$index + 1] } else { [char]0 }

        switch ($state) {
            'code' {
                if ($character -eq "'") {
                    $state = 'single-string'
                    $index++
                    continue
                }
                if ($character -eq '"') {
                    $quoted.Clear() | Out-Null
                    $state = 'double-identifier'
                    $index++
                    continue
                }
                if ($character -eq '[') {
                    $quoted.Clear() | Out-Null
                    $state = 'bracket-identifier'
                    $index++
                    continue
                }
                if ($character -eq '`') {
                    $quoted.Clear() | Out-Null
                    $state = 'backtick-identifier'
                    $index++
                    continue
                }
                if ($character -eq '-' -and $next -eq '-') {
                    $state = 'line-comment'
                    $index += 2
                    continue
                }
                if ($character -eq '/' -and $next -eq '*') {
                    $state = 'block-comment'
                    $index += 2
                    continue
                }
                if ([char]::IsLetter($character) -or $character -eq '_') {
                    $start = $index
                    $index++
                    while ($index -lt $Source.Length) {
                        $candidate = $Source[$index]
                        if (-not ([char]::IsLetterOrDigit($candidate) -or $candidate -eq '_' -or $candidate -eq '$')) {
                            break
                        }
                        $index++
                    }
                    Add-SqlToken -Tokens $tokens -Value $Source.Substring($start, $index - $start)
                    continue
                }
                if ($character -eq '.') {
                    $tokens.Add('.')
                }
                $index++
            }
            'single-string' {
                if ($character -eq "'" -and $next -eq "'") {
                    $index += 2
                    continue
                }
                if ($character -eq "'") {
                    $state = 'code'
                }
                $index++
            }
            'double-identifier' {
                if ($character -eq '"' -and $next -eq '"') {
                    $quoted.Append('"') | Out-Null
                    $index += 2
                    continue
                }
                if ($character -eq '"') {
                    Add-SqlToken -Tokens $tokens -Value $quoted.ToString()
                    $state = 'code'
                    $index++
                    continue
                }
                $quoted.Append($character) | Out-Null
                $index++
            }
            'bracket-identifier' {
                if ($character -eq ']' -and $next -eq ']') {
                    $quoted.Append(']') | Out-Null
                    $index += 2
                    continue
                }
                if ($character -eq ']') {
                    Add-SqlToken -Tokens $tokens -Value $quoted.ToString()
                    $state = 'code'
                    $index++
                    continue
                }
                $quoted.Append($character) | Out-Null
                $index++
            }
            'backtick-identifier' {
                if ($character -eq '`' -and $next -eq '`') {
                    $quoted.Append('`') | Out-Null
                    $index += 2
                    continue
                }
                if ($character -eq '`') {
                    Add-SqlToken -Tokens $tokens -Value $quoted.ToString()
                    $state = 'code'
                    $index++
                    continue
                }
                $quoted.Append($character) | Out-Null
                $index++
            }
            'line-comment' {
                if ($character -eq "`n") {
                    $state = 'code'
                }
                $index++
            }
            'block-comment' {
                if ($character -eq '*' -and $next -eq '/') {
                    $state = 'code'
                    $index += 2
                    continue
                }
                $index++
            }
        }
    }

    if ($state -notin @('code', 'line-comment')) {
        $violations.Add("unterminated SQL lexical state: $state")
    }

    for ($tokenIndex = 0; $tokenIndex -lt $tokens.Count; $tokenIndex++) {
        $token = $tokens[$tokenIndex]
        if ($token -in @('vacuum', 'attach', 'detach')) {
            $violations.Add("prohibited SQL token: $token")
            continue
        }
        if ($token -eq 'pragma') {
            $nameIndex = $tokenIndex + 1
            if ($nameIndex + 2 -lt $tokens.Count -and $tokens[$nameIndex + 1] -eq '.') {
                $nameIndex += 2
            }
            if ($nameIndex -lt $tokens.Count -and $tokens[$nameIndex] -eq 'journal_mode') {
                $violations.Add('prohibited SQL pragma: journal_mode')
            }
        }
    }

    return @($violations | Select-Object -Unique)
}

function Get-MaskedRustSource {
    param([string]$Source)

    $builder = [System.Text.StringBuilder]::new($Source.Length)
    $index = 0
    $state = 'code'
    $blockDepth = 0
    $escaped = $false
    while ($index -lt $Source.Length) {
        $character = $Source[$index]
        $next = if ($index + 1 -lt $Source.Length) { $Source[$index + 1] } else { [char]0 }

        if ($state -eq 'code') {
            if ($character -eq '/' -and $next -eq '/') {
                $builder.Append('  ') | Out-Null
                $state = 'line-comment'
                $index += 2
                continue
            }
            if ($character -eq '/' -and $next -eq '*') {
                $builder.Append('  ') | Out-Null
                $state = 'block-comment'
                $blockDepth = 1
                $index += 2
                continue
            }

            $rawMatch = [regex]::Match($Source.Substring($index), '^(?:b)?r(?<hashes>#{0,32})"')
            if ($rawMatch.Success) {
                $closing = '"' + $rawMatch.Groups['hashes'].Value
                $closingIndex = $Source.IndexOf($closing, $index + $rawMatch.Length, [System.StringComparison]::Ordinal)
                if ($closingIndex -lt 0) {
                    while ($index -lt $Source.Length) {
                        if ($Source[$index] -in @("`r", "`n")) { $builder.Append($Source[$index]) | Out-Null } else { $builder.Append(' ') | Out-Null }
                        $index++
                    }
                    $state = 'unterminated-raw-string'
                    continue
                }
                $rawEnd = $closingIndex + $closing.Length
                while ($index -lt $rawEnd) {
                    if ($Source[$index] -in @("`r", "`n")) { $builder.Append($Source[$index]) | Out-Null } else { $builder.Append(' ') | Out-Null }
                    $index++
                }
                continue
            }

            if ($character -eq '"') {
                $builder.Append(' ') | Out-Null
                $state = 'string'
                $escaped = $false
                $index++
                continue
            }
            $builder.Append($character) | Out-Null
            $index++
            continue
        }

        if ($state -eq 'line-comment') {
            if ($character -eq "`n") {
                $builder.Append("`n") | Out-Null
                $state = 'code'
            }
            else {
                $builder.Append(' ') | Out-Null
            }
            $index++
            continue
        }

        if ($state -eq 'block-comment') {
            if ($character -eq '/' -and $next -eq '*') {
                $builder.Append('  ') | Out-Null
                $blockDepth++
                $index += 2
                continue
            }
            if ($character -eq '*' -and $next -eq '/') {
                $builder.Append('  ') | Out-Null
                $blockDepth--
                $index += 2
                if ($blockDepth -eq 0) {
                    $state = 'code'
                }
                continue
            }
            if ($character -in @("`r", "`n")) { $builder.Append($character) | Out-Null } else { $builder.Append(' ') | Out-Null }
            $index++
            continue
        }

        if ($state -eq 'string') {
            if ($character -in @("`r", "`n")) { $builder.Append($character) | Out-Null } else { $builder.Append(' ') | Out-Null }
            if ($escaped) {
                $escaped = $false
            }
            elseif ($character -eq '\') {
                $escaped = $true
            }
            elseif ($character -eq '"') {
                $state = 'code'
            }
            $index++
        }
    }

    return [pscustomobject]@{
        Source = $builder.ToString()
        FinalState = $state
    }
}

function Get-RustPolicyViolations {
    param([string]$Source)

    $violations = [System.Collections.Generic.List[string]]::new()
    $masked = Get-MaskedRustSource -Source $Source
    if ($masked.FinalState -notin @('code', 'line-comment')) {
        $violations.Add("unterminated Rust lexical state: $($masked.FinalState)")
    }

    $capabilities = @(
        @{ label = 'filesystem'; pattern = '(?i)(?:\bstd\s*::\s*)?\bfs\s*::|\bFile\s*::|\bOpenOptions\s*::' },
        @{ label = 'path'; pattern = '(?i)\bstd\s*::\s*path\b|\bPath(?:Buf)?\b' },
        @{ label = 'subprocess'; pattern = '(?i)\bstd\s*::\s*process\b|\bCommand\s*::' },
        @{ label = 'network'; pattern = '(?i)\bstd\s*::\s*net\b|\btokio\s*::\s*net\b|\b(?:TcpStream|TcpListener|UdpSocket|reqwest|hyper|ureq|curl)\b' },
        @{ label = 'connection'; pattern = '(?i)\b(?:rusqlite\s*::\s*)?Connection\b' }
    )
    foreach ($capability in $capabilities) {
        if ($masked.Source -match $capability.pattern) {
            $violations.Add("prohibited Rust migration capability: $($capability.label)")
        }
    }

    $transformPattern = [regex]::new('(?is)\bfn\s+(?<name>[A-Za-z_][A-Za-z0-9_]*transform[A-Za-z0-9_]*)\s*\((?<parameters>[^)]*)\)')
    $transactionParameter = '^\s*[A-Za-z_][A-Za-z0-9_]*\s*:\s*&\s*(?:mut\s+)?(?:rusqlite\s*::\s*)?Transaction\s*(?:<\s*''[A-Za-z_][A-Za-z0-9_]*\s*>)?\s*$'
    foreach ($transform in $transformPattern.Matches($masked.Source)) {
        if ($transform.Groups['parameters'].Value -cnotmatch $transactionParameter) {
            $violations.Add("Rust transform $($transform.Groups['name'].Value) must accept only &Transaction")
        }
    }

    return @($violations | Select-Object -Unique)
}

function Get-MigrationFileViolations {
    param(
        [string]$Path,
        [ValidateSet('sql', 'rust')][string]$Kind
    )

    $source = [System.IO.File]::ReadAllText($Path, $script:Utf8NoBom)
    if ($Kind -eq 'sql') {
        return @(Get-SqlPolicyViolations -Source $source)
    }
    return @(Get-RustPolicyViolations -Source $source)
}

function Invoke-PolicySelfTest {
    param(
        [string]$Root,
        [string]$FixturePath
    )

    $fixture = [System.IO.File]::ReadAllText($FixturePath, $script:Utf8NoBom) | ConvertFrom-Json
    if ([int]$fixture.format_version -ne 1) {
        throw 'migration policy fixture format_version must be 1'
    }
    if (@($fixture.sql_cases).Count -eq 0 -or @($fixture.rust_cases).Count -eq 0) {
        throw 'migration policy fixture must contain SQL and Rust cases'
    }

    $failures = [System.Collections.Generic.List[string]]::new()
    $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('gpteasy-migration-policy-' + [guid]::NewGuid().ToString('N'))
    [System.IO.Directory]::CreateDirectory($tempRoot) | Out-Null
    try {
        foreach ($kind in @('sql', 'rust')) {
            $cases = if ($kind -eq 'sql') { @($fixture.sql_cases) } else { @($fixture.rust_cases) }
            $extension = if ($kind -eq 'sql') { '.sql' } else { '.rs' }
            foreach ($case in $cases) {
                $casePath = Join-Path $tempRoot (([string]$case.id) + $extension)
                [System.IO.File]::WriteAllText($casePath, [string]$case.source, $script:Utf8NoBom)
                $violations = @(Get-MigrationFileViolations -Path $casePath -Kind $kind)
                if ([bool]$case.should_pass -and $violations.Count -ne 0) {
                    $failures.Add("$kind case $([string]$case.id) was rejected: $($violations -join ', ')")
                }
                elseif (-not [bool]$case.should_pass -and $violations.Count -eq 0) {
                    $failures.Add("$kind case $([string]$case.id) accepted prohibited behavior")
                }
            }
        }
    }
    finally {
        if ([System.IO.Directory]::Exists($tempRoot)) {
            [System.IO.Directory]::Delete($tempRoot, $true)
        }
    }

    if ($failures.Count -gt 0) {
        throw ($failures -join '; ')
    }
    return [pscustomobject]@{
        outcome = 'passed'
        sql_case_count = @($fixture.sql_cases).Count
        rust_case_count = @($fixture.rust_cases).Count
        test_only = $true
        strict_gate_eligible = $false
    }
}

function Invoke-RepositoryPolicyScan {
    param([string]$Root)

    $migrationRoot = Resolve-RepositoryPath -Root $Root -RelativePath 'src-tauri/src/state/migrations'
    if (-not (Test-Path -LiteralPath $migrationRoot -PathType Container)) {
        throw 'production migration directory is missing'
    }
    $sqlFiles = @(Get-ChildItem -LiteralPath $migrationRoot -Recurse -File -Filter '*.sql' | Sort-Object FullName)
    $rustFiles = @(Get-ChildItem -LiteralPath $migrationRoot -Recurse -File -Filter '*.rs' | Sort-Object FullName)
    if ($sqlFiles.Count -eq 0 -or $rustFiles.Count -eq 0) {
        throw 'production migration scan requires both SQL and Rust sources'
    }

    $failures = [System.Collections.Generic.List[string]]::new()
    foreach ($file in $sqlFiles) {
        foreach ($violation in @(Get-MigrationFileViolations -Path $file.FullName -Kind sql)) {
            $relative = $file.FullName.Substring($Root.TrimEnd('\', '/').Length + 1).Replace('\', '/')
            $failures.Add("${relative}: $violation")
        }
    }
    foreach ($file in $rustFiles) {
        foreach ($violation in @(Get-MigrationFileViolations -Path $file.FullName -Kind rust)) {
            $relative = $file.FullName.Substring($Root.TrimEnd('\', '/').Length + 1).Replace('\', '/')
            $failures.Add("${relative}: $violation")
        }
    }
    if ($failures.Count -gt 0) {
        throw ($failures -join '; ')
    }

    return [pscustomobject]@{
        outcome = 'passed'
        sql_file_count = $sqlFiles.Count
        rust_file_count = $rustFiles.Count
        test_only = $false
        strict_gate_eligible = $true
    }
}

try {
    $root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
    $fixturePath = Resolve-RepositoryPath -Root $root -RelativePath 'tests/fixtures/migrations/forbidden-migration-cases.json'
    if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) {
        throw 'migration policy fixture is missing'
    }

    $result = if ($SelfTest) {
        Invoke-PolicySelfTest -Root $root -FixturePath $fixturePath
    }
    else {
        Invoke-RepositoryPolicyScan -Root $root
    }
    $result | ConvertTo-Json -Compress
}
catch {
    [Console]::Error.WriteLine("migration policy verification failed: $($_.Exception.Message)")
    exit 1
}
