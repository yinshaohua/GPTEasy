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
        [object[]]$Checks = @(),
        [bool]$TestOnly = $false,
        [bool]$ReleaseReady = $false,
        [AllowNull()]
        [string]$FreezeKind = $null,
        [string[]]$DeferredEvidence = @()
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
        test_only = $TestOnly
        freeze_kind = $FreezeKind
        release_ready = $ReleaseReady
        deferred_evidence = @($DeferredEvidence)
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
    param(
        [AllowNull()]
        [string]$OverridePath
    )

    $path = Get-MatrixPath
    if (-not [string]::IsNullOrWhiteSpace($OverridePath)) {
        if (-not (Test-Path -LiteralPath $OverridePath -PathType Leaf)) {
            throw "test-only matrix does not exist: $OverridePath"
        }
        $path = (Resolve-Path -LiteralPath $OverridePath).Path
    }
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
    $dispatchKinds = @('internal', 'command', 'aggregate', 'evidence_set', 'process', 'composite', 'phase_complete')
    foreach ($dispatchName in $dispatchNames) {
        $dispatch = $Matrix.dispatch.$dispatchName
        if ([string]$dispatch.kind -notin $dispatchKinds) {
            $errors.Add("dispatch has unknown kind: $dispatchName")
        }
        if ([string]$dispatch.kind -eq 'evidence_set') {
            if ([string]::IsNullOrWhiteSpace([string]$dispatch.validator_path)) {
                $errors.Add("evidence_set has no validator_path: $dispatchName")
            }
            if ([string]$dispatch.provenance_validator_path -cne 'scripts/contracts/verify-evidence-provenance.ps1') {
                $errors.Add("evidence_set has no canonical provenance validator: $dispatchName")
            }
            $requiredPaths = @(Get-StringArray $dispatch.required_paths)
            if ($requiredPaths.Count -eq 0) {
                $errors.Add("evidence_set has no required_paths: $dispatchName")
            }
        }
        if ([string]$dispatch.kind -eq 'process' -and
            [string]::IsNullOrWhiteSpace([string]$dispatch.executable)) {
            $errors.Add("process has no executable: $dispatchName")
        }
    }
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

    $freezeDispatch = $Matrix.dispatch.'freeze-local'
    $expectedFreezeChecks = @(
        'runner-self-test',
        'provenance-self-test',
        'path-smoke-self-test-local',
        'windows-contract-self-test-local',
        'contract-self-test-wsl2',
        'packaging-self-test-local'
    )
    if ($null -eq $freezeDispatch -or [string]$freezeDispatch.kind -cne 'composite') {
        $errors.Add('freeze-local must use composite dispatch')
    } else {
        if (-not (Test-SequenceEquals @(Get-StringArray $freezeDispatch.required_dispatches) $expectedFreezeChecks)) {
            $errors.Add('freeze-local required_dispatches drifted from the non-signing contract')
        }
        if ([string]$freezeDispatch.freeze_kind -cne 'non_signing_contract') {
            $errors.Add('freeze-local freeze_kind must be non_signing_contract')
        }
        if ([bool]$freezeDispatch.release_ready) {
            $errors.Add('freeze-local release_ready must remain false')
        }
        $expectedDeferred = @(
            'windows-x64-authenticode',
            'windows-arm64-authenticode',
            'macos-intel-developer-id-notarization',
            'macos-apple-silicon-developer-id-notarization'
        )
        if (-not (Test-SequenceEquals @(Get-StringArray $freezeDispatch.deferred_evidence) $expectedDeferred)) {
            $errors.Add('freeze-local deferred_evidence drifted')
        }
    }

    $phaseCompleteDispatch = $Matrix.dispatch.'phase-complete-local'
    if ($null -eq $phaseCompleteDispatch -or [string]$phaseCompleteDispatch.kind -cne 'phase_complete') {
        $errors.Add('phase-complete-local must use phase_complete dispatch')
    } else {
        $formal = @(Get-StringArray $phaseCompleteDispatch.formal_evidence_dispatches)
        $expectedFormal = @(
            'contract-self-test-windows-x64',
            'contract-self-test-windows-arm64',
            'contract-self-test-mac-intel',
            'contract-self-test-mac-apple-silicon',
            'packaging-self-test-windows-x64',
            'packaging-self-test-windows-arm64',
            'packaging-self-test-mac-intel',
            'packaging-self-test-mac-apple-silicon'
        )
        if (-not (Test-SequenceEquals $formal $expectedFormal)) {
            $errors.Add('phase-complete-local formal_evidence_dispatches drifted')
        }
        if ($formal -contains 'freeze-local') {
            $errors.Add('freeze-local cannot satisfy formal PhaseComplete evidence')
        }
        $required = @(Get-StringArray $phaseCompleteDispatch.required_dispatches)
        foreach ($formalDispatch in $formal) {
            if ($formalDispatch -notin $required) {
                $errors.Add("formal evidence dispatch is not required by PhaseComplete: $formalDispatch")
            }
            if ($formalDispatch -notin $dispatchNames -or
                [string]$Matrix.dispatch.$formalDispatch.kind -cne 'evidence_set') {
                $errors.Add("formal evidence dispatch must be an evidence_set: $formalDispatch")
            }
        }
    }

    foreach ($dispatchName in $dispatchNames) {
        $dispatch = $Matrix.dispatch.$dispatchName
        if ([string]$dispatch.kind -in @('composite', 'phase_complete')) {
            $childDispatches = @(Get-StringArray $dispatch.required_dispatches)
            if ([string]$dispatch.kind -ceq 'phase_complete') {
                $childDispatches += @(Get-StringArray $dispatch.formal_evidence_dispatches)
            }
            foreach ($childName in $childDispatches) {
                if ($childName -notin $dispatchNames) {
                    $errors.Add("dispatch references undeclared child: $dispatchName -> $childName")
                }
            }
        }
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

function Resolve-RepositoryFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepositoryRoot,
        [Parameter(Mandatory = $true)]
        [string]$RelativePath
    )

    $root = [System.IO.Path]::GetFullPath($RepositoryRoot)
    while ($root.EndsWith('\') -or $root.EndsWith('/')) {
        $root = $root.Substring(0, $root.Length - 1)
    }
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $root ($RelativePath -replace '/', '\')))
    $prefix = $root + [System.IO.Path]::DirectorySeparatorChar
    if (-not $candidate.Equals($root, [System.StringComparison]::OrdinalIgnoreCase) -and
        -not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "matrix path escapes repository: $RelativePath"
    }
    return $candidate
}

function Get-PathCheckResult {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    $missing = @(
        @(Get-StringArray $Dispatch.required_paths) |
            Where-Object {
                $path = Resolve-RepositoryFile -RepositoryRoot $RepositoryRoot -RelativePath $_
                -not (Test-Path -LiteralPath $path -PathType Leaf)
            }
    )
    if ($missing.Count -gt 0) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = "required paths do not exist: $($missing -join ', ')"
            Checks = @()
        }
    }
    return $null
}

function Invoke-CommandDispatch {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    foreach ($command in @($Dispatch.commands)) {
        $path = Resolve-RepositoryFile -RepositoryRoot $RepositoryRoot -RelativePath ([string]$command.path)
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            return [pscustomobject]@{
                Outcome = 'blocked'
                ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
                StrictGateEligible = $false
                Reason = "required command does not exist: $($command.path)"
            }
        }

        $arguments = @('-NoProfile', '-File', $path) + @(Get-StringArray $command.arguments)
        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $commandOutput = @(& powershell @arguments 2>&1 | ForEach-Object { [string]$_ })
        } finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
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
        Checks = @()
    }
}

function Invoke-AggregateDispatch {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    $pathResult = Get-PathCheckResult -Dispatch $Dispatch -RepositoryRoot $RepositoryRoot
    if ($null -ne $pathResult) {
        return $pathResult
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
        Checks = @()
    }
}

function Invoke-ProcessDispatch {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    $pathResult = Get-PathCheckResult -Dispatch $Dispatch -RepositoryRoot $RepositoryRoot
    if ($null -ne $pathResult) {
        return $pathResult
    }

    $command = Get-Command ([string]$Dispatch.executable) -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = "required executable does not exist: $($Dispatch.executable)"
            Checks = @()
        }
    }

    $arguments = @(Get-StringArray $Dispatch.arguments)
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $processOutput = @(& $command.Source @arguments 2>&1 | ForEach-Object { [string]$_ })
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $childExitCode = Get-ChildExitCode $LASTEXITCODE
    if ($childExitCode -ne 0) {
        $outcome = if ($childExitCode -eq $script:ExitCodes.StrictPrerequisiteBlocked) { 'blocked' } else { 'failed' }
        return [pscustomobject]@{
            Outcome = $outcome
            ExitCode = $childExitCode
            StrictGateEligible = $false
            Reason = "downstream process failed: $($Dispatch.executable) $($arguments -join ' ')"
            Checks = @()
        }
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
        Checks = @()
    }
}

function Invoke-EvidenceSetDispatch {
    param(
        [object]$Dispatch,
        [string]$RepositoryRoot
    )

    $pathResult = Get-PathCheckResult -Dispatch $Dispatch -RepositoryRoot $RepositoryRoot
    if ($null -ne $pathResult) {
        return $pathResult
    }

    $validatorPath = Resolve-RepositoryFile -RepositoryRoot $RepositoryRoot -RelativePath ([string]$Dispatch.validator_path)
    if (-not (Test-Path -LiteralPath $validatorPath -PathType Leaf)) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = "required evidence validator does not exist: $($Dispatch.validator_path)"
            Checks = @()
        }
    }
    $provenanceValidatorPath = Resolve-RepositoryFile `
        -RepositoryRoot $RepositoryRoot `
        -RelativePath ([string]$Dispatch.provenance_validator_path)
    if (-not (Test-Path -LiteralPath $provenanceValidatorPath -PathType Leaf)) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = "required provenance validator does not exist: $($Dispatch.provenance_validator_path)"
            Checks = @()
        }
    }

    $checks = New-Object System.Collections.Generic.List[object]
    foreach ($relativePath in @(Get-StringArray $Dispatch.required_paths)) {
        $manifestPath = Resolve-RepositoryFile -RepositoryRoot $RepositoryRoot -RelativePath $relativePath
        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $validatorOutput = @(
                & powershell -NoProfile -File $validatorPath -Manifest $manifestPath 2>&1 |
                    ForEach-Object { [string]$_ }
            )
        } finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
        $childExitCode = Get-ChildExitCode $LASTEXITCODE
        $checks.Add([ordered]@{
            dispatch = [string]$Dispatch.validator_path
            path = $relativePath
            outcome = if ($childExitCode -eq 0) { 'passed' } else { 'failed' }
            exit_code = $childExitCode
        })
        if ($childExitCode -ne 0) {
            $outcome = if ($childExitCode -eq $script:ExitCodes.StrictPrerequisiteBlocked) { 'blocked' } else { 'failed' }
            return [pscustomobject]@{
                Outcome = $outcome
                ExitCode = $childExitCode
                StrictGateEligible = $false
                Reason = "formal evidence failed validation: $relativePath"
                Checks = $checks.ToArray()
            }
        }

        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $provenanceOutput = @(
                & powershell -NoProfile -File $provenanceValidatorPath -Manifest $manifestPath 2>&1 |
                    ForEach-Object { [string]$_ }
            )
        } finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
        $provenanceExitCode = Get-ChildExitCode $LASTEXITCODE
        $provenanceDocument = $null
        $provenanceJson = @(
            $provenanceOutput |
                Where-Object { $_.TrimStart().StartsWith('{') } |
                Select-Object -Last 1
        )
        if ($provenanceJson.Count -gt 0) {
            try {
                $provenanceDocument = $provenanceJson[0] | ConvertFrom-Json
            } catch {
                $provenanceDocument = $null
            }
        }
        $provenancePassed = (
            $provenanceExitCode -eq $script:ExitCodes.Completed -and
            $null -ne $provenanceDocument -and
            [string]$provenanceDocument.outcome -ceq 'passed' -and
            [bool]$provenanceDocument.strict_gate_eligible -and
            -not [bool]$provenanceDocument.test_only
        )
        $checks.Add([ordered]@{
            dispatch = [string]$Dispatch.provenance_validator_path
            path = $relativePath
            outcome = if ($provenancePassed) { 'passed' } else { 'failed' }
            exit_code = $provenanceExitCode
        })
        if (-not $provenancePassed) {
            $outcome = if ($provenanceExitCode -eq $script:ExitCodes.StrictPrerequisiteBlocked) { 'blocked' } else { 'failed' }
            $exitCode = if ($provenanceExitCode -eq 0) { $script:ExitCodes.ProvenanceInvalid } else { $provenanceExitCode }
            return [pscustomobject]@{
                Outcome = $outcome
                ExitCode = $exitCode
                StrictGateEligible = $false
                Reason = "formal evidence provenance is not strict eligible: $relativePath"
                Checks = $checks.ToArray()
            }
        }
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
        Checks = $checks.ToArray()
    }
}

function Invoke-DispatchByName {
    param(
        [object]$Matrix,
        [string]$DispatchName,
        [string]$Mode,
        [string]$RepositoryRoot,
        [System.Collections.Generic.HashSet[string]]$Active = $null
    )

    if ($null -eq $Active) {
        $Active = New-Object System.Collections.Generic.HashSet[string]
    }
    if (-not $Active.Add($DispatchName)) {
        return [pscustomobject]@{
            Outcome = 'failed'
            ExitCode = $script:ExitCodes.AssertionFailed
            StrictGateEligible = $false
            Reason = "dispatch cycle detected: $DispatchName"
            Checks = @()
        }
    }

    $dispatch = $Matrix.dispatch.$DispatchName
    try {
        switch ([string]$dispatch.kind) {
            'internal' { return Invoke-RunnerSelfTest -Matrix $Matrix }
            'command' { return Invoke-CommandDispatch -Dispatch $dispatch -RepositoryRoot $RepositoryRoot }
            'aggregate' { return Invoke-AggregateDispatch -Dispatch $dispatch -RepositoryRoot $RepositoryRoot }
            'evidence_set' { return Invoke-EvidenceSetDispatch -Dispatch $dispatch -RepositoryRoot $RepositoryRoot }
            'process' { return Invoke-ProcessDispatch -Dispatch $dispatch -RepositoryRoot $RepositoryRoot }
            'composite' {
                $checks = New-Object System.Collections.Generic.List[object]
                foreach ($childName in @(Get-StringArray $dispatch.required_dispatches)) {
                    $child = Invoke-DispatchByName -Matrix $Matrix -DispatchName $childName -Mode $Mode -RepositoryRoot $RepositoryRoot -Active $Active
                    $checks.Add([ordered]@{
                        dispatch = $childName
                        outcome = [string]$child.Outcome
                        exit_code = [int]$child.ExitCode
                        strict_gate_eligible = [bool]$child.StrictGateEligible
                    })
                    if ([string]$child.Outcome -ne 'passed') {
                        return [pscustomobject]@{
                            Outcome = [string]$child.Outcome
                            ExitCode = [int]$child.ExitCode
                            StrictGateEligible = $false
                            Reason = "composite child failed: $childName"
                            Checks = $checks.ToArray()
                        }
                    }
                }
                return [pscustomobject]@{
                    Outcome = 'passed'
                    ExitCode = $script:ExitCodes.Completed
                    StrictGateEligible = $true
                    Reason = $null
                    Checks = $checks.ToArray()
                    FreezeKind = [string]$dispatch.freeze_kind
                    ReleaseReady = [bool]$dispatch.release_ready
                    DeferredEvidence = @(Get-StringArray $dispatch.deferred_evidence)
                }
            }
            'phase_complete' {
                return Invoke-PhaseCompleteDispatch -Matrix $Matrix -Dispatch $dispatch -Mode $Mode -RepositoryRoot $RepositoryRoot -Active $Active
            }
            default {
                return [pscustomobject]@{
                    Outcome = 'failed'
                    ExitCode = $script:ExitCodes.AssertionFailed
                    StrictGateEligible = $false
                    Reason = "unknown dispatch kind: $($dispatch.kind)"
                    Checks = @()
                }
            }
        }
    } finally {
        $null = $Active.Remove($DispatchName)
    }
}

function Invoke-PhaseCompleteDispatch {
    param(
        [object]$Matrix,
        [object]$Dispatch,
        [string]$Mode,
        [string]$RepositoryRoot,
        [System.Collections.Generic.HashSet[string]]$Active
    )

    $checks = New-Object System.Collections.Generic.List[object]
    foreach ($formalName in @(Get-StringArray $Dispatch.formal_evidence_dispatches)) {
        $formal = Invoke-DispatchByName -Matrix $Matrix -DispatchName $formalName -Mode 'Strict' -RepositoryRoot $RepositoryRoot -Active $Active
        $checks.Add([ordered]@{
            dispatch = $formalName
            outcome = [string]$formal.Outcome
            exit_code = [int]$formal.ExitCode
            strict_gate_eligible = [bool]$formal.StrictGateEligible
        })
        if ([string]$formal.Outcome -ne 'passed') {
            return [pscustomobject]@{
                Outcome = [string]$formal.Outcome
                ExitCode = [int]$formal.ExitCode
                StrictGateEligible = $false
                Reason = "formal evidence missing or invalid: $formalName"
                Checks = $checks.ToArray()
                ReleaseReady = $false
            }
        }
    }

    foreach ($requiredName in @(Get-StringArray $Dispatch.required_dispatches)) {
        $required = Invoke-DispatchByName -Matrix $Matrix -DispatchName $requiredName -Mode 'Strict' -RepositoryRoot $RepositoryRoot -Active $Active
        $checks.Add([ordered]@{
            dispatch = $requiredName
            outcome = [string]$required.Outcome
            exit_code = [int]$required.ExitCode
            strict_gate_eligible = [bool]$required.StrictGateEligible
        })
        if ([string]$required.Outcome -ne 'passed') {
            return [pscustomobject]@{
                Outcome = [string]$required.Outcome
                ExitCode = [int]$required.ExitCode
                StrictGateEligible = $false
                Reason = "PhaseComplete prerequisite failed: $requiredName"
                Checks = $checks.ToArray()
                ReleaseReady = $false
            }
        }
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
        Checks = $checks.ToArray()
        ReleaseReady = $true
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
        Checks = @()
    }
}

function Invoke-PhaseCompleteSourceAudit {
    param(
        [string]$RepositoryRoot
    )

    $auditRelativePath = 'scripts/contracts/audit-phase1-plan-source.ps1'
    $auditPath = Join-Path $RepositoryRoot ($auditRelativePath -replace '/', '\')
    $phaseDir = Join-Path $RepositoryRoot '.planning/phases/01-trusted-local-state-contract'
    if (-not (Test-Path -LiteralPath $auditPath -PathType Leaf)) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = "required command does not exist: $auditRelativePath"
            Checks = @()
        }
    }
    if (-not (Test-Path -LiteralPath $phaseDir -PathType Container)) {
        return [pscustomobject]@{
            Outcome = 'blocked'
            ExitCode = $script:ExitCodes.StrictPrerequisiteBlocked
            StrictGateEligible = $false
            Reason = 'required phase directory does not exist: .planning/phases/01-trusted-local-state-contract'
            Checks = @()
        }
    }

    $auditOutput = @(
        & powershell -NoProfile -File $auditPath -PhaseDir $phaseDir -ReadOnly 2>&1 |
            ForEach-Object { [string]$_ }
    )
    $childExitCode = Get-ChildExitCode $LASTEXITCODE
    if ($childExitCode -ne 0) {
        $reason = "phase source audit failed: $auditRelativePath"
        $detail = ($auditOutput -join ' ').Trim()
        if (-not [string]::IsNullOrWhiteSpace($detail)) {
            $reason = "$reason - $detail"
        }
        return [pscustomobject]@{
            Outcome = 'failed'
            ExitCode = $childExitCode
            StrictGateEligible = $false
            Reason = $reason
            Checks = @()
        }
    }

    return [pscustomobject]@{
        Outcome = 'passed'
        ExitCode = $script:ExitCodes.Completed
        StrictGateEligible = $true
        Reason = $null
        Checks = @()
    }
}

$parsed = Parse-Arguments $args
$isTestOnly = -not [string]::IsNullOrWhiteSpace([string]$parsed.Values.Matrix)
if ($parsed.Errors.Count -gt 0) {
    $result = New-Result `
        -Scope $parsed.Values.Scope `
        -Target $parsed.Values.Target `
        -Mode $parsed.Values.Mode `
        -Dispatch $null `
        -Outcome 'usage_error' `
        -ExitCode $script:ExitCodes.UsageOrCombinationError `
        -StrictGateEligible $false `
        -BlockingReasons $parsed.Errors `
        -TestOnly $isTestOnly
    Write-ResultAndExit $result $script:ExitCodes.UsageOrCombinationError
}

try {
    $matrix = Read-Matrix -OverridePath $parsed.Values.Matrix
    Assert-Matrix $matrix
} catch {
    $result = New-Result `
        -Scope $parsed.Values.Scope `
        -Target $parsed.Values.Target `
        -Mode $parsed.Values.Mode `
        -Dispatch $null `
        -Outcome 'failed' `
        -ExitCode $script:ExitCodes.AssertionFailed `
        -StrictGateEligible $false `
        -BlockingReasons @($_.Exception.Message) `
        -TestOnly $isTestOnly
    Write-ResultAndExit $result $script:ExitCodes.AssertionFailed
}

$scope = [string]$parsed.Values.Scope
$target = [string]$parsed.Values.Target
$mode = [string]$parsed.Values.Mode
if ($scope -notin (Get-StringArray $matrix.cli.parameters.Scope) -or
    $target -notin (Get-StringArray $matrix.cli.parameters.Target) -or
    $mode -notin (Get-StringArray $matrix.cli.parameters.Mode)) {
    $result = New-Result `
        -Scope $scope -Target $target -Mode $mode -Dispatch $null `
        -Outcome 'usage_error' -ExitCode $script:ExitCodes.UsageOrCombinationError `
        -StrictGateEligible $false -BlockingReasons @('Scope, Target, or Mode is undeclared') `
        -TestOnly $isTestOnly
    Write-ResultAndExit $result $script:ExitCodes.UsageOrCombinationError
}

$combination = Find-Combination $matrix $scope $target $mode
if ($null -eq $combination) {
    $result = New-Result `
        -Scope $scope -Target $target -Mode $mode -Dispatch $null `
        -Outcome 'usage_error' -ExitCode $script:ExitCodes.UsageOrCombinationError `
        -StrictGateEligible $false -BlockingReasons @('Scope/Target/Mode combination is undeclared') `
        -TestOnly $isTestOnly
    Write-ResultAndExit $result $script:ExitCodes.UsageOrCombinationError
}

try {
    $repositoryRoot = Get-RepositoryRoot
    if ($scope -eq 'PhaseComplete') {
        $sourceAuditResult = Invoke-PhaseCompleteSourceAudit -RepositoryRoot $repositoryRoot
        $sourceAuditCheck = [ordered]@{
            dispatch = 'source-audit'
            outcome = [string]$sourceAuditResult.Outcome
            exit_code = [int]$sourceAuditResult.ExitCode
            strict_gate_eligible = [bool]$sourceAuditResult.StrictGateEligible
        }
        if ([int]$sourceAuditResult.ExitCode -ne $script:ExitCodes.Completed) {
            $dispatchResult = $sourceAuditResult
            $dispatchResult.Checks = @($sourceAuditCheck)
        } else {
            $dispatchResult = Invoke-DispatchByName `
                -Matrix $matrix `
                -DispatchName ([string]$combination.dispatch) `
                -Mode $mode `
                -RepositoryRoot $repositoryRoot
            $dispatchResult.Checks = @($sourceAuditCheck) + @($dispatchResult.Checks)
        }
    } else {
        $dispatchResult = Invoke-DispatchByName `
            -Matrix $matrix `
            -DispatchName ([string]$combination.dispatch) `
            -Mode $mode `
            -RepositoryRoot $repositoryRoot
    }
} catch {
    $dispatchResult = [pscustomobject]@{
        Outcome = 'failed'
        ExitCode = $script:ExitCodes.AssertionFailed
        StrictGateEligible = $false
        Reason = $_.Exception.Message
        Checks = @()
    }
}

$exitCode = [int]$dispatchResult.ExitCode
$strictEligible = [bool]$dispatchResult.StrictGateEligible
$outcome = [string]$dispatchResult.Outcome
$reasons = @()
if ($null -ne $dispatchResult.Reason -and [string]$dispatchResult.Reason -ne '') {
    $reasons = @([string]$dispatchResult.Reason)
}
$checks = @()
if ($null -ne $dispatchResult.PSObject.Properties['Checks']) {
    $checks = @($dispatchResult.Checks)
}
$freezeKind = $null
if ($null -ne $dispatchResult.PSObject.Properties['FreezeKind']) {
    $freezeKind = [string]$dispatchResult.FreezeKind
}
$deferredEvidence = @()
if ($null -ne $dispatchResult.PSObject.Properties['DeferredEvidence']) {
    $deferredEvidence = @(Get-StringArray $dispatchResult.DeferredEvidence)
}
$releaseReady = $false
if ($null -ne $dispatchResult.PSObject.Properties['ReleaseReady']) {
    $releaseReady = [bool]$dispatchResult.ReleaseReady
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

if ($isTestOnly) {
    $strictEligible = $false
    $releaseReady = $false
}

$result = New-Result `
    -Scope $scope `
    -Target $target `
    -Mode $mode `
    -Dispatch $combination.dispatch `
    -Outcome $outcome `
    -ExitCode $exitCode `
    -StrictGateEligible $strictEligible `
    -BlockingReasons $reasons `
    -Checks $checks `
    -TestOnly $isTestOnly `
    -ReleaseReady $releaseReady `
    -FreezeKind $freezeKind `
    -DeferredEvidence $deferredEvidence
Write-ResultAndExit $result $exitCode
