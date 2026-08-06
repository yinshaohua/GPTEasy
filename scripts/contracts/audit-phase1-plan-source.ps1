[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PhaseDir,

    [switch]$ReadOnly,

    [switch]$UpdateLock,

    [string]$LockPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Convert-ToForwardSlash {
    param([Parameter(Mandatory = $true)][string]$Path)

    return $Path.Replace('\', '/')
}

function Get-NormalizedFullPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [string]$BasePath = (Get-Location).Path
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $BasePath $Path))
}

function Get-RepositoryRoot {
    param([Parameter(Mandatory = $true)][string]$ResolvedPhaseDir)

    $phaseParent = Split-Path -Parent $ResolvedPhaseDir
    $planningDir = Split-Path -Parent $phaseParent
    if ((Split-Path -Leaf $planningDir) -ne '.planning') {
        throw "PhaseDir 必须位于 <repo>/.planning/phases/ 下：$ResolvedPhaseDir"
    }

    return Split-Path -Parent $planningDir
}

function Get-RelativePathPortable {
    param(
        [Parameter(Mandatory = $true)][string]$BasePath,
        [Parameter(Mandatory = $true)][string]$TargetPath
    )

    $baseFull = (Get-NormalizedFullPath -Path $BasePath).TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    $targetFull = Get-NormalizedFullPath -Path $TargetPath
    $baseUri = New-Object System.Uri($baseFull)
    $targetUri = New-Object System.Uri($targetFull)
    return Convert-ToForwardSlash ([System.Uri]::UnescapeDataString($baseUri.MakeRelativeUri($targetUri).ToString()))
}

function Get-CanonicalDigestContent {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )

    $content = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8) -replace "\r\n?", "`n"
    if ($RelativePath -eq '.planning/ROADMAP.md') {
        $content = [regex]::Replace(
            $content,
            '(?m)^-\s+\[[ xX]\]\s+(\*\*Phase 1:[^\r\n]+)$',
            '- [ ] $1'
        )
        $content = [regex]::Replace(
            $content,
            '(?m)^(\*\*Plans\*\*:\s+)\d+/28(\s+plans executed)$',
            '${1}<executed>/28${2}'
        )
        $content = [regex]::Replace(
            $content,
            '(?m)^-\s+\[[ xX]\]\s+(01-\d{2}-PLAN\.md\b.*)$',
            '- [ ] $1'
        )
        $content = [regex]::Replace(
            $content,
            '(?m)^(\|\s*1\.\s+可信本地状态与实现契约\s+\|)\s*\d+/28\s*\|\s*(?:In Progress|Complete)\s*\|[^\r\n]*$',
            '${1} <executed>/28 | <status> | <date> |'
        )
    }
    elseif ($RelativePath -eq '.planning/REQUIREMENTS.md') {
        $content = [regex]::Replace(
            $content,
            '(?m)^-\s+\[[ xX]\]\s+(\*\*STATE-0[1-5]\*\*:.*)$',
            '- [ ] $1'
        )
        $content = [regex]::Replace(
            $content,
            '(?m)^(\|\s*STATE-0[1-5]\s*\|\s*Phase 1\s*\|)\s*(?:Pending|Complete)\s*(\|)$',
            '${1} <status> $2'
        )
    }

    return $content
}

function Get-AuditSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )

    $content = Get-CanonicalDigestContent -Path $Path -RelativePath $RelativePath
    $bytes = (New-Object System.Text.UTF8Encoding($false)).GetBytes($content)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Add-AuditError {
    param([Parameter(Mandatory = $true)][string]$Message)

    $script:AuditErrors.Add($Message) | Out-Null
}

function Get-Frontmatter {
    param(
        [Parameter(Mandatory = $true)][string]$Content,
        [Parameter(Mandatory = $true)][string]$PlanName
    )

    $match = [regex]::Match($Content, '(?s)\A---\r?\n(?<yaml>.*?)\r?\n---(?:\r?\n|$)')
    if (-not $match.Success) {
        Add-AuditError "$PlanName 缺少合法 YAML frontmatter。"
        return ''
    }

    return $match.Groups['yaml'].Value
}

function Get-YamlScalar {
    param(
        [Parameter(Mandatory = $true)][string]$Yaml,
        [Parameter(Mandatory = $true)][string]$Key
    )

    $match = [regex]::Match($Yaml, "(?m)^$([regex]::Escape($Key)):\s*(?<value>[^\r\n#]+?)\s*$")
    if (-not $match.Success) {
        return $null
    }

    return $match.Groups['value'].Value.Trim().Trim('"', "'")
}

function Get-YamlInlineList {
    param(
        [Parameter(Mandatory = $true)][string]$Yaml,
        [Parameter(Mandatory = $true)][string]$Key
    )

    $scalar = Get-YamlScalar -Yaml $Yaml -Key $Key
    if ($null -eq $scalar) {
        return @()
    }

    if ($scalar -notmatch '^\[(?<items>.*)\]$') {
        return @($scalar)
    }

    $items = $Matches['items'].Trim()
    if ([string]::IsNullOrWhiteSpace($items)) {
        return @()
    }

    return @(
        $items.Split(',') |
            ForEach-Object { $_.Trim().Trim('"', "'") } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    )
}

function Get-YamlBlockList {
    param(
        [Parameter(Mandatory = $true)][string]$Yaml,
        [Parameter(Mandatory = $true)][string]$Key
    )

    $lines = $Yaml -split '\r?\n'
    $values = New-Object System.Collections.Generic.List[string]
    $inside = $false
    $indent = -1

    foreach ($line in $lines) {
        if (-not $inside) {
            $header = [regex]::Match($line, "^(?<indent>\s*)$([regex]::Escape($Key)):\s*$")
            if ($header.Success) {
                $inside = $true
                $indent = $header.Groups['indent'].Value.Length
            }
            continue
        }

        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }

        $currentIndent = $line.Length - $line.TrimStart().Length
        if ($currentIndent -le $indent) {
            break
        }

        $item = [regex]::Match($line, '^\s*-\s+(?<value>.+?)\s*$')
        if ($item.Success) {
            $values.Add($item.Groups['value'].Value.Trim().Trim('"', "'")) | Out-Null
        }
    }

    return $values.ToArray()
}

function Test-ConcretePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (
        -not [string]::IsNullOrWhiteSpace($Path) -and
        $Path -notmatch '[*?\[\]{}]' -and
        $Path -notmatch '(^|/)\.\.(/|$)' -and
        -not [System.IO.Path]::IsPathRooted($Path)
    )
}

function Get-TaskRecords {
    param(
        [Parameter(Mandatory = $true)][string]$Content,
        [Parameter(Mandatory = $true)][string]$PlanName
    )

    $records = New-Object System.Collections.Generic.List[object]
    $matches = [regex]::Matches($Content, '(?s)<task\s+(?<attrs>[^>]+)>(?<body>.*?)</task>')
    if ($matches.Count -eq 0) {
        Add-AuditError "$PlanName 未定义任何 <task>。"
        return @()
    }

    foreach ($match in $matches) {
        $attrs = $match.Groups['attrs'].Value
        $body = $match.Groups['body'].Value
        $typeMatch = [regex]::Match($attrs, '\btype="(?<type>[^"]+)"')
        if (-not $typeMatch.Success) {
            Add-AuditError "$PlanName 存在未声明 type 的 task。"
            continue
        }

        foreach ($tag in @('name', 'verify', 'done')) {
            $tagPattern = '(?s)<{0}(?:\s+[^>]*)?>.*?</{0}>' -f [regex]::Escape($tag)
            if ($body -notmatch $tagPattern) {
                Add-AuditError ('{0} 的 task 缺少 <{1}>' -f $PlanName, $tag)
            }
        }
        if ($typeMatch.Success -and
            $typeMatch.Groups['type'].Value -ne 'checkpoint:decision' -and
            $body -notmatch '(?s)<action>.*?</action>') {
            Add-AuditError "$PlanName 的 $($typeMatch.Groups['type'].Value) task 缺少 <action>。"
        }
        if ($typeMatch.Success -and
            $typeMatch.Groups['type'].Value -match '^(auto|tracer)$' -and
            $body -notmatch '(?s)<files>.*?</files>') {
            Add-AuditError "$PlanName 的 $($typeMatch.Groups['type'].Value) task 缺少 <files>。"
        }
        if ($body -notmatch '(?s)<verify\b[^>]*>.*?<automated\b[^>]*>.+?</automated>.*?</verify>') {
            Add-AuditError "$PlanName 的 task 必须包含非空 <verify><automated>。"
        }

        $filesMatch = [regex]::Match($body, '(?s)<files>(?<files>.*?)</files>')
        $taskFiles = @()
        if ($filesMatch.Success) {
            $taskFiles = @(
                $filesMatch.Groups['files'].Value.Split(',') |
                    ForEach-Object { $_.Trim() } |
                    Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
            )
        }

        $records.Add([pscustomobject]@{
            Type = $typeMatch.Groups['type'].Value
            Files = $taskFiles
            Body = $body
        }) | Out-Null
    }

    return $records.ToArray()
}

function Expand-PlanReferences {
    param([Parameter(Mandatory = $true)][string]$Value)

    $numbers = New-Object System.Collections.Generic.HashSet[int]
    $normalized = $Value.Replace('–', '-').Replace('—', '-')
    foreach ($part in ($normalized -split '\s*,\s*')) {
        $trimmed = $part.Trim()
        if ($trimmed -match '^(?<start>\d{1,2})-(?<end>\d{1,2})$') {
            $start = [int]$Matches['start']
            $end = [int]$Matches['end']
            if ($end -lt $start) {
                Add-AuditError "SOURCE-AUDIT 含倒序计划范围：$trimmed"
                continue
            }
            for ($number = $start; $number -le $end; $number++) {
                $numbers.Add($number) | Out-Null
            }
        }
        elseif ($trimmed -match '^\d{1,2}$') {
            $numbers.Add([int]$trimmed) | Out-Null
        }
        elseif (-not [string]::IsNullOrWhiteSpace($trimmed)) {
            Add-AuditError "SOURCE-AUDIT 含无法解析的计划引用：$trimmed"
        }
    }

    return @($numbers | Sort-Object)
}

function Get-SourceAuditMappings {
    param([Parameter(Mandatory = $true)][string]$Content)

    $records = New-Object System.Collections.Generic.List[object]
    foreach ($line in ($Content -split '\r?\n')) {
        if ($line -notmatch '^\|\s*(?<source>GOAL|REQ|RESEARCH|CONTEXT|ADR|PATTERNS|VALIDATION)\s*\|\s*(?<id>[^|]+?)\s*\|\s*(?<item>[^|]+?)\s*\|\s*(?<plans>[^|]+?)\s*\|\s*(?<status>[^|]+?)\s*\|$') {
            continue
        }

        $records.Add([pscustomobject]@{
            Source = $Matches['source'].Trim()
            Id = $Matches['id'].Trim()
            Item = $Matches['item'].Trim()
            PlanText = $Matches['plans'].Trim()
            Plans = @(Expand-PlanReferences -Value $Matches['plans'].Trim())
            Status = $Matches['status'].Trim()
        }) | Out-Null
    }

    return $records.ToArray()
}

function Get-PatternPathMappings {
    param([Parameter(Mandatory = $true)][string]$Content)

    $records = New-Object System.Collections.Generic.List[object]
    foreach ($line in ($Content -split '\r?\n')) {
        if ($line -notmatch '^\|') {
            continue
        }

        $codeTick = [char]96
        $pathPattern = '^\|\s*' + $codeTick + '(?<path>[^' + $codeTick + '\r\n]+)' + $codeTick + '\s*\|'
        $pathMatch = [regex]::Match($line, $pathPattern)
        if (-not $pathMatch.Success) {
            continue
        }

        $planMatch = [regex]::Match($line, '\|\s*(?:Plan\s*)?(?<plan>\d{1,2})(?:\s*[-–][^|]*)?\s*\|', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
        if (-not $planMatch.Success) {
            $planMatch = [regex]::Match($line, '\b01-(?<plan>\d{2})\b')
        }

        $records.Add([pscustomobject]@{
            Path = Convert-ToForwardSlash $pathMatch.Groups['path'].Value.Trim()
            Plan = if ($planMatch.Success) { [int]$planMatch.Groups['plan'].Value } else { $null }
            Line = $line
        }) | Out-Null
    }

    return $records.ToArray()
}

function Test-RunnerInvocations {
    param(
        [Parameter(Mandatory = $true)][string]$Content,
        [Parameter(Mandatory = $true)][string]$PlanName,
        [Parameter(Mandatory = $true)]$RunnerMatrix
    )

    $legacyPatterns = @(
        '-Require[A-Za-z0-9]+',
        '-Scope\s+Packaging\b',
        '-Scope\s+Contract\b',
        '-AllowBlocked\b(?!\s+(?:true|false))'
    )
    foreach ($legacyPattern in $legacyPatterns) {
        if ($Content -match $legacyPattern) {
            Add-AuditError "$PlanName 使用旧 runner 参数或值：$($Matches[0])"
        }
    }

    $automatedBlocks = [regex]::Matches($Content, '(?s)<automated>(?<command>.*?)</automated>')
    foreach ($automatedBlock in $automatedBlocks) {
        $command = $automatedBlock.Groups['command'].Value
        $invocations = [regex]::Matches(
            $command,
            '(?im)^[^\r\n]*run-phase1-contracts\.ps1(?<args>[^\r\n<]*)'
        )
        foreach ($invocation in $invocations) {
            $args = $invocation.Groups['args'].Value
        $values = @{}
        foreach ($parameter in @('Scope', 'Target', 'Mode')) {
            $parameterMatch = [regex]::Match($args, "(?i)-$parameter\s+(?<value>[A-Za-z0-9_-]+)")
            if (-not $parameterMatch.Success) {
                Add-AuditError "$PlanName 的 runner 调用未显式声明 -$parameter。"
                continue
            }
            $values[$parameter] = $parameterMatch.Groups['value'].Value
        }

        if ($values.Count -ne 3) {
            continue
        }

        $combination = @(
            $RunnerMatrix.combinations |
                Where-Object {
                    $_.scope -eq $values['Scope'] -and
                    $_.target -eq $values['Target'] -and
                    @($_.modes) -contains $values['Mode']
                }
        )
        if ($combination.Count -ne 1) {
            Add-AuditError "$PlanName 的 runner 调用不在 matrix 中：Scope=$($values['Scope']) Target=$($values['Target']) Mode=$($values['Mode'])"
        }
        }
    }
}

function Get-DigestInputPaths {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$ResolvedPhaseDir,
        [Parameter(Mandatory = $true)][System.IO.FileInfo[]]$PlanFiles
    )

    $paths = New-Object System.Collections.Generic.List[string]
    foreach ($relativePath in @(
        '.planning/ROADMAP.md',
        '.planning/REQUIREMENTS.md',
        '.planning/phases/01-trusted-local-state-contract/01-PATTERNS.md',
        '.planning/phases/01-trusted-local-state-contract/01-VALIDATION.md',
        '.planning/phases/01-trusted-local-state-contract/01-SOURCE-AUDIT.md',
        'tests/fixtures/contracts/runner-cli-matrix.json'
    )) {
        $paths.Add($relativePath) | Out-Null
    }

    foreach ($planFile in $PlanFiles) {
        $paths.Add((Get-RelativePathPortable -BasePath $RepositoryRoot -TargetPath $planFile.FullName)) | Out-Null
    }

    return @($paths | Sort-Object -Unique)
}

function New-DigestLockObject {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string[]]$RelativePaths
    )

    $files = [ordered]@{}
    foreach ($relativePath in ($RelativePaths | Sort-Object)) {
        $fullPath = Join-Path $RepositoryRoot ($relativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar))
        if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
            Add-AuditError "digest 输入不存在：$relativePath"
            continue
        }
        $files[$relativePath] = Get-AuditSha256 -Path $fullPath -RelativePath $relativePath
    }

    return [ordered]@{
        schema_version = 1
        phase = '01'
        algorithm = 'SHA-256'
        excluded = @('tests/fixtures/contracts/phase1-plan-audit-lock.json')
        normalization = [ordered]@{
            '.planning/ROADMAP.md' = 'execution-progress-v1'
            '.planning/REQUIREMENTS.md' = 'requirement-status-v1'
        }
        files = $files
    }
}

if ($ReadOnly -and $UpdateLock) {
    throw '-ReadOnly 与 -UpdateLock 不能同时使用。'
}

$script:AuditErrors = New-Object System.Collections.Generic.List[string]
$resolvedPhaseDir = Get-NormalizedFullPath -Path $PhaseDir
$repositoryRoot = Get-RepositoryRoot -ResolvedPhaseDir $resolvedPhaseDir
$resolvedLockPath = if ([string]::IsNullOrWhiteSpace($LockPath)) {
    Join-Path $repositoryRoot 'tests/fixtures/contracts/phase1-plan-audit-lock.json'
}
else {
    Get-NormalizedFullPath -Path $LockPath
}

$requiredFiles = [ordered]@{
    Roadmap = Join-Path $repositoryRoot '.planning/ROADMAP.md'
    Requirements = Join-Path $repositoryRoot '.planning/REQUIREMENTS.md'
    Patterns = Join-Path $resolvedPhaseDir '01-PATTERNS.md'
    Validation = Join-Path $resolvedPhaseDir '01-VALIDATION.md'
    SourceAudit = Join-Path $resolvedPhaseDir '01-SOURCE-AUDIT.md'
    RunnerMatrix = Join-Path $repositoryRoot 'tests/fixtures/contracts/runner-cli-matrix.json'
}

foreach ($entry in $requiredFiles.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        Add-AuditError "缺少必需输入 $($entry.Key)：$(Get-RelativePathPortable -BasePath $repositoryRoot -TargetPath $entry.Value)"
    }
}
if ($AuditErrors.Count -gt 0) {
    $AuditErrors | ForEach-Object { Write-Error $_ }
    exit 2
}

$roadmapContent = Get-Content -LiteralPath $requiredFiles.Roadmap -Raw -Encoding UTF8
$requirementsContent = Get-Content -LiteralPath $requiredFiles.Requirements -Raw -Encoding UTF8
$patternsContent = Get-Content -LiteralPath $requiredFiles.Patterns -Raw -Encoding UTF8
$validationContent = Get-Content -LiteralPath $requiredFiles.Validation -Raw -Encoding UTF8
$sourceAuditContent = Get-Content -LiteralPath $requiredFiles.SourceAudit -Raw -Encoding UTF8
$runnerMatrix = Get-Content -LiteralPath $requiredFiles.RunnerMatrix -Raw -Encoding UTF8 | ConvertFrom-Json
$planFiles = @(
    Get-ChildItem -LiteralPath $resolvedPhaseDir -Filter '01-??-PLAN.md' -File |
        Sort-Object Name
)

if ($planFiles.Count -ne 28) {
    Add-AuditError "Phase 1 必须恰有 28 个计划，实际为 $($planFiles.Count)。"
}

$plans = @{}
$allFilesByWave = @{}
$requirementsToPlans = @{}
foreach ($requirementId in @('STATE-01', 'STATE-02', 'STATE-03', 'STATE-04', 'STATE-05')) {
    $requirementsToPlans[$requirementId] = New-Object System.Collections.Generic.List[int]
}

for ($index = 1; $index -le 28; $index++) {
    $expectedName = '01-{0:D2}-PLAN.md' -f $index
    if (-not ($planFiles.Name -contains $expectedName)) {
        Add-AuditError "缺少顺序计划：$expectedName"
    }
}

foreach ($planFile in $planFiles) {
    $content = Get-Content -LiteralPath $planFile.FullName -Raw -Encoding UTF8
    $yaml = Get-Frontmatter -Content $content -PlanName $planFile.Name
    $planValue = Get-YamlScalar -Yaml $yaml -Key 'plan'
    $waveValue = Get-YamlScalar -Yaml $yaml -Key 'wave'
    if ($planFile.Name -notmatch '^01-(?<number>\d{2})-PLAN\.md$') {
        Add-AuditError "计划文件名不合法：$($planFile.Name)"
        continue
    }
    $number = [int]$Matches['number']
    if ($planValue -notmatch '^\d{1,2}$' -or [int]$planValue -ne $number) {
        Add-AuditError "$($planFile.Name) 的 frontmatter plan 与文件名不一致。"
    }
    if ($waveValue -notmatch '^\d+$') {
        Add-AuditError "$($planFile.Name) 缺少整数 wave。"
        $wave = -1
    }
    else {
        $wave = [int]$waveValue
    }

    $filesModified = @(Get-YamlBlockList -Yaml $yaml -Key 'files_modified' | ForEach-Object { Convert-ToForwardSlash $_ })
    foreach ($path in $filesModified) {
        if (-not (Test-ConcretePath -Path $path)) {
            Add-AuditError "$($planFile.Name) 的 files_modified 不是具体仓库相对路径：$path"
        }
    }

    $artifactPaths = @(
        [regex]::Matches(
            $content,
            '(?ms)^## Artifacts this phase produces\s*(?<body>.*?)(?=^<tasks>|^## |\z)'
        ) |
            ForEach-Object {
                $codeTick = [char]96
                $artifactPattern = '(?m)^\s*-\s+' + $codeTick + '(?<path>[^' + $codeTick + ']+)' + $codeTick
                [regex]::Matches($_.Groups['body'].Value, $artifactPattern) |
                    ForEach-Object { Convert-ToForwardSlash $_.Groups['path'].Value.Trim() }
            }
    )
    $frontmatterSet = @($filesModified | Sort-Object -Unique)
    $artifactSet = @($artifactPaths | Sort-Object -Unique)
    if (@(Compare-Object -ReferenceObject $frontmatterSet -DifferenceObject $artifactSet).Count -gt 0) {
        Add-AuditError "$($planFile.Name) 的 Artifacts 集合与 files_modified 不相等。"
    }

    $tasks = @(Get-TaskRecords -Content $content -PlanName $planFile.Name)
    foreach ($task in $tasks) {
        if ($task.Type -notmatch '^(auto|tracer|checkpoint:(?:human-verify|human-action|decision))$') {
            Add-AuditError "$($planFile.Name) 使用未知 task type：$($task.Type)"
        }
        foreach ($taskFile in $task.Files) {
            $normalizedTaskFile = Convert-ToForwardSlash $taskFile
            if (-not (Test-ConcretePath -Path $normalizedTaskFile)) {
                Add-AuditError "$($planFile.Name) 的 task files 不是具体路径：$normalizedTaskFile"
            }
            if ($frontmatterSet -notcontains $normalizedTaskFile) {
                Add-AuditError "$($planFile.Name) 的 task file 未在 files_modified 中声明：$normalizedTaskFile"
            }
        }
    }

    $requirements = @(Get-YamlBlockList -Yaml $yaml -Key 'requirements')
    if ($requirements.Count -eq 0) {
        $requirements = @(Get-YamlInlineList -Yaml $yaml -Key 'requirements')
    }
    foreach ($requirement in $requirements) {
        if ($requirement -notmatch '^STATE-(?<id>\d{2})$') {
            Add-AuditError "$($planFile.Name) 含非法 requirement ID：$requirement"
            continue
        }
        if (@('STATE-01', 'STATE-02', 'STATE-03', 'STATE-04', 'STATE-05') -notcontains $requirement) {
            Add-AuditError "$($planFile.Name) 引用了 Phase 2–8 requirement：$requirement"
            continue
        }
        $requirementsToPlans[$requirement].Add($number) | Out-Null
    }

    $dependsOn = @(Get-YamlInlineList -Yaml $yaml -Key 'depends_on')
    Test-RunnerInvocations -Content $content -PlanName $planFile.Name -RunnerMatrix $runnerMatrix

    foreach ($threatLine in ($content -split '\r?\n')) {
        if ($threatLine -notmatch '^\|\s*T-[^|]+\|\s*[^|]+\|\s*[^|]+\|\s*(?<severity>high|critical)\s*\|\s*(?<disposition>[^|]+)\|') {
            continue
        }
        if ($Matches['disposition'].Trim().ToLowerInvariant() -ne 'mitigate') {
            Add-AuditError "$($planFile.Name) 的 $($Matches['severity']) threat 未标记 mitigate：$threatLine"
        }
    }

    foreach ($keyLinkBlock in [regex]::Matches($yaml, '(?ms)^\s{2}-\s+from:\s*"?(?<from>[^"\r\n]+)"?\s*\r?\n\s+to:\s*"?(?<to>[^"\r\n]+)"?\s*\r?\n\s+via:\s*"?(?<via>[^"\r\n]+)"?\s*\r?\n\s+pattern:\s*"?(?<pattern>[^"\r\n]+)"?')) {
        foreach ($endpoint in @($keyLinkBlock.Groups['from'].Value.Trim(), $keyLinkBlock.Groups['to'].Value.Trim())) {
            if (-not (Test-ConcretePath -Path (Convert-ToForwardSlash $endpoint))) {
                Add-AuditError "$($planFile.Name) 的 key_link endpoint 不是具体路径：$endpoint"
            }
        }
        $fromPath = Join-Path $repositoryRoot ($keyLinkBlock.Groups['from'].Value.Trim().Replace('/', [System.IO.Path]::DirectorySeparatorChar))
        $toPath = Join-Path $repositoryRoot ($keyLinkBlock.Groups['to'].Value.Trim().Replace('/', [System.IO.Path]::DirectorySeparatorChar))
        if ((Test-Path -LiteralPath $fromPath -PathType Leaf) -and
            (Get-Content -LiteralPath $fromPath -Raw -Encoding UTF8) -notmatch [regex]::Escape($keyLinkBlock.Groups['pattern'].Value.Trim())) {
            Add-AuditError "$($planFile.Name) 的 key_link pattern 未出现在 from 文件中：$($keyLinkBlock.Groups['pattern'].Value.Trim())"
        }
        if ((Test-Path -LiteralPath $fromPath -PathType Leaf) -and
            (Test-Path -LiteralPath $toPath -PathType Leaf)) {
            $fromContent = Get-Content -LiteralPath $fromPath -Raw -Encoding UTF8
            $toReference = Convert-ToForwardSlash $keyLinkBlock.Groups['to'].Value.Trim()
            $toLeaf = Split-Path -Leaf $toReference
            if ($fromContent -notmatch [regex]::Escape($toReference) -and
                $fromContent -notmatch [regex]::Escape($toLeaf)) {
                Add-AuditError "$($planFile.Name) 的 key_link from 未引用 to：$toReference"
            }
        }
    }

    $plans[$number] = [pscustomobject]@{
        Number = $number
        Name = $planFile.Name
        Wave = $wave
        DependsOn = $dependsOn
        Files = $frontmatterSet
        Requirements = $requirements
        Content = $content
    }

    if (-not $allFilesByWave.ContainsKey($wave)) {
        $allFilesByWave[$wave] = @{}
    }
    foreach ($path in $frontmatterSet) {
        if ($allFilesByWave[$wave].ContainsKey($path)) {
            Add-AuditError "同 wave 文件冲突：wave $wave 的 $path 同时由 $($allFilesByWave[$wave][$path]) 与 $($planFile.Name) 修改。"
        }
        else {
            $allFilesByWave[$wave][$path] = $planFile.Name
        }
    }
}

foreach ($plan in $plans.Values) {
    foreach ($dependency in $plan.DependsOn) {
        if ($dependency -notmatch '^(?:01-)?(?<number>\d{2})$') {
            Add-AuditError "$($plan.Name) 的 depends_on 格式不合法：$dependency"
            continue
        }
        $dependencyNumber = [int]$Matches['number']
        if (-not $plans.Contains($dependencyNumber)) {
            Add-AuditError "$($plan.Name) 依赖不存在的计划：$dependency"
            continue
        }
        if ($plan.Wave -le $plans[$dependencyNumber].Wave) {
            Add-AuditError "$($plan.Name) 的 wave $($plan.Wave) 未严格晚于依赖 $dependency 的 wave $($plans[$dependencyNumber].Wave)。"
        }
    }
}

foreach ($requirementId in $requirementsToPlans.Keys) {
    if ($requirementsToPlans[$requirementId].Count -eq 0) {
        Add-AuditError "$requirementId 未映射到任何计划。"
    }
    if ($requirementsContent -notmatch "(?m)^-\s+\[[ xX]\]\s+\*\*$([regex]::Escape($requirementId))\*\*:") {
        Add-AuditError "REQUIREMENTS.md 缺少 $requirementId 定义。"
    }
    if ($requirementsContent -notmatch "(?m)^\|\s*$([regex]::Escape($requirementId))\s*\|\s*Phase 1\s*\|") {
        Add-AuditError "REQUIREMENTS.md traceability 未把 $requirementId 分配给 Phase 1。"
    }
}

$roadmapPlanNumbers = @(
    [regex]::Matches($roadmapContent, '(?m)^-\s+\[[ xX]\]\s+01-(?<number>\d{2})-PLAN\.md\b') |
        ForEach-Object { [int]$_.Groups['number'].Value }
)
if ($roadmapPlanNumbers.Count -ne 28 -or
    @(Compare-Object -ReferenceObject @(1..28) -DifferenceObject @($roadmapPlanNumbers | Sort-Object -Unique)).Count -gt 0) {
    Add-AuditError 'ROADMAP.md 的 Phase 1 计划清单必须完整覆盖 01-01..01-28。'
}
foreach ($requirementId in @('STATE-01', 'STATE-02', 'STATE-03', 'STATE-04', 'STATE-05')) {
    if ($roadmapContent -notmatch [regex]::Escape($requirementId)) {
        Add-AuditError "ROADMAP.md 未引用 $requirementId。"
    }
}

$sourceMappings = @(Get-SourceAuditMappings -Content $sourceAuditContent)
$requiredSourceRows = @(
    'GOAL:—',
    'REQ:STATE-01', 'REQ:STATE-02', 'REQ:STATE-03', 'REQ:STATE-04', 'REQ:STATE-05',
    'RESEARCH:R-01', 'RESEARCH:R-02', 'RESEARCH:R-03', 'RESEARCH:R-04',
    'RESEARCH:R-05', 'RESEARCH:R-06', 'RESEARCH:R-07', 'RESEARCH:R-08',
    'RESEARCH:R-09', 'RESEARCH:R-10', 'RESEARCH:R-11', 'RESEARCH:R-12',
    'RESEARCH:R-13', 'RESEARCH:R-14', 'RESEARCH:R-15', 'RESEARCH:R-16',
    'RESEARCH:R-17',
    'CONTEXT:CTX-01', 'CONTEXT:CTX-02', 'CONTEXT:CTX-03', 'CONTEXT:CTX-04',
    'PATTERNS:P-01', 'VALIDATION:V-01'
)
$actualSourceRows = @($sourceMappings | ForEach-Object { "$($_.Source):$($_.Id)" })
foreach ($requiredSourceRow in $requiredSourceRows) {
    if ($actualSourceRows -notcontains $requiredSourceRow) {
        Add-AuditError "SOURCE-AUDIT 缺少映射：$requiredSourceRow"
    }
}
foreach ($mapping in $sourceMappings) {
    if ($mapping.Plans.Count -eq 0) {
        Add-AuditError "SOURCE-AUDIT 映射没有计划：$($mapping.Source)/$($mapping.Id)"
    }
    foreach ($planNumber in $mapping.Plans) {
        if (-not $plans.Contains($planNumber)) {
            Add-AuditError "SOURCE-AUDIT 映射引用不存在计划：$($mapping.Source)/$($mapping.Id) -> $planNumber"
        }
    }
}

if ($sourceAuditContent -notmatch '文本状态不参与最终通过判定' -or
    $sourceAuditContent -notmatch '不得解释为执行通过') {
    Add-AuditError 'SOURCE-AUDIT 必须明确静态状态不能授予最终通过。'
}

$patternMappings = @(Get-PatternPathMappings -Content $patternsContent)
$patternPaths = @($patternMappings | ForEach-Object { $_.Path } | Sort-Object -Unique)
foreach ($patternMapping in $patternMappings) {
    if (-not (Test-ConcretePath -Path $patternMapping.Path)) {
        Add-AuditError "PATTERNS 含非具体关键路径：$($patternMapping.Path)"
    }
}
foreach ($plan in $plans.Values) {
    foreach ($path in $plan.Files) {
        if ($patternPaths -notcontains $path) {
            Add-AuditError "$($plan.Name) 的 files_modified 未在 PATTERNS 中分类：$path"
        }
    }
}
$codeTickForGlob = [char]96
$globPattern = '(?m)' + $codeTickForGlob + '[^' + $codeTickForGlob + '\r\n]*[*?\[\]][^' + $codeTickForGlob + '\r\n]*' + $codeTickForGlob
if ($patternsContent -match $globPattern) {
    Add-AuditError "PATTERNS 含通配关键路径：$($Matches[0])"
}

foreach ($requiredValidationToken in @(
    'source_audit_reparses_requirements_plans_paths_runner_waves_threats_and_current_digests',
    'final gate 依赖 read-only machine source audit',
    'high/critical threats 均 disposition=mitigate'
)) {
    if ($validationContent -notmatch [regex]::Escape($requiredValidationToken)) {
        Add-AuditError "VALIDATION 缺少最终审计保证：$requiredValidationToken"
    }
}

$digestPaths = @(Get-DigestInputPaths -RepositoryRoot $repositoryRoot -ResolvedPhaseDir $resolvedPhaseDir -PlanFiles $planFiles)
$currentLock = New-DigestLockObject -RepositoryRoot $repositoryRoot -RelativePaths $digestPaths

if ($UpdateLock) {
    if ($AuditErrors.Count -gt 0) {
        Write-Host "Phase 1 计划/来源审计失败，拒绝更新 digest lock：" -ForegroundColor Red
        $AuditErrors | ForEach-Object { Write-Host " - $_" -ForegroundColor Red }
        exit 2
    }

    $lockDirectory = Split-Path -Parent $resolvedLockPath
    if (-not (Test-Path -LiteralPath $lockDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $lockDirectory -Force | Out-Null
    }
    $json = ($currentLock | ConvertTo-Json -Depth 8) -replace "\r\n?", "`n"
    [System.IO.File]::WriteAllText(
        $resolvedLockPath,
        ($json.TrimEnd() + "`n"),
        (New-Object System.Text.UTF8Encoding($false))
    )
    Write-Host "已更新 Phase 1 digest lock：$(Get-RelativePathPortable -BasePath $repositoryRoot -TargetPath $resolvedLockPath)"
    exit 0
}

if (-not (Test-Path -LiteralPath $resolvedLockPath -PathType Leaf)) {
    Add-AuditError "缺少 digest lock：$(Get-RelativePathPortable -BasePath $repositoryRoot -TargetPath $resolvedLockPath)"
}
else {
    try {
        $expectedLock = Get-Content -LiteralPath $resolvedLockPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($expectedLock.schema_version -ne 1 -or $expectedLock.phase -ne '01' -or $expectedLock.algorithm -ne 'SHA-256') {
            Add-AuditError 'digest lock 元数据不合法。'
        }
        $expectedFiles = @{}
        foreach ($property in $expectedLock.files.PSObject.Properties) {
            $expectedFiles[$property.Name] = [string]$property.Value
        }
        $currentFiles = @{}
        foreach ($entry in $currentLock.files.GetEnumerator()) {
            $currentFiles[$entry.Key] = [string]$entry.Value
        }
        $pathDiff = @(Compare-Object -ReferenceObject @($currentFiles.Keys | Sort-Object) -DifferenceObject @($expectedFiles.Keys | Sort-Object))
        if ($pathDiff.Count -gt 0) {
            Add-AuditError 'digest lock 文件集合与当前审计输入不一致。'
        }
        foreach ($relativePath in $currentFiles.Keys) {
            if (-not $expectedFiles.ContainsKey($relativePath)) {
                continue
            }
            if ($expectedFiles[$relativePath].ToLowerInvariant() -ne $currentFiles[$relativePath].ToLowerInvariant()) {
                Add-AuditError "digest 漂移：$relativePath"
            }
        }
    }
    catch {
        Add-AuditError "digest lock 无法解析：$($_.Exception.Message)"
    }
}

if ($AuditErrors.Count -gt 0) {
    Write-Host "Phase 1 计划/来源审计失败：" -ForegroundColor Red
    $AuditErrors | Sort-Object -Unique | ForEach-Object { Write-Host " - $_" -ForegroundColor Red }
    exit 2
}

Write-Host "Phase 1 计划/来源审计通过：28 个计划、5 个 requirements、拓扑/路径/CLI/threat/digest 均一致。"
exit 0
