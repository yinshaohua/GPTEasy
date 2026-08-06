$ErrorActionPreference = 'Stop'

Set-StrictMode -Version Latest

$script:ExitCodes = @{
    Completed = 0
    StrictPrerequisiteBlocked = 3
    UsageOrCombinationError = 64
}

function Get-RepositoryRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
}

function Get-RunnerPath {
    return (Join-Path (Get-RepositoryRoot) 'scripts\contracts\run-phase1-contracts.ps1')
}

function Get-MatrixPath {
    return (Join-Path (Get-RepositoryRoot) 'tests\fixtures\contracts\runner-cli-matrix.json')
}

function Parse-Arguments {
    param(
        [string[]]$Arguments
    )

    $values = @{
        ScanPlans = $null
    }
    $errors = New-Object System.Collections.Generic.List[string]
    for ($index = 0; $index -lt $Arguments.Count; $index++) {
        $argument = $Arguments[$index]
        if ($argument -ne '-ScanPlans') {
            $errors.Add("unknown parameter: $argument")
            continue
        }
        if ($index + 1 -ge $Arguments.Count -or $Arguments[$index + 1].StartsWith('-')) {
            $errors.Add('ScanPlans is missing a value')
            continue
        }
        if ($null -ne $values.ScanPlans) {
            $errors.Add('ScanPlans was supplied more than once')
        } else {
            $values.ScanPlans = $Arguments[$index + 1]
        }
        $index++
    }
    if ([string]::IsNullOrWhiteSpace([string]$values.ScanPlans)) {
        $errors.Add('ScanPlans is required')
    }

    return [pscustomobject]@{
        Values = $values
        Errors = @($errors)
    }
}

function Read-Utf8Json {
    param(
        [string]$Path
    )

    $encoding = New-Object System.Text.UTF8Encoding($false)
    return [System.IO.File]::ReadAllText((Resolve-Path $Path), $encoding) | ConvertFrom-Json
}

function Get-StringArray {
    param(
        [AllowNull()]
        [object[]]$Value
    )

    if ($null -eq $Value) {
        return @()
    }
    return @($Value | ForEach-Object { [string]$_ })
}

function Test-SequenceEquals {
    param(
        [object[]]$Actual,
        [object[]]$Expected
    )

    return (($Actual -join "`n") -ceq ($Expected -join "`n"))
}

function Assert-Condition {
    param(
        [bool]$Condition,
        [string]$Message,
        [System.Collections.Generic.List[string]]$Failures
    )

    if (-not $Condition) {
        $Failures.Add($Message)
    }
}

function Invoke-Runner {
    param(
        [string[]]$Arguments
    )

    $output = @(& powershell -NoProfile -File (Get-RunnerPath) @Arguments 2>&1)
    $exitCode = [int]$LASTEXITCODE
    $jsonLine = @(
        $output |
            ForEach-Object { [string]$_ } |
            Where-Object { $_.TrimStart().StartsWith('{') } |
            Select-Object -Last 1
    )
    $parsed = $null
    if ($jsonLine.Count -gt 0) {
        try {
            $parsed = $jsonLine[0] | ConvertFrom-Json
        } catch {
            $parsed = $null
        }
    }

    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = $output
        Json = $parsed
    }
}

function Copy-JsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return (($Value | ConvertTo-Json -Depth 50 -Compress) | ConvertFrom-Json)
}

function Write-Utf8Json {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth 50),
        $encoding
    )
}

function Find-Combination {
    param(
        [object]$Matrix,
        [string]$Scope,
        [string]$Target,
        [string]$Mode
    )

    foreach ($combination in @($Matrix.combinations)) {
        if ([string]$combination.scope -ceq $Scope -and
            [string]$combination.target -ceq $Target -and
            (Get-StringArray $combination.modes) -contains $Mode) {
            return $combination
        }
    }
    return $null
}

function Test-MatrixAndParser {
    param(
        [object]$Matrix,
        [System.Collections.Generic.List[string]]$Failures
    )

    $expectedScopes = @('RunnerSelfTest', 'ProvenanceSelfTest', 'ContractSelfTest', 'PackagingSelfTest', 'Freeze', 'PhaseComplete')
    $expectedTargets = @('Local', 'WindowsX64', 'WindowsArm64', 'MacIntel', 'MacAppleSilicon', 'Wsl2')
    $expectedModes = @('Strict', 'AllowBlocked')
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $Matrix.cli.parameters.Scope) $expectedScopes) 'Scope values drifted from the canonical matrix' $Failures
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $Matrix.cli.parameters.Target) $expectedTargets) 'Target values drifted from the canonical matrix' $Failures
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $Matrix.cli.parameters.Mode) $expectedModes) 'Mode values drifted from the canonical matrix' $Failures
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $Matrix.cli.required_parameters) @('Scope', 'Target', 'Mode')) 'Required parameter order is not Scope, Target, Mode' $Failures

    $exitCodes = @($Matrix.exit_codes.psobject.Properties.Name | ForEach-Object { [int]$_ } | Sort-Object)
    Assert-Condition (Test-SequenceEquals $exitCodes @(0, 2, 3, 4, 5, 64)) 'Exit-code declarations are incomplete or drifted' $Failures

    $dispatchNames = @($Matrix.dispatch.psobject.Properties.Name)
    $combinationKeys = New-Object System.Collections.Generic.HashSet[string]
    $combinationCount = 0
    foreach ($combination in @($Matrix.combinations)) {
        $combinationCount++
        $key = '{0}|{1}' -f $combination.scope, $combination.target
        Assert-Condition $combinationKeys.Add($key) "Duplicate combination: $key" $Failures
        Assert-Condition ($combination.dispatch -in $dispatchNames) "Missing dispatch for $key" $Failures
        $modes = @(Get-StringArray $combination.modes)
        Assert-Condition ($modes.Count -gt 0) "No modes declared for $key" $Failures
        if ($combination.scope -in @('Freeze', 'PhaseComplete')) {
            Assert-Condition ($combination.target -eq 'Local' -and $modes.Count -eq 1 -and $modes[0] -eq 'Strict') "$key violates Freeze/PhaseComplete strict-only rule" $Failures
        }
        if ($modes -contains 'AllowBlocked') {
            Assert-Condition ($combination.scope -in @('ContractSelfTest', 'PackagingSelfTest') -and $combination.target -ne 'Local') "$key illegally permits AllowBlocked" $Failures
        }
    }
    Assert-Condition ($combinationCount -eq 16) "Unexpected canonical combination count: $combinationCount" $Failures
    Assert-Condition ([int]$Matrix.allow_blocked.exit_code -eq 0) 'AllowBlocked must use exit code 0 for blocked resources' $Failures
    Assert-Condition ([string]$Matrix.allow_blocked.outcome -ceq 'blocked') 'AllowBlocked outcome must be blocked' $Failures
    Assert-Condition (-not [bool]$Matrix.allow_blocked.strict_gate_eligible) 'AllowBlocked must never be strict eligible' $Failures

    $positive = Invoke-Runner @('-Scope', 'RunnerSelfTest', '-Target', 'Local', '-Mode', 'Strict')
    Assert-Condition ($positive.ExitCode -eq 0) "RunnerSelfTest positive case returned $($positive.ExitCode)" $Failures
    Assert-Condition ($null -ne $positive.Json -and $positive.Json.outcome -ceq 'passed' -and [bool]$positive.Json.strict_gate_eligible) 'RunnerSelfTest positive result is not strict eligible' $Failures

    $negativeCases = @(
        @('-Scope', 'RunnerSelfTest', '-Target', 'Local'),
        @('-Scope', 'RunnerSelfTest', '-Target', 'Local', '-Mode'),
        @('-Scope', 'NotARealScope', '-Target', 'Local', '-Mode', 'Strict'),
        @('-Freeze'),
        @('-PhaseComplete'),
        @('-Scope', 'Freeze', '-Target', 'Local', '-Mode', 'AllowBlocked'),
        @('-Scope', 'Freeze', '-Target', 'WindowsX64', '-Mode', 'Strict'),
        @('-Scope', 'RunnerSelfTest', '-Target', 'WindowsX64', '-Mode', 'Strict'),
        @('-Scope', 'ContractSelfTest', '-Target', 'Local', '-Mode', 'AllowBlocked')
    )
    foreach ($case in $negativeCases) {
        $negative = Invoke-Runner $case
        Assert-Condition ($negative.ExitCode -eq 64) "Invalid parser case returned $($negative.ExitCode): $($case -join ' ')" $Failures
        Assert-Condition ($null -ne $negative.Json -and $negative.Json.outcome -ceq 'usage_error') "Invalid parser case did not emit usage_error: $($case -join ' ')" $Failures
    }

    $allowBlocked = Invoke-Runner @('-Scope', 'ContractSelfTest', '-Target', 'WindowsX64', '-Mode', 'AllowBlocked')
    Assert-Condition ($allowBlocked.ExitCode -eq 0) "AllowBlocked missing-resource case returned $($allowBlocked.ExitCode)" $Failures
    Assert-Condition ($null -ne $allowBlocked.Json -and $allowBlocked.Json.outcome -ceq 'blocked') 'AllowBlocked missing-resource case did not report blocked' $Failures
    Assert-Condition ($null -ne $allowBlocked.Json -and -not [bool]$allowBlocked.Json.strict_gate_eligible) 'AllowBlocked result became strict eligible' $Failures

    $strictBlocked = Invoke-Runner @('-Scope', 'ContractSelfTest', '-Target', 'WindowsX64', '-Mode', 'Strict')
    Assert-Condition ($strictBlocked.ExitCode -eq 3) "Strict missing-resource case returned $($strictBlocked.ExitCode)" $Failures
    Assert-Condition ($null -ne $strictBlocked.Json -and $strictBlocked.Json.outcome -ceq 'blocked') 'Strict missing-resource case did not report blocked' $Failures
    Assert-Condition ($null -ne $strictBlocked.Json -and -not [bool]$strictBlocked.Json.strict_gate_eligible) 'Strict blocked result became strict eligible' $Failures

    return $combinationCount
}

function Test-FreezeAndPhaseComplete {
    param(
        [object]$Matrix,
        [System.Collections.Generic.List[string]]$Failures
    )

    $freezeDispatch = $Matrix.dispatch.'freeze-local'
    $phaseCompleteDispatch = $Matrix.dispatch.'phase-complete-local'
    $expectedFreezeChecks = @(
        'runner-self-test',
        'provenance-self-test',
        'path-smoke-self-test-local',
        'windows-contract-self-test-local',
        'contract-self-test-wsl2',
        'packaging-self-test-local'
    )
    $expectedDeferredEvidence = @(
        'windows-x64-authenticode',
        'windows-arm64-authenticode',
        'macos-intel-developer-id-notarization',
        'macos-apple-silicon-developer-id-notarization'
    )
    $expectedFormalDispatches = @(
        'contract-self-test-windows-x64',
        'contract-self-test-windows-arm64',
        'contract-self-test-mac-intel',
        'contract-self-test-mac-apple-silicon',
        'packaging-self-test-windows-x64',
        'packaging-self-test-windows-arm64',
        'packaging-self-test-mac-intel',
        'packaging-self-test-mac-apple-silicon'
    )

    Assert-Condition ([string]$freezeDispatch.kind -ceq 'composite') 'Freeze dispatch must be composite' $Failures
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $freezeDispatch.required_dispatches) $expectedFreezeChecks) 'Freeze non-signing check set drifted' $Failures
    Assert-Condition ([string]$freezeDispatch.freeze_kind -ceq 'non_signing_contract') 'Freeze kind must be non_signing_contract' $Failures
    Assert-Condition (-not [bool]$freezeDispatch.release_ready) 'Freeze must never report release_ready=true' $Failures
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $freezeDispatch.deferred_evidence) $expectedDeferredEvidence) 'Freeze deferred evidence set drifted' $Failures

    Assert-Condition ([string]$phaseCompleteDispatch.kind -ceq 'phase_complete') 'PhaseComplete must use the dedicated fail-closed dispatch' $Failures
    Assert-Condition (Test-SequenceEquals @(Get-StringArray $phaseCompleteDispatch.formal_evidence_dispatches) $expectedFormalDispatches) 'PhaseComplete formal evidence set drifted' $Failures
    Assert-Condition (-not ((Get-StringArray $phaseCompleteDispatch.formal_evidence_dispatches) -contains 'freeze-local')) 'Freeze result cannot satisfy PhaseComplete formal evidence' $Failures

    $freeze = Invoke-Runner @('-Scope', 'Freeze', '-Target', 'Local', '-Mode', 'Strict')
    Assert-Condition ($freeze.ExitCode -eq 0) "Freeze positive case returned $($freeze.ExitCode)" $Failures
    Assert-Condition ($null -ne $freeze.Json -and $freeze.Json.outcome -ceq 'passed' -and [bool]$freeze.Json.strict_gate_eligible) 'Freeze positive result is not strict eligible' $Failures
    Assert-Condition ($null -ne $freeze.Json -and [string]$freeze.Json.freeze_kind -ceq 'non_signing_contract') 'Freeze result omitted non_signing_contract kind' $Failures
    Assert-Condition ($null -ne $freeze.Json -and -not [bool]$freeze.Json.release_ready) 'Freeze result incorrectly became release ready' $Failures
    Assert-Condition ($null -ne $freeze.Json -and (Test-SequenceEquals @(Get-StringArray $freeze.Json.deferred_evidence) $expectedDeferredEvidence)) 'Freeze result deferred evidence drifted' $Failures

    $phaseComplete = Invoke-Runner @('-Scope', 'PhaseComplete', '-Target', 'Local', '-Mode', 'Strict')
    Assert-Condition ($phaseComplete.ExitCode -eq 3) "PhaseComplete missing-evidence case returned $($phaseComplete.ExitCode)" $Failures
    Assert-Condition ($null -ne $phaseComplete.Json -and $phaseComplete.Json.outcome -ceq 'blocked') 'PhaseComplete missing-evidence case did not report blocked' $Failures
    Assert-Condition ($null -ne $phaseComplete.Json -and -not [bool]$phaseComplete.Json.strict_gate_eligible -and -not [bool]$phaseComplete.Json.release_ready) 'Blocked PhaseComplete became strict or release eligible' $Failures

    $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('gpteasy-freeze-matrix-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    try {
        foreach ($requiredDispatch in $expectedFreezeChecks) {
            $mutated = Copy-JsonObject $Matrix
            $mutated.dispatch.'freeze-local'.required_dispatches = @(
                Get-StringArray $mutated.dispatch.'freeze-local'.required_dispatches |
                    Where-Object { $_ -cne $requiredDispatch }
            )
            $matrixPath = Join-Path $tempRoot ("missing-{0}.json" -f $requiredDispatch)
            Write-Utf8Json $matrixPath $mutated
            $negative = Invoke-Runner @('-Scope', 'RunnerSelfTest', '-Target', 'Local', '-Mode', 'Strict', '-Matrix', $matrixPath)
            Assert-Condition ($negative.ExitCode -eq 2) "Freeze matrix without $requiredDispatch returned $($negative.ExitCode)" $Failures
        }

        $fakeFormal = Copy-JsonObject $Matrix
        $fakeFormal.dispatch.'phase-complete-local'.formal_evidence_dispatches = @('freeze-local')
        $fakeFormalPath = Join-Path $tempRoot 'freeze-as-formal-evidence.json'
        Write-Utf8Json $fakeFormalPath $fakeFormal
        $fakeFormalResult = Invoke-Runner @('-Scope', 'RunnerSelfTest', '-Target', 'Local', '-Mode', 'Strict', '-Matrix', $fakeFormalPath)
        Assert-Condition ($fakeFormalResult.ExitCode -eq 2) "PhaseComplete accepted Freeze as formal evidence with exit $($fakeFormalResult.ExitCode)" $Failures
    } finally {
        if (Test-Path -LiteralPath $tempRoot) {
            Remove-Item -LiteralPath $tempRoot -Recurse -Force
        }
    }
}

function Get-RunnerInvocations {
    param(
        [string]$Content
    )

    $blocks = [regex]::Matches($Content, '(?is)<automated>\s*(.*?)\s*</automated>')
    $invocations = New-Object System.Collections.Generic.List[object]
    foreach ($block in $blocks) {
        $body = $block.Groups[1].Value
        $matches = [regex]::Matches($body, '(?i)run-phase1-contracts\.ps1(?<tail>[^;\r\n<]*)')
        foreach ($match in $matches) {
            $invocations.Add([pscustomobject]@{
                Text = $match.Value.Trim()
                Tail = $match.Groups['tail'].Value
            })
        }
    }
    return $invocations.ToArray()
}

function Scan-PlanConsumers {
    param(
        [string]$PlanDirectory,
        [object]$Matrix
    )

    $errors = New-Object System.Collections.Generic.List[string]
    $invocationCount = 0
    if (-not (Test-Path -LiteralPath $PlanDirectory -PathType Container)) {
        $errors.Add("plan directory does not exist: $PlanDirectory")
        return [pscustomobject]@{ Errors = @($errors); InvocationCount = 0; PlanCount = 0 }
    }

    $plans = @(Get-ChildItem -LiteralPath $PlanDirectory -Filter '*-PLAN.md' -File | Sort-Object Name)
    if ($plans.Count -eq 0) {
        $errors.Add('no PLAN.md files found')
    }

    foreach ($plan in $plans) {
        $content = Get-Content -LiteralPath $plan.FullName -Raw
        foreach ($invocation in @(Get-RunnerInvocations $content)) {
            $invocationCount++
            $tail = $invocation.Tail
            $scopeMatches = @([regex]::Matches($tail, '(?i)-Scope\s+(?<value>[A-Za-z0-9]+)') | ForEach-Object { $_.Groups['value'].Value })
            $targetMatches = @([regex]::Matches($tail, '(?i)-Target\s+(?<value>[A-Za-z0-9]+)') | ForEach-Object { $_.Groups['value'].Value })
            $modeMatches = @([regex]::Matches($tail, '(?i)-Mode\s+(?<value>[A-Za-z0-9]+)') | ForEach-Object { $_.Groups['value'].Value })
            if ($scopeMatches.Count -ne 1 -or $targetMatches.Count -ne 1 -or $modeMatches.Count -ne 1) {
                $errors.Add("$($plan.Name): invocation must contain exactly one -Scope, -Target, and -Mode: $($invocation.Text)")
                continue
            }
            $combination = Find-Combination $Matrix $scopeMatches[0] $targetMatches[0] $modeMatches[0]
            if ($null -eq $combination) {
                $errors.Add("$($plan.Name): undeclared runner combination: $($invocation.Text)")
            }
        }
    }

    return [pscustomobject]@{
        Errors = @($errors)
        InvocationCount = $invocationCount
        PlanCount = $plans.Count
    }
}

function Test-ConsumerNegativeCase {
    param(
        [object]$Matrix
    )

    $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('gpteasy-runner-cli-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    try {
        $badPlan = Join-Path $tempRoot 'bad-PLAN.md'
        $content = @"
<verify><automated>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Freeze</automated></verify>
"@
        [System.IO.File]::WriteAllText($badPlan, $content, (New-Object System.Text.UTF8Encoding($false)))
        return (Scan-PlanConsumers $tempRoot $Matrix)
    } finally {
        if (Test-Path -LiteralPath $tempRoot) {
            Remove-Item -LiteralPath $tempRoot -Recurse -Force
        }
    }
}

$parsed = Parse-Arguments $args
if ($parsed.Errors.Count -gt 0) {
    $parsed.Errors | ForEach-Object { Write-Output ("FAIL: " + [string]$_) }
    exit 64
}

$failures = New-Object System.Collections.Generic.List[string]
try {
    $matrix = Read-Utf8Json (Get-MatrixPath)
    $combinationCount = Test-MatrixAndParser $matrix $failures
    Test-FreezeAndPhaseComplete $matrix $failures
    $scan = Scan-PlanConsumers ([string]$parsed.Values.ScanPlans) $matrix
    foreach ($error in $scan.Errors) {
        $failures.Add($error)
    }
    $negativeScan = Test-ConsumerNegativeCase $matrix
    if ($negativeScan.Errors.Count -eq 0) {
        $failures.Add('negative consumer scan unexpectedly passed an old-style invocation')
    }
} catch {
    $failures.Add(($_ | Out-String).Trim())
}

if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Output ("FAIL: " + [string]$_) }
    exit 1
}

Write-Output ("PASS: validated {0} matrix combinations and scanned {1} plans with {2} runner invocations" -f $combinationCount, $scan.PlanCount, $scan.InvocationCount)
exit 0
