param(
    [ValidateSet("All", "Workflow")]
    [string]$CaseSet = "All"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:ExitCodes = @{
    Completed = 0
    AssertionFailed = 2
    SecurityBoundaryFailed = 5
}

function Get-RepositoryRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
}

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-PowerShellJson {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ScriptPath,
        [string[]]$Arguments = @()
    )

    $output = @(
        & powershell -NoProfile -File $ScriptPath @Arguments 2>&1 |
            ForEach-Object { [string]$_ }
    )
    $exitCode = $LASTEXITCODE
    $jsonLine = @($output | Where-Object { $_.TrimStart().StartsWith("{") }) |
        Select-Object -Last 1
    $document = $null
    if (-not [string]::IsNullOrWhiteSpace([string]$jsonLine)) {
        $document = $jsonLine | ConvertFrom-Json
    }

    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = @($output)
        Document = $document
    }
}

function Copy-JsonDocument {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Document
    )

    return (($Document | ConvertTo-Json -Depth 30) | ConvertFrom-Json)
}

function Write-CaseFixture {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Document,
        [Parameter(Mandatory = $true)]
        [string]$Directory,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $path = Join-Path $Directory "$Name.json"
    $json = $Document | ConvertTo-Json -Depth 30
    [IO.File]::WriteAllText(
        $path,
        ($json + "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    return $path
}

function Assert-PredicateResult {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VerifierPath,
        [Parameter(Mandatory = $true)]
        [string]$FixturePath,
        [Parameter(Mandatory = $true)]
        [bool]$ShouldPass,
        [Parameter(Mandatory = $true)]
        [string]$CaseName,
        [string]$ExpectedReason
    )

    $result = Invoke-PowerShellJson `
        -ScriptPath $VerifierPath `
        -Arguments @("-FixturePath", $FixturePath)

    Assert-True `
        -Condition ($null -ne $result.Document) `
        -Message "$CaseName did not emit a JSON result: $($result.Output -join ' | ')"

    if ($ShouldPass) {
        Assert-True `
            -Condition ($result.ExitCode -eq $script:ExitCodes.Completed) `
            -Message "$CaseName should pass but exited $($result.ExitCode)"
        Assert-True `
            -Condition ([string]$result.Document.outcome -ceq "passed") `
            -Message "$CaseName should report outcome=passed"
        Assert-True `
            -Condition ([bool]$result.Document.test_only) `
            -Message "$CaseName must be marked test_only=true"
        Assert-True `
            -Condition (-not [bool]$result.Document.strict_gate_eligible) `
            -Message "$CaseName fixture must never become strict-gate eligible"
        return
    }

    Assert-True `
        -Condition ($result.ExitCode -eq $script:ExitCodes.SecurityBoundaryFailed) `
        -Message "$CaseName should fail closed with exit 5 but exited $($result.ExitCode)"
    Assert-True `
        -Condition ([string]$result.Document.outcome -ceq "failed") `
        -Message "$CaseName should report outcome=failed"
    Assert-True `
        -Condition (-not [bool]$result.Document.strict_gate_eligible) `
        -Message "$CaseName failure must not be strict-gate eligible"
    Assert-True `
        -Condition ($ExpectedReason -in @($result.Document.blocking_reasons)) `
        -Message "$CaseName should report $ExpectedReason"
}

function Invoke-PredicateCases {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VerifierPath,
        [Parameter(Mandatory = $true)]
        [string]$PositiveFixturePath
    )

    $positive = Get-Content -LiteralPath $PositiveFixturePath -Raw | ConvertFrom-Json
    $temporaryDirectory = Join-Path `
        ([IO.Path]::GetTempPath()) `
        ("gpteasy-windows-package-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

    try {
        Assert-PredicateResult `
            -VerifierPath $VerifierPath `
            -FixturePath $PositiveFixturePath `
            -ShouldPass $true `
            -CaseName "positive-control"

        $cases = @(
            [pscustomobject]@{
                Name = "unsigned"
                Reason = "WINDOWS_PACKAGE_UNSIGNED"
                Mutate = {
                    param($case)
                    $case.package.authenticode.status = "NotSigned"
                }
            },
            [pscustomobject]@{
                Name = "wrong-arch"
                Reason = "WINDOWS_PACKAGE_ARCH_MISMATCH"
                Mutate = {
                    param($case)
                    $case.package.pe.machine = "ARM64"
                }
            },
            [pscustomobject]@{
                Name = "system-install"
                Reason = "WINDOWS_PACKAGE_NOT_CURRENT_USER"
                Mutate = {
                    param($case)
                    $case.package.install.scope = "perMachine"
                    $case.package.install.root_kind = "ProgramFiles"
                }
            },
            [pscustomobject]@{
                Name = "marker-only"
                Reason = "WINDOWS_LIFECYCLE_NOT_ATTESTED"
                Mutate = {
                    param($case)
                    $case.lifecycle.account_lifecycle.created_for_job = $false
                    $case.lifecycle.account_lifecycle.profile_created_for_job = $false
                    $case.lifecycle.account_lifecycle.cleanup_attested = $false
                    $case.lifecycle.account_lifecycle.cleanup_succeeded = $false
                    $case.lifecycle.account_lifecycle.account_absent_after_cleanup = $false
                    $case.lifecycle.account_lifecycle.profile_absent_after_cleanup = $false
                    $case.package.marker_correlated = $true
                }
            },
            [pscustomobject]@{
                Name = "cleanup-missing"
                Reason = "WINDOWS_LIFECYCLE_CLEANUP_MISSING"
                Mutate = {
                    param($case)
                    $case.lifecycle.account_lifecycle.cleanup_attempted = $false
                    $case.lifecycle.account_lifecycle.cleanup_attested = $false
                    $case.lifecycle.account_lifecycle.cleanup_succeeded = $false
                    $case.lifecycle.account_lifecycle.account_absent_after_cleanup = $false
                    $case.lifecycle.account_lifecycle.profile_absent_after_cleanup = $false
                }
            }
        )

        foreach ($definition in $cases) {
            $case = Copy-JsonDocument -Document $positive
            & $definition.Mutate $case
            $path = Write-CaseFixture `
                -Document $case `
                -Directory $temporaryDirectory `
                -Name $definition.Name
            Assert-PredicateResult `
                -VerifierPath $VerifierPath `
                -FixturePath $path `
                -ShouldPass $false `
                -CaseName $definition.Name `
                -ExpectedReason $definition.Reason
        }
    } finally {
        if (Test-Path -LiteralPath $temporaryDirectory) {
            Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
        }
    }
}

function Assert-SourceContract {
    param(
        [Parameter(Mandatory = $true)]
        [string]$LifecyclePath,
        [Parameter(Mandatory = $true)]
        [string]$VerifierPath,
        [Parameter(Mandatory = $true)]
        [string]$WorkflowPath
    )

    $lifecycleSource = Get-Content -LiteralPath $LifecyclePath -Raw
    foreach ($pattern in @(
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "GITHUB_JOB",
        "RUNNER_NAME",
        "RUNNER_TRACKING_ID",
        "RUNNER_ARCH",
        "New-LocalUser",
        "Start-Process",
        "-Credential",
        "-LoadUserProfile",
        "Stop-Process",
        "Remove-LocalUser",
        "Win32_UserProfile",
        "Remove-CimInstance",
        "RUNNER_ENVIRONMENT",
        "baseline",
        "cleanup_attested"
    )) {
        Assert-True `
            -Condition ($lifecycleSource -match [regex]::Escape($pattern)) `
            -Message "lifecycle guard is missing required source contract: $pattern"
    }

    $verifierSource = Get-Content -LiteralPath $VerifierPath -Raw
    foreach ($pattern in @(
        "Get-AuthenticodeSignature",
        "AMD64",
        "ARM64",
        "LOCALAPPDATA",
        "currentUser",
        "marker_correlated",
        "strict_gate_eligible",
        "fixture_mode"
    )) {
        Assert-True `
            -Condition ($verifierSource -match [regex]::Escape($pattern)) `
            -Message "package verifier is missing required source contract: $pattern"
    }

    if (-not (Test-Path -LiteralPath $WorkflowPath -PathType Leaf)) {
        return
    }

    $workflowSource = Get-Content -LiteralPath $WorkflowPath -Raw
    foreach ($pattern in @(
        "windows-x64",
        "windows-arm64",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
        "ref: `${{ github.sha }}",
        "persist-credentials: false",
        "assert-windows-job-lifecycle.ps1",
        "-Action Initialize",
        "-Action Invoke",
        "-Action Finalize",
        "probe-windows-host.ps1",
        "probe-wsl2.ps1",
        "verify-windows-package.ps1",
        "preflight-gh-evidence.ps1",
        "WINDOWS_AUTHENTICODE_PFX_BASE64",
        "certificateThumbprint",
        "Get-AuthenticodeSignature",
        "phase1-path-smoke",
        "strict-pass.json",
        "if: always()",
        "upload-artifact",
        "attest-build-provenance",
        "cleanup_attested"
    )) {
        Assert-True `
            -Condition ($workflowSource -match [regex]::Escape($pattern)) `
            -Message "Windows evidence workflow is missing required contract: $pattern"
    }

    Assert-True `
        -Condition ($workflowSource -notmatch '(?m)^\s+GPTEASY_(?:PRIVATE|EVIDENCE)_DIR:\s*\$\{\{\s*runner\.temp\s*\}\}') `
        -Message "runner.temp must not be evaluated from job-level env"
    foreach ($pattern in @(
        '$env:RUNNER_TEMP',
        'GPTEASY_PRIVATE_DIR=$privateDirectory',
        'GPTEASY_EVIDENCE_DIR=$evidenceDirectory',
        '$env:GITHUB_ENV'
    )) {
        Assert-True `
            -Condition ($workflowSource -match [regex]::Escape($pattern)) `
            -Message "Windows evidence workflow is missing runner-time temp initialization: $pattern"
    }

    $finalizeOffset = $workflowSource.IndexOf("-Action Finalize")
    $verifyOffset = $workflowSource.LastIndexOf("verify-windows-package.ps1")
    $uploadOffset = $workflowSource.IndexOf("upload-artifact")
    $attestOffset = $workflowSource.IndexOf("attest-build-provenance")
    Assert-True `
        -Condition ($finalizeOffset -ge 0 -and $verifyOffset -gt $finalizeOffset) `
        -Message "package predicate must consume finalized lifecycle evidence"
    Assert-True `
        -Condition ($uploadOffset -gt $verifyOffset -and $attestOffset -gt $uploadOffset) `
        -Message "artifact upload and provenance attestation must follow package and lifecycle verification"

    $actionReferences = @(
        [regex]::Matches($workflowSource, "(?m)^\s*uses:\s*(?<reference>[^\s]+)\s*$") |
            ForEach-Object { $_.Groups["reference"].Value }
    )
    Assert-True `
        -Condition (
            $actionReferences.Count -gt 0 -and
            @($actionReferences | Where-Object { $_ -notmatch "@[0-9a-f]{40}$" }).Count -eq 0
        ) `
        -Message "every GitHub Action must be pinned to an immutable commit"
}

$repositoryRoot = Get-RepositoryRoot
$lifecyclePath = Join-Path $repositoryRoot "scripts\contracts\assert-windows-job-lifecycle.ps1"
$verifierPath = Join-Path $repositoryRoot "scripts\contracts\verify-windows-package.ps1"
$positiveFixturePath = Join-Path `
    $repositoryRoot `
    "tests\fixtures\contracts\packaging\windows-positive-control.json"
$workflowPath = Join-Path $repositoryRoot ".github\workflows\phase1-windows-evidence.yml"

try {
    foreach ($requiredPath in @(
        $lifecyclePath,
        $verifierPath,
        $positiveFixturePath
    )) {
        Assert-True `
            -Condition (Test-Path -LiteralPath $requiredPath -PathType Leaf) `
            -Message "required Windows package contract artifact is missing: $requiredPath"
    }

    Assert-SourceContract `
        -LifecyclePath $lifecyclePath `
        -VerifierPath $verifierPath `
        -WorkflowPath $workflowPath

    if ($CaseSet -ceq "All") {
        Invoke-PredicateCases `
            -VerifierPath $verifierPath `
            -PositiveFixturePath $positiveFixturePath
    }

    [Console]::Out.WriteLine((([ordered]@{
        schema_version = 1
        probe = "windows-package-verifier-self-test"
        case_set = $CaseSet
        outcome = "passed"
        exit_code = $script:ExitCodes.Completed
        strict_gate_eligible = $false
        test_only = $true
    }) | ConvertTo-Json -Compress))
    exit $script:ExitCodes.Completed
} catch {
    [Console]::Out.WriteLine((([ordered]@{
        schema_version = 1
        probe = "windows-package-verifier-self-test"
        case_set = $CaseSet
        outcome = "failed"
        exit_code = $script:ExitCodes.AssertionFailed
        strict_gate_eligible = $false
        test_only = $true
        error = [string]$_.Exception.Message
    }) | ConvertTo-Json -Compress))
    exit $script:ExitCodes.AssertionFailed
}
