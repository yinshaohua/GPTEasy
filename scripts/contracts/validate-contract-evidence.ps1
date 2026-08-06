[CmdletBinding(DefaultParameterSetName = "Validate")]
param(
    [Parameter(Mandatory = $true, ParameterSetName = "SelfTest")]
    [switch]$SelfTest,

    [Parameter(Mandatory = $true, ParameterSetName = "Validate")]
    [string]$Manifest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:Repository = "yinshaohua/GPTEasy"
$script:DigestPattern = "^sha256:[0-9a-f]{64}$"
$script:CommitPattern = "^[0-9a-f]{40}$"
$script:WorkflowPattern = "^yinshaohua/GPTEasy/\.github/workflows/[A-Za-z0-9._/-]+\.ya?ml@[0-9a-f]{40}$"
$script:NamePattern = "^[a-z][a-z0-9_]*$"
$script:ArtifactNamePattern = "^[A-Za-z0-9][A-Za-z0-9._-]*$"
$script:RequiredRedactions = @(
    "api_key",
    "authorization",
    "command_line",
    "database_content",
    "raw_app_server_response",
    "raw_config"
)
$script:ForbiddenPropertyNames = @(
    "api_key",
    "artifact_sha256",
    "authorization",
    "command_line",
    "config_toml",
    "database",
    "database_content",
    "experimental_bearer_token",
    "full_command_line",
    "raw_app_server_response",
    "raw_config",
    "raw_response",
    "runner_label",
    "sqlite_dump",
    "strict_gate_eligible",
    "token",
    "verified"
)
$script:ForbiddenValueNeedles = @(
    "GPTEASY-TOKEN-CANARY",
    "Authorization:",
    "Bearer ",
    "experimental_bearer_token",
    "github_pat_",
    "ghp_"
)

function Throw-ValidationFailure {
    throw [System.InvalidOperationException]::new("contract evidence validation failed")
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
        Throw-ValidationFailure
    }

    try {
        return [System.IO.File]::ReadAllText($Path) | ConvertFrom-Json
    } catch {
        Throw-ValidationFailure
    }
}

function Copy-JsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    return (($Value | ConvertTo-Json -Depth 40 -Compress) | ConvertFrom-Json)
}

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    if ($null -eq $Object) {
        Throw-ValidationFailure
    }

    $actual = @($Object.PSObject.Properties | ForEach-Object { $_.Name } | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if (($actual -join "`n") -cne ($expectedSorted -join "`n")) {
        Throw-ValidationFailure
    }
}

function Assert-PositiveInteger {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    if ($Value -isnot [byte] -and
        $Value -isnot [int16] -and
        $Value -isnot [int32] -and
        $Value -isnot [int64]) {
        Throw-ValidationFailure
    }
    if ([int64]$Value -lt 1) {
        Throw-ValidationFailure
    }
}

function Assert-Digest {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value
    )

    if ([string]$Value -cnotmatch $script:DigestPattern) {
        Throw-ValidationFailure
    }
}

function Assert-NoSensitiveData {
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Value
    )

    if ($null -eq $Value) {
        return
    }

    if ($Value -is [string]) {
        foreach ($needle in $script:ForbiddenValueNeedles) {
            if ([string]$Value -match [regex]::Escape($needle)) {
                Throw-ValidationFailure
            }
        }
        return
    }

    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [pscustomobject]) {
        foreach ($item in @($Value)) {
            Assert-NoSensitiveData -Value $item
        }
        return
    }

    foreach ($property in @($Value.PSObject.Properties)) {
        if ([string]$property.Name -in $script:ForbiddenPropertyNames) {
            Throw-ValidationFailure
        }
        Assert-NoSensitiveData -Value $property.Value
    }
}

function Assert-NamedMap {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Map,

        [Parameter(Mandatory = $true)]
        [ValidateSet("Digest", "Boolean", "Count")]
        [string]$ValueType
    )

    $properties = @($Map.PSObject.Properties)
    if ($properties.Count -eq 0) {
        Throw-ValidationFailure
    }

    foreach ($property in $properties) {
        if ([string]$property.Name -cnotmatch $script:NamePattern) {
            Throw-ValidationFailure
        }

        switch ($ValueType) {
            "Digest" {
                Assert-Digest -Value $property.Value
            }
            "Boolean" {
                if ($property.Value -isnot [bool]) {
                    Throw-ValidationFailure
                }
            }
            "Count" {
                if ($property.Value -isnot [byte] -and
                    $property.Value -isnot [int16] -and
                    $property.Value -isnot [int32] -and
                    $property.Value -isnot [int64]) {
                    Throw-ValidationFailure
                }
                if ([int64]$property.Value -lt 0) {
                    Throw-ValidationFailure
                }
            }
        }
    }
}

function Assert-AttestationReference {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Attestation
    )

    Assert-ExactProperties -Object $Attestation -Expected @(
        "subject_name",
        "subject_digest",
        "predicate_type"
    )

    $subjectName = [string]$Attestation.subject_name
    if ([string]::IsNullOrWhiteSpace($subjectName) -or
        $subjectName.Length -gt 256 -or
        $subjectName -match "[`r`n]") {
        Throw-ValidationFailure
    }
    Assert-Digest -Value $Attestation.subject_digest
    if ([string]$Attestation.predicate_type -cne "https://slsa.dev/provenance/v1") {
        Throw-ValidationFailure
    }
}

function Assert-ArtifactReference {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Artifact
    )

    Assert-ExactProperties -Object $Artifact -Expected @(
        "role",
        "artifact_id",
        "artifact_name",
        "artifact_digest",
        "attestation"
    )

    if ([string]$Artifact.role -notin @("evidence_bundle", "subject_artifact")) {
        Throw-ValidationFailure
    }
    Assert-PositiveInteger -Value $Artifact.artifact_id
    $artifactName = [string]$Artifact.artifact_name
    if ([string]::IsNullOrWhiteSpace($artifactName) -or
        $artifactName.Length -gt 128 -or
        $artifactName -cnotmatch $script:ArtifactNamePattern) {
        Throw-ValidationFailure
    }
    Assert-Digest -Value $Artifact.artifact_digest
    Assert-AttestationReference -Attestation $Artifact.attestation
}

function Assert-Provenance {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Provenance
    )

    Assert-ExactProperties -Object $Provenance -Expected @(
        "schema_version",
        "repository",
        "workflow_ref",
        "run_id",
        "run_attempt",
        "job_id",
        "commit_sha",
        "artifacts"
    )

    if ([int]$Provenance.schema_version -ne 1) {
        Throw-ValidationFailure
    }
    if ([string]$Provenance.repository -cne $script:Repository) {
        Throw-ValidationFailure
    }
    if ([string]$Provenance.workflow_ref -cnotmatch $script:WorkflowPattern) {
        Throw-ValidationFailure
    }
    Assert-PositiveInteger -Value $Provenance.run_id
    Assert-PositiveInteger -Value $Provenance.run_attempt
    Assert-PositiveInteger -Value $Provenance.job_id
    if ([string]$Provenance.commit_sha -cnotmatch $script:CommitPattern) {
        Throw-ValidationFailure
    }

    $artifacts = @($Provenance.artifacts)
    if ($artifacts.Count -ne 2) {
        Throw-ValidationFailure
    }

    $roles = @{}
    $artifactIds = @{}
    foreach ($artifact in $artifacts) {
        Assert-ArtifactReference -Artifact $artifact
        $role = [string]$artifact.role
        $artifactId = [string]$artifact.artifact_id
        if ($roles.ContainsKey($role) -or $artifactIds.ContainsKey($artifactId)) {
            Throw-ValidationFailure
        }
        $roles[$role] = $true
        $artifactIds[$artifactId] = $true
    }
    if (-not $roles.ContainsKey("evidence_bundle") -or
        -not $roles.ContainsKey("subject_artifact")) {
        Throw-ValidationFailure
    }
}

function Assert-Lifecycle {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Manifest
    )

    Assert-ExactProperties -Object $Manifest.runner_lifecycle -Expected @(
        "ephemeral",
        "dedicated_job"
    )
    foreach ($name in @("ephemeral", "dedicated_job")) {
        if ($Manifest.runner_lifecycle.$name -isnot [bool] -or
            -not [bool]$Manifest.runner_lifecycle.$name) {
            Throw-ValidationFailure
        }
    }

    Assert-ExactProperties -Object $Manifest.account_lifecycle -Expected @(
        "created_for_job",
        "profile_created_for_job",
        "cleanup_attempted",
        "cleanup_succeeded",
        "baseline_restored"
    )
    foreach ($name in @(
        "created_for_job",
        "profile_created_for_job",
        "cleanup_attempted",
        "cleanup_succeeded",
        "baseline_restored"
    )) {
        if ($Manifest.account_lifecycle.$name -isnot [bool] -or
            -not [bool]$Manifest.account_lifecycle.$name) {
            Throw-ValidationFailure
        }
    }
}

function Assert-Manifest {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Document
    )

    Assert-NoSensitiveData -Value $Document
    Assert-ExactProperties -Object $Document -Expected @(
        "schema_version",
        "contract_name",
        "observed_version",
        "evidence_level",
        "captured_at",
        "origin_type",
        "summary_digests",
        "assertions",
        "counts",
        "redactions",
        "runner_lifecycle",
        "account_lifecycle",
        "provenance"
    )

    if ([int]$Document.schema_version -ne 1) {
        Throw-ValidationFailure
    }
    $contractName = [string]$Document.contract_name
    if ([string]::IsNullOrWhiteSpace($contractName) -or
        $contractName.Length -gt 128 -or
        $contractName -cnotmatch "^[a-z0-9]+(?:[._-][a-z0-9]+)*$") {
        Throw-ValidationFailure
    }
    $observedVersion = [string]$Document.observed_version
    if ([string]::IsNullOrWhiteSpace($observedVersion) -or
        $observedVersion.Length -gt 128 -or
        $observedVersion -match "[`r`n]") {
        Throw-ValidationFailure
    }
    if ([string]$Document.evidence_level -cne "native") {
        Throw-ValidationFailure
    }
    if ([string]$Document.origin_type -cne "github-actions") {
        Throw-ValidationFailure
    }

    $capturedAt = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string]$Document.captured_at,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::AssumeUniversal,
        [ref]$capturedAt
    )) {
        Throw-ValidationFailure
    }

    Assert-NamedMap -Map $Document.summary_digests -ValueType "Digest"
    Assert-NamedMap -Map $Document.assertions -ValueType "Boolean"
    foreach ($assertion in @($Document.assertions.PSObject.Properties)) {
        if (-not [bool]$assertion.Value) {
            Throw-ValidationFailure
        }
    }
    Assert-NamedMap -Map $Document.counts -ValueType "Count"

    $redactions = @($Document.redactions | ForEach-Object { [string]$_ } | Sort-Object)
    $requiredRedactions = @($script:RequiredRedactions | Sort-Object)
    if (($redactions -join "`n") -cne ($requiredRedactions -join "`n")) {
        Throw-ValidationFailure
    }

    Assert-Lifecycle -Manifest $Document
    Assert-Provenance -Provenance $Document.provenance

    return [pscustomobject]@{
        SchemaValid = $true
        StrictGateEligible = $false
    }
}

function Assert-SchemaContracts {
    $root = Get-RepositoryRoot
    $manifestSchema = Read-JsonFile -Path (Join-Path $root "tests\fixtures\contracts\schema\contract-manifest.schema.json")
    $provenanceSchema = Read-JsonFile -Path (Join-Path $root "tests\fixtures\contracts\schema\provenance.schema.json")

    if ([string]$manifestSchema.'$schema' -cne "https://json-schema.org/draft/2020-12/schema" -or
        [bool]$manifestSchema.additionalProperties -or
        [string]$manifestSchema.properties.provenance.'$ref' -cne "provenance.schema.json") {
        Throw-ValidationFailure
    }
    if ([string]$provenanceSchema.'$schema' -cne "https://json-schema.org/draft/2020-12/schema" -or
        [bool]$provenanceSchema.additionalProperties -or
        [string]$provenanceSchema.properties.workflow_ref.pattern -cne $script:WorkflowPattern -or
        [string]$provenanceSchema.'$defs'.sha256.pattern -cne $script:DigestPattern) {
        Throw-ValidationFailure
    }
}

function Get-PathParent {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Root,

        [Parameter(Mandatory = $true)]
        [string[]]$Segments
    )

    $current = $Root
    for ($index = 0; $index -lt $Segments.Count - 1; $index++) {
        $segment = $Segments[$index]
        if ($segment -match "^\d+$") {
            $current = @($current)[[int]$segment]
        } else {
            $property = $current.PSObject.Properties[$segment]
            if ($null -eq $property) {
                Throw-ValidationFailure
            }
            $current = $property.Value
        }
    }

    return [pscustomobject]@{
        Parent = $current
        Leaf = $Segments[$Segments.Count - 1]
    }
}

function Apply-Mutation {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Root,

        [Parameter(Mandatory = $true)]
        [object]$Case
    )

    $segments = @(([string]$Case.path).Split("."))
    $location = Get-PathParent -Root $Root -Segments $segments
    $leaf = [string]$location.Leaf
    $operation = [string]$Case.operation

    if ($leaf -match "^\d+$") {
        Throw-ValidationFailure
    }

    if ($operation -eq "remove") {
        $location.Parent.PSObject.Properties.Remove($leaf)
        return
    }
    if ($operation -eq "add") {
        $location.Parent | Add-Member -NotePropertyName $leaf -NotePropertyValue $Case.value
        return
    }
    if ($operation -in @("set", "replace")) {
        $property = $location.Parent.PSObject.Properties[$leaf]
        if ($null -eq $property) {
            Throw-ValidationFailure
        }
        $property.Value = $Case.value
        return
    }

    Throw-ValidationFailure
}

function Invoke-SelfTest {
    Assert-SchemaContracts

    $root = Get-RepositoryRoot
    $fixture = Read-JsonFile -Path (Join-Path $root "tests\fixtures\contracts\provenance-negative-cases.json")
    Assert-ExactProperties -Object $fixture -Expected @("schema_version", "positive_control", "cases")
    if ([int]$fixture.schema_version -ne 1) {
        Throw-ValidationFailure
    }

    $positive = Copy-JsonObject -Value $fixture.positive_control.manifest
    $positiveResult = Assert-Manifest -Document $positive
    if (-not [bool]$positiveResult.SchemaValid -or [bool]$positiveResult.StrictGateEligible) {
        Throw-ValidationFailure
    }

    $validationCases = @($fixture.cases | Where-Object { [string]$_.stage -ceq "validation" })
    if ($validationCases.Count -lt 8) {
        Throw-ValidationFailure
    }

    foreach ($case in $validationCases) {
        $mutatedRoot = [pscustomobject]@{
            manifest = Copy-JsonObject -Value $fixture.positive_control.manifest
        }
        Apply-Mutation -Root $mutatedRoot -Case $case

        $rejected = $false
        try {
            Assert-Manifest -Document $mutatedRoot.manifest | Out-Null
        } catch {
            $rejected = $true
        }
        if (-not $rejected) {
            Throw-ValidationFailure
        }
    }

    Write-Output ("contract evidence validator self-test passed: schemas, positive control, and {0} fail-closed cases" -f $validationCases.Count)
}

try {
    if ($SelfTest) {
        Invoke-SelfTest
    } else {
        $document = Read-JsonFile -Path $Manifest
        $result = Assert-Manifest -Document $document
        [pscustomobject]@{
            schema_version = 1
            outcome = "validated"
            strict_gate_eligible = [bool]$result.StrictGateEligible
        } | ConvertTo-Json -Compress
    }
    exit 0
} catch {
    Write-Output "contract evidence validation failed; untrusted or sensitive evidence is not emitted."
    exit 4
}
