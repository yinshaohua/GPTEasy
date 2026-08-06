$ErrorActionPreference = 'Stop'

Set-StrictMode -Version Latest

$script:ExitCodes = @{
    Completed = 0
    AssertionFailed = 2
    StrictPrerequisiteBlocked = 3
    ProvenanceInvalid = 4
    SecurityBoundaryFailed = 5
    UsageOrCombinationError = 64
}

function Get-RepositoryRoot {
    $root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
    return $root.Path
}

function Get-MatrixPath {
    return (Join-Path (Get-RepositoryRoot) 'tests\fixtures\contracts\runner-cli-matrix.json')
}

function New-Result {
    param(
        [AllowNull()]
        [string]$Scope,
        [AllowNull()]
        [string]$Target,
        [AllowNull()]
        [string]$Mode,
        [AllowNull()]
        [string]$Dispatch,
        [string]$Outcome,
        [int]$ExitCode,
        [bool]$StrictGateEligible,
        [string[]]$BlockingReasons = @(),
        [object[]]$Checks = @()
    )

    return [ordered]@{
        schema_version = 1
        phase = '01'
        scope = $Scope
        target = $Target
        mode = $Mode
        dispatch = $Dispatch
        outcome = $Outcome
        exit_code = $ExitCode
        strict_gate_eligible = $StrictGateEligible
        blocking_reasons = @($BlockingReasons)
        checks = @($Checks)
    }
}

function Write-ResultAndExit {
    param(
        [hashtable]$Result,
        [int]$ExitCode
    )

    $Result.exit_code = $ExitCode
    [Console]::Out.WriteLine(($Result | ConvertTo-Json -Compress -Depth 20))
    exit $ExitCode
}

function Parse-Arguments {
    param(
        [string[]]$Arguments
    )

    $values = @{
        Scope = $null
        Target = $null
        Mode = $null
        Matrix = $null
    }
    $errors = New-Object System.Collections.Generic.List[string]
    $known = @('-Scope', '-Target', '-Mode', '-Matrix')

    for ($index = 0; $index -lt $Arguments.Count; $index++) {
        $argument = $Arguments[$index]
        if ($argument -notin $known) {
            $errors.Add("unknown parameter: $argument")
            continue
        }

        if ($index + 1 -ge $Arguments.Count -or $Arguments[$index + 1].StartsWith('-')) {
            $errors.Add("parameter is missing a value: $argument")
            continue
        }

        $name = $argument.TrimStart('-')
        if ($null -ne $values[$name]) {
            $errors.Add("duplicate parameter: $argument")
        } else {
            $values[$name] = $Arguments[$index + 1]
        }
        $index++
    }

    foreach ($required in @('Scope', 'Target', 'Mode')) {
        if ([string]::IsNullOrWhiteSpace([string]$values[$required])) {
            $errors.Add("missing required parameter: -$required")
        }
    }

    return [pscustomobject]@{
        Values = $values
        Errors = @($errors)
    }
}

function Read-Matrix {
    $path = Get-MatrixPath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "runner matrix does not exist: $path"
    }

    return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
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

function Assert-Matrix {
    param(
        [object]$Matrix
    )

    $errors = New-Object System.Collections.Generic.List[string]
    $expectedScopes = @('RunnerSelfTest', 'ProvenanceSelfTest', 'ContractSelfTest', 'PackagingSelfTest', 'Freeze', 'PhaseComplete')
    $expectedTargets = @('Local', 'WindowsX64', 'WindowsArm64', 'MacIntel', 'MacAppleSilicon', 'Wsl2')
    $expectedModes = @('Strict', 'AllowBlocked')
    $actualScopes = Get-StringArray $Matrix.cli.parameters.Scope
    $actualTargets = Get-StringArray $Matrix.cli.parameters.Target
    $actualModes = Get-StringArray $Matrix.cli.parameters.Mode

    if ([int]$Matrix.schema_version -ne 1) {
        $errors.Add('matrix schema_version must be 1')
    }
    if ([string]$Matrix.phase -ne '01') {
        $errors.Add('matrix phase must be 01')
    }
    if (-not (Test-SequenceEquals @($actualScopes) @($expectedScopes))) {
        $errors.Add('Scope declaration set does not match the contract')
    }
    if (-not (Test-SequenceEquals @($actualTargets) @($expectedTargets))) {
        $errors.Add('Target declaration set does not match the contract')
    }
    if (-not (Test-SequenceEquals @($actualModes) @($expectedModes))) {
        $errors.Add('Mode declaration set does not match the contract')
    }
    if (-not (Test-SequenceEquals @(Get-StringArray $Matrix.cli.required_parameters) @('Scope', 'Target', 'Mode'))) {
        $errors.Add('required_parameters must declare Scope, Target, Mode in order')
    }

    $exitCodeKeys = @($Matrix.exit_codes.psobject.Properties.Name | ForEach-Object { [int]$_ } | Sort-Object)
    if (-not (Test-SequenceEquals @($exitCodeKeys) @(0, 2, 3, 4, 5, 64))) {
        $errors.Add('exit_codes must declare 0, 2, 3, 4, 5, 64')
    }

    $dispatchNames = @($Matrix.dispatch.psobject.Properties.Name)
    $combinationKeys = New-Object System.Collections.Generic.HashSet[string]
    foreach ($combination in @($Matrix.combinations)) {
        $key = '{0}|{1}' -f $combination.scope, $combination.target
        if (-not $combinationKeys.Add($key)) {
            $errors.Add("duplicate Scope/Target combination: $key")
        }
        if ($combination.scope -notin $actualScopes) {
            $errors.Add("combination uses undeclared Scope: $($combination.scope)")
        }
        if ($combination.target -notin $actualTargets) {
            $errors.Add("combination uses undeclared Target: $($combination.target)")
        }
        if ([string]::IsNullOrWhiteSpace([string]$combination.dispatch) -or $combination.dispatch -notin $dispatchNames) {
            $errors.Add("combination has no unique dispatch: $key")
        }
        $modes = @(Get-StringArray $combination.modes)
        $invalidModes = @($modes | Where-Object { $_ -notin $actualModes })
        if ($modes.Count -eq 0 -or $invalidModes.Count -gt 0) {
            $errors.Add("combination contains undeclared Mode: $key")
        }
        if (($combination.scope -in @('Freeze', 'PhaseComplete')) -and
            ($combination.target -ne 'Local' -or -not ($modes -contains 'Strict') -or $modes.Count -ne 1)) {
            $errors.Add("$($combination.scope) must allow only Local+Strict")
        }
        if (($combination.scope -in @('RunnerSelfTest', 'ProvenanceSelfTest')) -and
            ($combination.target -ne 'Local' -or $modes.Count -ne 1 -or $modes[0] -ne 'Strict')) {
            $errors.Add("$($combination.scope) must allow only Local+Strict")
        }
        if (($modes -contains 'AllowBlocked') -and
            ($combination.scope -notin @('ContractSelfTest', 'PackagingSelfTest') -or $combination.target -eq 'Local')) {
            $errors.Add("AllowBlocked is only valid for concrete ContractSelfTest/PackagingSelfTest targets: $key")
        }
    }

    $allowBlockedScopes = Get-StringArray $Matrix.allow_blocked.scopes
    $allowBlockedTargets = Get-StringArray $Matrix.allow_blocked.targets
    if (-not (Test-SequenceEquals @($allowBlockedScopes) @('ContractSelfTest', 'PackagingSelfTest'))) {
        $errors.Add('allow_blocked.scopes does not match the contract')
    }
    if (-not (Test-SequenceEquals @($allowBlockedTargets) @('WindowsX64', 'WindowsArm64', 'MacIntel', 'MacAppleSilicon', 'Wsl2'))) {
        $errors.Add('allow_blocked.targets does not match the contract')
    }
    if ([int]$Matrix.allow_blocked.exit_code -ne 0 -or
        [string]$Matrix.allow_blocked.outcome -cne 'blocked' -or
        [bool]$Matrix.allow_blocked.strict_gate_eligible) {
        $errors.Add('AllowBlocked blocked result must be exit 0, outcome=blocked, strict_gate_eligible=false')
    }

    if ($errors.Count -gt 0) {
        throw ($errors -join '; ')
    }
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

function Get-ChildExitCode {
    param(
        [int]$ExitCode
    )

    if ($ExitCode -in @(0, 2, 3, 4, 5, 64)) {
        return $ExitCode
    }

    return $script:ExitCodes.AssertionFailed
}

function Invoke-CommandDispatch {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    foreach ($command in @($Dispatch.commands)) {
        $path = Join-Path $RepositoryRoot ([string]$command.path -replace '/', '\')
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            return [pscustomobject]@{
                Outcome = 'blocked'
                ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
                StrictGateEligible = $false
                Reason = "required command does not exist: $($command.path)"
            }
        }

        $arguments = @('-NoProfile', '-File', $path) + @(Get-StringArray $command.arguments)
        & powershell @arguments | Out-Null
        $childExitCode = Get-ChildExitCode $LASTEXITCODE
        if ($childExitCode -ne 0) {
            $outcome = if ($childExitCode -eq $script:ExitCodes.StrictPrerequisiteBlocked) { 'blocked' } else { 'failed' }
            return [pscustomobject]@{
                Outcome = $outcome
                ExitCode = $childExitCode
                StrictGateEligible = $false
                Reason = "downstream command failed: $($command.path)"
            }
        }
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
    }
}

function Invoke-AggregateDispatch {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    $missing = @(
        @(Get-StringArray $Dispatch.required_paths) |
            Where-Object {
                $path = Join-Path $RepositoryRoot ($_ -replace '/', '\')
                -not (Test-Path -LiteralPath $path -PathType Leaf)
            }
    )
    if ($missing.Count -gt 0) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = "required paths do not exist: $($missing -join ', ')"
        }
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
    }
}

function Invoke-RunnerSelfTest {
    param(
        [object]$Matrix
    )

    Assert-Matrix $Matrix
    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
    }
}

function Invoke-Dispatch {
    param(
        [object]$Matrix,
        [object]$Combination,
        [string]$Mode
    )

    $dispatch = $Matrix.dispatch.($Combination.dispatch)
    $repositoryRoot = Get-RepositoryRoot
    if ($dispatch.kind -eq 'internal') {
        return Invoke-RunnerSelfTest $Matrix
    }
    if ($dispatch.kind -eq 'command') {
        return Invoke-CommandDispatch $dispatch $repositoryRoot
    }
    if ($dispatch.kind -eq 'aggregate') {
        return Invoke-AggregateDispatch $dispatch $repositoryRoot
    }

    return [pscustomobject]@{
        Outcome = 'failed'
        ExitCode = $script:ExitCodes.AssertionFailed
        StrictGateEligible = $false
        Reason = "unknown dispatch kind: $($dispatch.kind)"
    }
}

$parsed = Parse-Arguments $args
if ($parsed.Errors.Count -gt 0) {
    $result = New-Result $parsed.Values.Scope $parsed.Values.Target $parsed.Values.Mode $null 'usage_error' $script:ExitCodes.UsageOrCombinationError $false $parsed.Errors
    Write-ResultAndExit $result $script:ExitCodes.UsageOrCombinationError
}

try {
    $matrix = Read-Matrix
    Assert-Matrix $matrix
} catch {
    $result = New-Result $parsed.Values.Scope $parsed.Values.Target $parsed.Values.Mode $null 'failed' $script:ExitCodes.AssertionFailed $false @($_.Exception.Message)
    Write-ResultAndExit $result $script:ExitCodes.AssertionFailed
}

$scope = [string]$parsed.Values.Scope
$target = [string]$parsed.Values.Target
$mode = [string]$parsed.Values.Mode
if ($scope -notin (Get-StringArray $matrix.cli.parameters.Scope) -or
    $target -notin (Get-StringArray $matrix.cli.parameters.Target) -or
    $mode -notin (Get-StringArray $matrix.cli.parameters.Mode)) {
    $result = New-Result $scope $target $mode $null 'usage_error' $script:ExitCodes.UsageOrCombinationError $false @('Scope, Target, or Mode is undeclared')
    Write-ResultAndExit $result $script:ExitCodes.UsageOrCombinationError
}

$combination = Find-Combination $matrix $scope $target $mode
if ($null -eq $combination) {
    $result = New-Result $scope $target $mode $null 'usage_error' $script:ExitCodes.UsageOrCombinationError $false @('Scope/Target/Mode combination is undeclared')
    Write-ResultAndExit $result $script:ExitCodes.UsageOrCombinationError
}

try {
    $dispatchResult = Invoke-Dispatch $matrix $combination $mode
} catch {
    $dispatchResult = [pscustomobject]@{
        Outcome = 'failed'
        ExitCode = $script:ExitCodes.AssertionFailed
        StrictGateEligible = $false
        Reason = $_.Exception.Message
    }
}

$exitCode = [int]$dispatchResult.ExitCode
$strictEligible = [bool]$dispatchResult.StrictGateEligible
$outcome = [string]$dispatchResult.Outcome
$reasons = @()
if ($null -ne $dispatchResult.Reason -and [string]$dispatchResult.Reason -ne '') {
    $reasons = @([string]$dispatchResult.Reason)
}

if ($mode -eq 'AllowBlocked' -and $outcome -eq 'blocked') {
    $exitCode = $script:ExitCodes.Completed
    $strictEligible = $false
}
if ($outcome -eq 'blocked') {
    $strictEligible = $false
}
if ($mode -eq 'Strict' -and $outcome -eq 'blocked' -and $exitCode -eq $script:ExitCodes.Completed) {
    $exitCode = $script:ExitCodes.StrictPrerequisiteBlocked
}

$result = New-Result $scope $target $mode $combination.dispatch $outcome $exitCode $strictEligible $reasons
Write-ResultAndExit $result $exitCode
