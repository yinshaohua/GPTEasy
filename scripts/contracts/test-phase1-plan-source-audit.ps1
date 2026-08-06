[CmdletBinding()]
param()

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Get-NormalizedFullPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [System.IO.Path]::GetFullPath($Path)
}

function Assert-PathUnderDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Directory
    )

    $resolvedPath = Get-NormalizedFullPath -Path $Path
    $resolvedDirectory = (Get-NormalizedFullPath -Path $Directory).TrimEnd('\', '/')
    $prefix = $resolvedDirectory + [System.IO.Path]::DirectorySeparatorChar
    if (-not $resolvedPath.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "拒绝操作预期目录之外的路径：$resolvedPath"
    }
}

function Read-Utf8File {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
}

function Write-Utf8File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )

    $normalized = $Content -replace "\r\n?", "`n"
    [System.IO.File]::WriteAllText(
        $Path,
        $normalized,
        (New-Object System.Text.UTF8Encoding($false))
    )
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
    return [System.Uri]::UnescapeDataString($baseUri.MakeRelativeUri($targetUri).ToString()).Replace('\', '/')
}

function Get-ProductionHashSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$PhaseDir
    )

    $paths = New-Object System.Collections.Generic.List[string]
    foreach ($path in @(
        '.planning/ROADMAP.md',
        '.planning/REQUIREMENTS.md',
        '.planning/phases/01-trusted-local-state-contract/01-PATTERNS.md',
        '.planning/phases/01-trusted-local-state-contract/01-VALIDATION.md',
        '.planning/phases/01-trusted-local-state-contract/01-SOURCE-AUDIT.md',
        'tests/fixtures/contracts/runner-cli-matrix.json',
        'tests/fixtures/contracts/phase1-plan-audit-lock.json'
    )) {
        $paths.Add($path) | Out-Null
    }
    foreach ($planFile in (Get-ChildItem -LiteralPath $PhaseDir -Filter '01-??-PLAN.md' -File)) {
        $paths.Add((Get-RelativePathPortable -BasePath $RepositoryRoot -TargetPath $planFile.FullName)) | Out-Null
    }

    $snapshot = @{}
    foreach ($relativePath in ($paths | Sort-Object -Unique)) {
        $fullPath = Join-Path $RepositoryRoot ($relativePath.Replace('/', [System.IO.Path]::DirectorySeparatorChar))
        $snapshot[$relativePath] = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return $snapshot
}

function Assert-SnapshotsEqual {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After
    )

    $beforeKeys = @($Before.Keys | Sort-Object)
    $afterKeys = @($After.Keys | Sort-Object)
    if (@(Compare-Object -ReferenceObject $beforeKeys -DifferenceObject $afterKeys).Count -gt 0) {
        throw '生产审计输入文件集合在自测前后发生变化。'
    }
    foreach ($path in $beforeKeys) {
        if ($Before[$path] -ne $After[$path]) {
            throw "生产审计输入在自测期间被修改：$path"
        }
    }
}

function Copy-AuditWorkspace {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$SourcePhaseDir,
        [Parameter(Mandatory = $true)][string]$DestinationRoot
    )

    $planningRoot = Join-Path $DestinationRoot '.planning'
    $phaseParent = Join-Path $planningRoot 'phases'
    $fixtureRoot = Join-Path $DestinationRoot 'tests/fixtures/contracts'
    $scriptRoot = Join-Path $DestinationRoot 'scripts/contracts'
    New-Item -ItemType Directory -Path $phaseParent -Force | Out-Null
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $scriptRoot -Force | Out-Null

    Copy-Item -LiteralPath (Join-Path $RepositoryRoot '.planning/ROADMAP.md') -Destination (Join-Path $planningRoot 'ROADMAP.md')
    Copy-Item -LiteralPath (Join-Path $RepositoryRoot '.planning/REQUIREMENTS.md') -Destination (Join-Path $planningRoot 'REQUIREMENTS.md')
    Copy-Item -LiteralPath $SourcePhaseDir -Destination $phaseParent -Recurse
    Copy-Item -LiteralPath (Join-Path $RepositoryRoot 'tests/fixtures/contracts/runner-cli-matrix.json') -Destination (Join-Path $fixtureRoot 'runner-cli-matrix.json')
    Copy-Item -LiteralPath (Join-Path $RepositoryRoot 'tests/fixtures/contracts/phase1-plan-audit-lock.json') -Destination (Join-Path $fixtureRoot 'phase1-plan-audit-lock.json')
    Copy-Item -LiteralPath (Join-Path $RepositoryRoot 'scripts/contracts/audit-phase1-plan-source.ps1') -Destination (Join-Path $scriptRoot 'audit-phase1-plan-source.ps1')
    Copy-Item -LiteralPath (Join-Path $RepositoryRoot 'scripts/contracts/run-phase1-contracts.ps1') -Destination (Join-Path $scriptRoot 'run-phase1-contracts.ps1')
    Copy-Item -LiteralPath (Join-Path $RepositoryRoot 'scripts/contracts/test-run-phase1-cli.ps1') -Destination (Join-Path $scriptRoot 'test-run-phase1-cli.ps1')
}

function New-CaseWorkspace {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$SourcePhaseDir,
        [Parameter(Mandatory = $true)][string]$TestRoot
    )

    $safeName = $Name -replace '[^A-Za-z0-9_-]', '-'
    $caseRoot = Join-Path $TestRoot ("{0}-{1}" -f $safeName, [guid]::NewGuid().ToString('N'))
    Assert-PathUnderDirectory -Path $caseRoot -Directory $TestRoot
    New-Item -ItemType Directory -Path $caseRoot -Force | Out-Null
    Copy-AuditWorkspace -RepositoryRoot $RepositoryRoot -SourcePhaseDir $SourcePhaseDir -DestinationRoot $caseRoot
    return $caseRoot
}

function Invoke-Auditor {
    param(
        [Parameter(Mandatory = $true)][string]$AuditorPath,
        [Parameter(Mandatory = $true)][string]$CaseRoot
    )

    $phaseDir = Join-Path $CaseRoot '.planning/phases/01-trusted-local-state-contract'
    $lockPath = Join-Path $CaseRoot 'tests/fixtures/contracts/phase1-plan-audit-lock.json'
    $output = & powershell -NoProfile -File $AuditorPath -PhaseDir $phaseDir -ReadOnly -LockPath $lockPath 2>&1 | Out-String
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output = $output
    }
}

function Invoke-PhaseCompleteGate {
    param(
        [Parameter(Mandatory = $true)][string]$CaseRoot
    )

    $runnerPath = Join-Path $CaseRoot 'scripts/contracts/run-phase1-contracts.ps1'
    $output = & powershell -NoProfile -File $runnerPath -Scope PhaseComplete -Target Local -Mode Strict 2>&1 | Out-String
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output = $output
    }
}

function Assert-NegativeCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Mutate,
        [Parameter(Mandatory = $true)][string]$ExpectedPattern,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$SourcePhaseDir,
        [Parameter(Mandatory = $true)][string]$TestRoot,
        [Parameter(Mandatory = $true)][string]$AuditorPath
    )

    $caseRoot = New-CaseWorkspace -Name $Name -RepositoryRoot $RepositoryRoot -SourcePhaseDir $SourcePhaseDir -TestRoot $TestRoot
    & $Mutate $caseRoot
    $result = Invoke-Auditor -AuditorPath $AuditorPath -CaseRoot $caseRoot
    if ($result.ExitCode -eq 0) {
        throw "负例意外通过：$Name"
    }
    if ($result.Output -notmatch $ExpectedPattern) {
        throw "负例 $Name 未产生预期诊断 /$ExpectedPattern/。实际输出：`n$($result.Output)"
    }
    Write-Host "[PASS] $Name"
}

function Update-FirstRunnerInvocation {
    param(
        [Parameter(Mandatory = $true)][string]$CaseRoot,
        [Parameter(Mandatory = $true)][scriptblock]$Transform
    )

    $phaseDir = Join-Path $CaseRoot '.planning/phases/01-trusted-local-state-contract'
    foreach ($planFile in (Get-ChildItem -LiteralPath $phaseDir -Filter '01-??-PLAN.md' -File | Sort-Object Name)) {
        $content = Read-Utf8File -Path $planFile.FullName
        $match = [regex]::Match($content, '(?im)^.*run-phase1-contracts\.ps1[^\r\n<]*-Scope\s+\w+[^\r\n<]*-Target\s+\w+[^\r\n<]*-Mode\s+\w+')
        if (-not $match.Success) {
            continue
        }
        $replacement = & $Transform $match.Value
        $updated = $content.Substring(0, $match.Index) + $replacement + $content.Substring($match.Index + $match.Length)
        Write-Utf8File -Path $planFile.FullName -Content $updated
        return
    }
    throw '未找到可修改的 runner invocation。'
}

$repositoryRoot = Get-NormalizedFullPath -Path (Join-Path $PSScriptRoot '..\..')
$phaseDir = Join-Path $repositoryRoot '.planning/phases/01-trusted-local-state-contract'
$auditorPath = Join-Path $repositoryRoot 'scripts/contracts/audit-phase1-plan-source.ps1'
$tempBase = Get-NormalizedFullPath -Path ([System.IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ("gpteasy-phase1-audit-tests-{0}" -f [guid]::NewGuid().ToString('N'))
Assert-PathUnderDirectory -Path $testRoot -Directory $tempBase

$beforeSnapshot = Get-ProductionHashSnapshot -RepositoryRoot $repositoryRoot -PhaseDir $phaseDir
$completed = $false

try {
    New-Item -ItemType Directory -Path $testRoot -Force | Out-Null

    $positiveRoot = New-CaseWorkspace -Name 'positive-baseline' -RepositoryRoot $repositoryRoot -SourcePhaseDir $phaseDir -TestRoot $testRoot
    $positive = Invoke-Auditor -AuditorPath $auditorPath -CaseRoot $positiveRoot
    if ($positive.ExitCode -ne 0) {
        throw "基线副本审计失败：`n$($positive.Output)"
    }
    Write-Host '[PASS] 基线副本通过实时审计'

    $positiveGate = Invoke-PhaseCompleteGate -CaseRoot $positiveRoot
    if ($positiveGate.ExitCode -ne 0 -or $positiveGate.Output -notmatch '"outcome"\s*:\s*"passed"') {
        throw "PhaseComplete 未执行并通过只读来源审计：`n$($positiveGate.Output)"
    }
    Write-Host '[PASS] PhaseComplete 执行只读来源审计'

    Assert-NegativeCase `
        -Name '缺少 requirement mapping' `
        -ExpectedPattern 'STATE-05 未映射到任何计划' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $casePhase = Join-Path $caseRoot '.planning/phases/01-trusted-local-state-contract'
            foreach ($planFile in (Get-ChildItem -LiteralPath $casePhase -Filter '01-??-PLAN.md' -File)) {
                $content = Read-Utf8File -Path $planFile.FullName
                $updated = [regex]::Replace($content, '(?m)^\s{2}- STATE-05\s*\r?\n', '')
                Write-Utf8File -Path $planFile.FullName -Content $updated
            }
        }

    Assert-NegativeCase `
        -Name '错误 runner mode' `
        -ExpectedPattern 'runner 调用不在 matrix 中' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            Update-FirstRunnerInvocation -CaseRoot $caseRoot -Transform {
                param($invocation)
                return [regex]::Replace($invocation, '(?i)(-Mode\s+)Strict\b', '${1}WrongMode', 1)
            }
        }

    Assert-NegativeCase `
        -Name '未声明 runner 参数' `
        -ExpectedPattern 'runner 调用未显式声明 -Mode' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            Update-FirstRunnerInvocation -CaseRoot $caseRoot -Transform {
                param($invocation)
                return [regex]::Replace($invocation, '(?i)\s+-Mode\s+Strict\b', '', 1)
            }
        }

    Assert-NegativeCase `
        -Name '同 wave 文件冲突' `
        -ExpectedPattern '同 wave 文件冲突' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $planPath = Join-Path $caseRoot '.planning/phases/01-trusted-local-state-contract/01-05-PLAN.md'
            $content = Read-Utf8File -Path $planPath
            $updated = [regex]::Replace(
                $content,
                '(?m)^(files_modified:\s*\r?\n)',
                ('$1' + '  - scripts/contracts/validate-contract-evidence.ps1' + "`n"),
                1
            )
            Write-Utf8File -Path $planPath -Content $updated
        }

    Assert-NegativeCase `
        -Name 'concrete path 漂移' `
        -ExpectedPattern '未在 PATTERNS 中分类' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $planPath = Join-Path $caseRoot '.planning/phases/01-trusted-local-state-contract/01-07-PLAN.md'
            $content = Read-Utf8File -Path $planPath
            $updated = $content.Replace(
                'scripts/contracts/audit-phase1-plan-source.ps1',
                'scripts/contracts/audit-phase1-plan-source-drift.ps1'
            )
            Write-Utf8File -Path $planPath -Content $updated
        }

    Assert-NegativeCase `
        -Name '关键路径通配符' `
        -ExpectedPattern '不是具体仓库相对路径' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $planPath = Join-Path $caseRoot '.planning/phases/01-trusted-local-state-contract/01-07-PLAN.md'
            $content = Read-Utf8File -Path $planPath
            $updated = $content.Replace(
                'scripts/contracts/audit-phase1-plan-source.ps1',
                'scripts/contracts/*.ps1'
            )
            Write-Utf8File -Path $planPath -Content $updated
        }

    Assert-NegativeCase `
        -Name '缺少顺序计划' `
        -ExpectedPattern '缺少顺序计划：01-28-PLAN.md|必须恰有 28 个计划' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $casePhase = Join-Path $caseRoot '.planning/phases/01-trusted-local-state-contract'
            $planPath = Join-Path $casePhase '01-28-PLAN.md'
            Assert-PathUnderDirectory -Path $planPath -Directory $casePhase
            Remove-Item -LiteralPath $planPath -Force
        }

    Assert-NegativeCase `
        -Name 'digest lock 漂移' `
        -ExpectedPattern 'digest 漂移' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $lockPath = Join-Path $caseRoot 'tests/fixtures/contracts/phase1-plan-audit-lock.json'
            $content = Read-Utf8File -Path $lockPath
            $updated = [regex]::Replace($content, '(?i)"[a-f0-9]{64}"', ('"' + ('0' * 64) + '"'), 1)
            Write-Utf8File -Path $lockPath -Content $updated
        }

    Assert-NegativeCase `
        -Name '静态 COVERED 不能关闭缺口' `
        -ExpectedPattern 'STATE-05 未映射到任何计划' `
        -RepositoryRoot $repositoryRoot `
        -SourcePhaseDir $phaseDir `
        -TestRoot $testRoot `
        -AuditorPath $auditorPath `
        -Mutate {
            param($caseRoot)
            $casePhase = Join-Path $caseRoot '.planning/phases/01-trusted-local-state-contract'
            foreach ($planFile in (Get-ChildItem -LiteralPath $casePhase -Filter '01-??-PLAN.md' -File)) {
                $content = Read-Utf8File -Path $planFile.FullName
                $updated = [regex]::Replace($content, '(?m)^\s{2}- STATE-05\s*\r?\n', '')
                Write-Utf8File -Path $planFile.FullName -Content $updated
            }
            $sourceAuditPath = Join-Path $casePhase '01-SOURCE-AUDIT.md'
            $sourceAudit = Read-Utf8File -Path $sourceAuditPath
            $covered = $sourceAudit.Replace('PLANNED-COVERAGE', 'COVERED').Replace('PLANNED', 'COVERED')
            Write-Utf8File -Path $sourceAuditPath -Content $covered
        }

    $phaseCompleteNegativeRoot = New-CaseWorkspace -Name 'phase-complete-static-covered' -RepositoryRoot $repositoryRoot -SourcePhaseDir $phaseDir -TestRoot $testRoot
    $phaseCompleteNegativePhase = Join-Path $phaseCompleteNegativeRoot '.planning/phases/01-trusted-local-state-contract'
    foreach ($planFile in (Get-ChildItem -LiteralPath $phaseCompleteNegativePhase -Filter '01-??-PLAN.md' -File)) {
        $content = Read-Utf8File -Path $planFile.FullName
        $updated = [regex]::Replace($content, '(?m)^\s{2}- STATE-05\s*\r?\n', '')
        Write-Utf8File -Path $planFile.FullName -Content $updated
    }
    $phaseCompleteSourceAuditPath = Join-Path $phaseCompleteNegativePhase '01-SOURCE-AUDIT.md'
    $phaseCompleteSourceAudit = Read-Utf8File -Path $phaseCompleteSourceAuditPath
    Write-Utf8File -Path $phaseCompleteSourceAuditPath -Content $phaseCompleteSourceAudit.Replace('PLANNED-COVERAGE', 'COVERED').Replace('PLANNED', 'COVERED')
    $phaseCompleteNegative = Invoke-PhaseCompleteGate -CaseRoot $phaseCompleteNegativeRoot
    if ($phaseCompleteNegative.ExitCode -eq 0 -or
        $phaseCompleteNegative.Output -notmatch 'phase source audit failed') {
        throw "PhaseComplete 未因实时来源审计缺口失败：`n$($phaseCompleteNegative.Output)"
    }
    Write-Host '[PASS] PhaseComplete 拒绝静态 COVERED 掩盖的实时缺口'

    $completed = $true
}
finally {
    $afterSnapshot = Get-ProductionHashSnapshot -RepositoryRoot $repositoryRoot -PhaseDir $phaseDir
    Assert-SnapshotsEqual -Before $beforeSnapshot -After $afterSnapshot
    Write-Host '[PASS] 生产规划与 digest 输入在自测前后 hash 不变'

    if (Test-Path -LiteralPath $testRoot -PathType Container) {
        Assert-PathUnderDirectory -Path $testRoot -Directory $tempBase
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}

if (-not $completed) {
    exit 2
}

Write-Host 'Phase 1 计划/来源审计负例自测通过。'
exit 0
