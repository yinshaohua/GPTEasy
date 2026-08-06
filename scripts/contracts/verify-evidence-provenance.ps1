[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Manifest,

    [Parameter(Mandatory = $false)]
    [string]$Transcript,

    [Parameter(Mandatory = $false)]
    [string]$GhExecutable = "gh",

    [Parameter(Mandatory = $false)]
    [ValidateRange(1, 720)]
    [int]$MaxAgeHours = 168,

    [Parameter(Mandatory = $false)]
    [string]$Now
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:Repository = "yinshaohua/GPTEasy"
$script:TempRoot = $null
$script:Checks = New-Object System.Collections.Generic.List[string]
$script:ExitCode = 4
$script:Outcome = "failed"
$script:StrictGateEligible = $false
$script:TestOnly = -not [string]::IsNullOrWhiteSpace($Transcript)

function Throw-Blocked {
    $exception = [System.InvalidOperationException]::new("evidence retrieval blocked")
    $exception.Data["EvidenceKind"] = "blocked"
    throw $exception
}

function Throw-ProvenanceInvalid {
    $exception = [System.InvalidOperationException]::new("evidence provenance invalid")
    $exception.Data["EvidenceKind"] = "invalid"
    throw $exception
}

function Get-RepositoryRoot {
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
}

function Read-JsonFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Throw-ProvenanceInvalid
    }
    try {
        return [System.IO.File]::ReadAllText($Path) | ConvertFrom-Json
    } catch {
        Throw-ProvenanceInvalid
    }
}

function Get-PropertyValue {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ($null -eq $Object) {
        return $null
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Assert-EqualString {
    param(
        [AllowNull()]
        [object]$Actual,

        [AllowNull()]
        [object]$Expected
    )

    if ([string]$Actual -cne [string]$Expected) {
        Throw-ProvenanceInvalid
    }
}

function Assert-EqualInteger {
    param(
        [AllowNull()]
        [object]$Actual,

        [AllowNull()]
        [object]$Expected
    )

    try {
        if ([int64]$Actual -ne [int64]$Expected) {
            Throw-ProvenanceInvalid
        }
    } catch {
        Throw-ProvenanceInvalid
    }
}

function Get-WorkflowIdentity {
    param(
        [Parameter(Mandatory = $true)]
        [string]$WorkflowRef
    )

    $separator = $WorkflowRef.LastIndexOf("@")
    if ($separator -lt 1 -or $separator -eq $WorkflowRef.Length - 1) {
        Throw-ProvenanceInvalid
    }
    $signerWorkflow = $WorkflowRef.Substring(0, $separator)
    $signerDigest = $WorkflowRef.Substring($separator + 1)
    $repositoryPrefix = "$($script:Repository)/"
    if (-not $signerWorkflow.StartsWith($repositoryPrefix, [System.StringComparison]::Ordinal) -or
        $signerDigest -cnotmatch "^[0-9a-f]{40}$") {
        Throw-ProvenanceInvalid
    }

    return [pscustomobject]@{
        SignerWorkflow = $signerWorkflow
        SignerDigest = $signerDigest
        WorkflowPath = $signerWorkflow.Substring($repositoryPrefix.Length)
    }
}

function Invoke-PowerShellScript {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $false)]
        [string[]]$Arguments = @()
    )

    $powershellExecutable = (Get-Command powershell.exe -ErrorAction Stop).Source
    @(& $powershellExecutable -NoProfile -ExecutionPolicy Bypass -File $Path @Arguments 2>$null) | Out-Null
    return [int]$LASTEXITCODE
}

function Invoke-Gh {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    if ($null -eq $script:TempRoot) {
        Throw-Blocked
    }
    $errorPath = Join-Path $script:TempRoot ("gh-error-" + [guid]::NewGuid().ToString("N") + ".txt")
    try {
        $output = @(& $GhExecutable @Arguments 2> $errorPath)
        $exitCode = [int]$LASTEXITCODE
        $errorText = ""
        if (Test-Path -LiteralPath $errorPath -PathType Leaf) {
            $errorText = [System.IO.File]::ReadAllText($errorPath)
        }
        return [pscustomobject]@{
            ExitCode = $exitCode
            Output = ($output -join "`n")
            Error = $errorText
        }
    } catch {
        Throw-Blocked
    } finally {
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-GhJson {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $result = Invoke-Gh -Arguments $Arguments
    if ($result.ExitCode -ne 0) {
        Throw-Blocked
    }
    try {
        return $result.Output | ConvertFrom-Json
    } catch {
        Throw-Blocked
    }
}

function Test-TransientGhFailure {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    return $Text -match "(?i)(HTTP\s+(401|403|429)|not authenticated|timed?\s*out|timeout|network|connection|could not resolve|TLS|rate limit|temporary|unavailable)"
}

function Invoke-Preflight {
    $preflight = Join-Path $PSScriptRoot "preflight-gh-evidence.ps1"
    if (-not (Test-Path -LiteralPath $preflight -PathType Leaf)) {
        Throw-Blocked
    }

    $exitCode = Invoke-PowerShellScript -Path $preflight -Arguments @(
        "-Repository",
        $script:Repository,
        "-MinimumVersion",
        "2.49.0"
    )
    if ($exitCode -ne 0) {
        Throw-Blocked
    }
    $script:Checks.Add("gh_preflight")
}

function Invoke-ManifestValidation {
    $validator = Join-Path $PSScriptRoot "validate-contract-evidence.ps1"
    if (-not (Test-Path -LiteralPath $validator -PathType Leaf)) {
        Throw-ProvenanceInvalid
    }
    $exitCode = Invoke-PowerShellScript -Path $validator -Arguments @("-Manifest", $Manifest)
    if ($exitCode -ne 0) {
        Throw-ProvenanceInvalid
    }
    $script:Checks.Add("manifest_schema")
}

function Get-Sha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Throw-Blocked
    }
    return "sha256:" + ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant())
}

function Find-DownloadedSubject {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Directory,

        [Parameter(Mandatory = $true)]
        [string]$SubjectName
    )

    $matches = @(
        Get-ChildItem -LiteralPath $Directory -Recurse -File |
            Where-Object { $_.Name -ceq $SubjectName }
    )
    if ($matches.Count -ne 1) {
        Throw-ProvenanceInvalid
    }
    return $matches[0].FullName
}

function Read-AttestationSubject {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Json,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedName,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedDigest
    )

    try {
        $parsed = $Json | ConvertFrom-Json
    } catch {
        Throw-ProvenanceInvalid
    }

    foreach ($entry in @($parsed)) {
        $verificationResult = Get-PropertyValue -Object $entry -Name "verificationResult"
        $statement = Get-PropertyValue -Object $entry -Name "statement"
        if ($null -ne $verificationResult) {
            $nestedStatement = Get-PropertyValue -Object $verificationResult -Name "statement"
            if ($null -ne $nestedStatement) {
                $statement = $nestedStatement
            }
        }
        if ($null -eq $statement) {
            continue
        }

        foreach ($subject in @(Get-PropertyValue -Object $statement -Name "subject")) {
            $name = Get-PropertyValue -Object $subject -Name "name"
            $digestObject = Get-PropertyValue -Object $subject -Name "digest"
            $sha256 = Get-PropertyValue -Object $digestObject -Name "sha256"
            if ([string]$name -ceq $ExpectedName -and
                ("sha256:" + [string]$sha256) -ceq $ExpectedDigest) {
                return [pscustomobject]@{
                    Name = [string]$name
                    Digest = "sha256:" + [string]$sha256
                }
            }
        }
    }

    Throw-ProvenanceInvalid
}

function Invoke-AttestationVerification {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SubjectPath,

        [Parameter(Mandatory = $true)]
        [object]$Artifact,

        [Parameter(Mandatory = $true)]
        [object]$WorkflowIdentity,

        [Parameter(Mandatory = $true)]
        [string]$CommitSha
    )

    # Real path: gh attestation verify --repo yinshaohua/GPTEasy with immutable workflow and source digests.
    $arguments = @(
        "attestation",
        "verify",
        $SubjectPath,
        "--repo",
        $script:Repository,
        "--signer-workflow",
        [string]$WorkflowIdentity.SignerWorkflow,
        "--signer-digest",
        [string]$WorkflowIdentity.SignerDigest,
        "--source-digest",
        $CommitSha,
        "--predicate-type",
        [string]$Artifact.attestation.predicate_type,
        "--format",
        "json"
    )
    $result = Invoke-Gh -Arguments $arguments
    if ($result.ExitCode -ne 0) {
        if (Test-TransientGhFailure -Text $result.Error) {
            Throw-Blocked
        }
        Throw-ProvenanceInvalid
    }

    $subject = Read-AttestationSubject `
        -Json $result.Output `
        -ExpectedName ([string]$Artifact.attestation.subject_name) `
        -ExpectedDigest ([string]$Artifact.attestation.subject_digest)

    return [pscustomobject]@{
        role = [string]$Artifact.role
        repository = $script:Repository
        subject_name = [string]$subject.Name
        subject_digest = [string]$subject.Digest
        predicate_type = [string]$Artifact.attestation.predicate_type
        verified = $true
    }
}

function Get-RealRetrieval {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Document
    )

    $provenance = $Document.provenance
    $workflow = Get-WorkflowIdentity -WorkflowRef ([string]$provenance.workflow_ref)
    $runId = [int64]$provenance.run_id
    $runAttempt = [int64]$provenance.run_attempt
    $jobId = [int64]$provenance.job_id

    $run = Invoke-GhJson -Arguments @(
        "api",
        "repos/$($script:Repository)/actions/runs/$runId"
    )
    $jobsResponse = Invoke-GhJson -Arguments @(
        "api",
        "repos/$($script:Repository)/actions/runs/$runId/attempts/$runAttempt/jobs?per_page=100"
    )
    $jobs = @($jobsResponse.jobs | Where-Object { [int64]$_.id -eq $jobId })
    if ($jobs.Count -ne 1) {
        Throw-ProvenanceInvalid
    }
    $job = $jobs[0]

    $retrievedArtifacts = @()
    $attestations = @()
    foreach ($artifact in @($provenance.artifacts)) {
        $artifactId = [int64]$artifact.artifact_id
        $artifactMetadata = Invoke-GhJson -Arguments @(
            "api",
            "repos/$($script:Repository)/actions/artifacts/$artifactId"
        )

        $downloadDirectory = Join-Path $script:TempRoot ([string]$artifact.role)
        New-Item -ItemType Directory -Path $downloadDirectory -Force | Out-Null
        $downloadResult = Invoke-Gh -Arguments @(
            "run",
            "download",
            [string]$runId,
            "-R",
            $script:Repository,
            "-n",
            [string]$artifact.artifact_name,
            "-D",
            $downloadDirectory
        )
        if ($downloadResult.ExitCode -ne 0) {
            Throw-Blocked
        }

        $subjectPath = Find-DownloadedSubject `
            -Directory $downloadDirectory `
            -SubjectName ([string]$artifact.attestation.subject_name)
        $downloadDigest = Get-Sha256 -Path $subjectPath
        $attestation = Invoke-AttestationVerification `
            -SubjectPath $subjectPath `
            -Artifact $artifact `
            -WorkflowIdentity $workflow `
            -CommitSha ([string]$provenance.commit_sha)

        $artifactRun = Get-PropertyValue -Object $artifactMetadata -Name "workflow_run"
        if ($null -ne $artifactRun) {
            Assert-EqualInteger -Actual (Get-PropertyValue -Object $artifactRun -Name "id") -Expected $runId
            Assert-EqualString -Actual (Get-PropertyValue -Object $artifactRun -Name "head_sha") -Expected $provenance.commit_sha
        }

        $retrievedArtifacts += [pscustomobject]@{
            role = [string]$artifact.role
            id = [int64]$artifactMetadata.id
            name = [string]$artifactMetadata.name
            digest = [string]$artifactMetadata.digest
            expired = [bool]$artifactMetadata.expired
            download_sha256 = $downloadDigest
        }
        $attestations += $attestation
    }

    $workflowPath = Get-PropertyValue -Object $run -Name "path"
    if ($null -eq $workflowPath) {
        $workflowPath = [string]$workflow.WorkflowPath
    }
    $normalizedWorkflowRef = "$($script:Repository)/$workflowPath@$($workflow.SignerDigest)"

    return [pscustomobject]@{
        available = $true
        retrieved_at = [DateTimeOffset]::UtcNow.ToString("o")
        run = [pscustomobject]@{
            id = $run.id
            run_attempt = $run.run_attempt
            head_sha = $run.head_sha
            workflow_ref = $normalizedWorkflowRef
            status = $run.status
            conclusion = $run.conclusion
            created_at = $run.created_at
        }
        job = [pscustomobject]@{
            id = $job.id
            run_id = Get-PropertyValue -Object $job -Name "run_id"
            run_attempt = $runAttempt
            head_sha = Get-PropertyValue -Object $job -Name "head_sha"
            workflow_ref = $normalizedWorkflowRef
            conclusion = $job.conclusion
        }
        artifacts = @($retrievedArtifacts)
        attestations = @($attestations)
    }
}

function Assert-RetrievedEvidence {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Document,

        [Parameter(Mandatory = $true)]
        [object]$Retrieval,

        [Parameter(Mandatory = $true)]
        [DateTimeOffset]$ReferenceTime
    )

    if ($null -eq $Retrieval -or
        (Get-PropertyValue -Object $Retrieval -Name "available") -isnot [bool] -or
        -not [bool]$Retrieval.available) {
        Throw-Blocked
    }

    $provenance = $Document.provenance
    Assert-EqualInteger -Actual $Retrieval.run.id -Expected $provenance.run_id
    Assert-EqualInteger -Actual $Retrieval.run.run_attempt -Expected $provenance.run_attempt
    Assert-EqualString -Actual $Retrieval.run.head_sha -Expected $provenance.commit_sha
    Assert-EqualString -Actual $Retrieval.run.workflow_ref -Expected $provenance.workflow_ref
    Assert-EqualString -Actual $Retrieval.run.status -Expected "completed"
    Assert-EqualString -Actual $Retrieval.run.conclusion -Expected "success"

    $createdAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$Retrieval.run.created_at,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::AssumeUniversal,
        [ref]$createdAt
    )) {
        Throw-ProvenanceInvalid
    }
    if ($createdAt -gt $ReferenceTime.AddMinutes(5) -or
        ($ReferenceTime - $createdAt).TotalHours -gt $MaxAgeHours) {
        Throw-ProvenanceInvalid
    }
    $script:Checks.Add("run_identity")

    Assert-EqualInteger -Actual $Retrieval.job.id -Expected $provenance.job_id
    Assert-EqualInteger -Actual $Retrieval.job.run_id -Expected $provenance.run_id
    Assert-EqualInteger -Actual $Retrieval.job.run_attempt -Expected $provenance.run_attempt
    Assert-EqualString -Actual $Retrieval.job.head_sha -Expected $provenance.commit_sha
    Assert-EqualString -Actual $Retrieval.job.workflow_ref -Expected $provenance.workflow_ref
    Assert-EqualString -Actual $Retrieval.job.conclusion -Expected "success"
    $script:Checks.Add("job_identity")

    foreach ($expectedArtifact in @($provenance.artifacts)) {
        $retrievedMatches = @(
            $Retrieval.artifacts |
                Where-Object { [string]$_.role -ceq [string]$expectedArtifact.role }
        )
        if ($retrievedMatches.Count -ne 1) {
            Throw-ProvenanceInvalid
        }
        $retrievedArtifact = $retrievedMatches[0]
        Assert-EqualInteger -Actual $retrievedArtifact.id -Expected $expectedArtifact.artifact_id
        Assert-EqualString -Actual $retrievedArtifact.name -Expected $expectedArtifact.artifact_name
        Assert-EqualString -Actual $retrievedArtifact.digest -Expected $expectedArtifact.artifact_digest
        if ((Get-PropertyValue -Object $retrievedArtifact -Name "expired") -isnot [bool] -or
            [bool]$retrievedArtifact.expired) {
            Throw-Blocked
        }
        Assert-EqualString `
            -Actual $retrievedArtifact.download_sha256 `
            -Expected $expectedArtifact.attestation.subject_digest

        $attestationMatches = @(
            $Retrieval.attestations |
                Where-Object {
                    $null -ne $_ -and
                    [string]$_.role -ceq [string]$expectedArtifact.role
                }
        )
        if ($attestationMatches.Count -ne 1) {
            Throw-ProvenanceInvalid
        }
        $attestation = $attestationMatches[0]
        Assert-EqualString -Actual $attestation.repository -Expected $script:Repository
        Assert-EqualString -Actual $attestation.subject_name -Expected $expectedArtifact.attestation.subject_name
        Assert-EqualString -Actual $attestation.subject_digest -Expected $expectedArtifact.attestation.subject_digest
        Assert-EqualString -Actual $attestation.predicate_type -Expected $expectedArtifact.attestation.predicate_type
        if ((Get-PropertyValue -Object $attestation -Name "verified") -isnot [bool] -or
            -not [bool]$attestation.verified) {
            Throw-ProvenanceInvalid
        }
    }
    $script:Checks.Add("artifact_digests")
    $script:Checks.Add("attestations")
}

try {
    if (-not [string]::IsNullOrWhiteSpace($Now) -and -not $script:TestOnly) {
        Throw-ProvenanceInvalid
    }

    Invoke-ManifestValidation
    $document = Read-JsonFile -Path $Manifest

    $referenceTime = [DateTimeOffset]::UtcNow
    if ($script:TestOnly) {
        if ([string]::IsNullOrWhiteSpace($Now) -or
            -not [DateTimeOffset]::TryParse(
                $Now,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::AssumeUniversal,
                [ref]$referenceTime
            )) {
            Throw-ProvenanceInvalid
        }
        $retrieval = Read-JsonFile -Path $Transcript
    } else {
        $script:TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("gpteasy-provenance-" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $script:TempRoot -Force | Out-Null
        Invoke-Preflight
        $retrieval = Get-RealRetrieval -Document $document
    }

    Assert-RetrievedEvidence -Document $document -Retrieval $retrieval -ReferenceTime $referenceTime
    $script:Outcome = "passed"
    $script:ExitCode = 0
    $script:StrictGateEligible = -not $script:TestOnly
} catch {
    $kind = [string]$_.Exception.Data["EvidenceKind"]
    if ($kind -ceq "blocked") {
        $script:Outcome = "blocked"
        $script:ExitCode = 3
    } else {
        $script:Outcome = "failed"
        $script:ExitCode = 4
    }
    $script:StrictGateEligible = $false
} finally {
    if ($null -ne $script:TempRoot -and
        (Test-Path -LiteralPath $script:TempRoot -PathType Container)) {
        Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

[pscustomobject]@{
    schema_version = 1
    outcome = $script:Outcome
    exit_code = $script:ExitCode
    strict_gate_eligible = $script:StrictGateEligible
    test_only = $script:TestOnly
    checks = @($script:Checks.ToArray())
} | ConvertTo-Json -Compress
exit $script:ExitCode
