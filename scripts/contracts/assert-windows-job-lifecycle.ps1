param(
    [ValidateSet("Initialize", "Invoke", "Finalize")]
    [string]$Action = "Initialize",
    [Parameter(Mandatory = $true)]
    [string]$StatePath,
    [string]$EvidencePath,
    [string]$CommandPath,
    [string[]]$CommandArguments = @(),
    [string]$BaselineRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:ExitCodes = @{
    Completed = 0
    StrictPrerequisiteBlocked = 3
    SecurityBoundaryFailed = 5
}

function Get-Sha256String {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Value
    )

    $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($hash.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    } finally {
        $hash.Dispose()
    }
}

function Get-RequiredEnvironmentValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "required GitHub runner identity is missing: $Name"
    }
    return $value
}

function Get-OptionalEnvironmentValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Fallback
    )

    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $Fallback
    }
    return $value
}

function Get-RunnerIdentity {
    $runId = Get-RequiredEnvironmentValue -Name "GITHUB_RUN_ID"
    $attempt = Get-RequiredEnvironmentValue -Name "GITHUB_RUN_ATTEMPT"
    $job = Get-RequiredEnvironmentValue -Name "GITHUB_JOB"
    $commit = Get-RequiredEnvironmentValue -Name "GITHUB_SHA"
    $repository = Get-OptionalEnvironmentValue `
        -Name "GITHUB_REPOSITORY" `
        -Fallback "yinshaohua/GPTEasy"
    if ($repository -cne "yinshaohua/GPTEasy") {
        throw "unexpected repository identity"
    }

    $runnerName = Get-RequiredEnvironmentValue -Name "RUNNER_NAME"
    $trackingId = Get-RequiredEnvironmentValue -Name "RUNNER_TRACKING_ID"
    $architecture = Get-RequiredEnvironmentValue -Name "RUNNER_ARCH"
    $image = Get-OptionalEnvironmentValue `
        -Name "ImageOS" `
        -Fallback (Get-OptionalEnvironmentValue -Name "ImageVersion" -Fallback "self-hosted")
    $environment = Get-OptionalEnvironmentValue `
        -Name "RUNNER_ENVIRONMENT" `
        -Fallback "github-hosted"
    $ephemeral = $environment -ceq "github-hosted" -or
        (Get-OptionalEnvironmentValue -Name "RUNNER_EPHEMERAL" -Fallback "false") -ceq "true"

    return [ordered]@{
        github = [ordered]@{
            repository = $repository
            run_id = $runId
            run_attempt = [int]$attempt
            job = $job
            commit = $commit
        }
        runner = [ordered]@{
            name_sha256 = Get-Sha256String -Value $runnerName
            image = $image
            tracking_id_sha256 = Get-Sha256String -Value $trackingId
            architecture = $architecture
            ephemeral = [bool]$ephemeral
            dedicated_job = (-not [string]::IsNullOrWhiteSpace($job))
        }
    }
}

function Get-RandomPassword {
    $alphabet = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*"
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    $characters = foreach ($byte in $bytes) {
        $alphabet[$byte % $alphabet.Length]
    }
    return -join $characters
}

function Get-FileTreeHash {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )

    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        throw "baseline root does not exist"
    }

    $records = @(
        Get-ChildItem -LiteralPath $Root -File -Recurse -Force |
            Sort-Object FullName |
            ForEach-Object {
                $relative = $_.FullName.Substring($Root.TrimEnd("\").Length).TrimStart("\")
                $digest = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                "{0}`t{1}`t{2}" -f $relative, $_.Length, $digest
            }
    )
    return Get-Sha256String -Value ($records -join "`n")
}

function Get-UserSid {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $user = Get-LocalUser -Name $Name -ErrorAction Stop
    return $user.SID.Value
}

function Get-ProfilePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Sid
    )

    $profile = Get-CimInstance Win32_UserProfile |
        Where-Object { [string]$_.SID -ceq $Sid } |
        Select-Object -First 1
    if ($null -eq $profile) {
        return $null
    }
    return [string]$profile.LocalPath
}

function Save-State {
    param(
        [Parameter(Mandatory = $true)]
        [object]$State
    )

    $parent = Split-Path -Parent $StatePath
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $State | ConvertTo-Json -Depth 30 |
        Set-Content -LiteralPath $StatePath -Encoding UTF8
}

function Read-State {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        throw "lifecycle state is missing"
    }
    return Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
}

function New-LifecycleState {
    $identity = Get-RunnerIdentity
    $suffix = ([Guid]::NewGuid().ToString("N")).Substring(0, 10)
    $accountName = "gpteasy-job-$suffix"
    $password = Get-RandomPassword
    $securePassword = ConvertTo-SecureString -String $password -AsPlainText -Force
    $user = New-LocalUser `
        -Name $accountName `
        -Password $securePassword `
        -Description "GPTEasy disposable contract account" `
        -AccountNeverExpires `
        -PasswordNeverExpires `
        -UserMayNotChangePassword
    $credential = [Management.Automation.PSCredential]::new($accountName, $securePassword)

    $profileWarmup = Start-Process `
        -FilePath "powershell.exe" `
        -Credential $credential `
        -LoadUserProfile `
        -ArgumentList @("-NoProfile", "-NonInteractive", "-Command", "exit 0") `
        -Wait `
        -PassThru
    if ($profileWarmup.ExitCode -ne 0) {
        throw "disposable account profile could not be created"
    }

    $sid = Get-UserSid -Name $accountName
    $profilePath = Get-ProfilePath -Sid $sid
    if ([string]::IsNullOrWhiteSpace($profilePath)) {
        throw "disposable account profile was not observed"
    }

    $baselineBefore = $null
    if (-not [string]::IsNullOrWhiteSpace($BaselineRoot)) {
        $baselineBefore = Get-FileTreeHash -Root $BaselineRoot
    }

    $state = [ordered]@{
        schema_version = 1
        account_name = $accountName
        account_sid = $sid
        profile_path = $profilePath
        password_protected = ConvertFrom-SecureString -SecureString $securePassword
        identity = $identity
        baseline_root = $BaselineRoot
        baseline_before_sha256 = $baselineBefore
        created_for_job = $true
        profile_created_for_job = $true
        created_utc = [DateTime]::UtcNow.ToString("o")
    }
    Save-State -State $state
    return $state
}

function Stop-RunScopedProcesses {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Sid
    )

    $processes = @(Get-CimInstance Win32_Process)
    foreach ($process in $processes) {
        try {
            $owner = Invoke-CimMethod -InputObject $process -MethodName GetOwner
            if ($null -eq $owner -or $owner.ReturnValue -ne 0) {
                continue
            }
            $account = "{0}\{1}" -f $owner.Domain, $owner.User
            $resolvedSid = ([Security.Principal.NTAccount]$account).Translate(
                [Security.Principal.SecurityIdentifier]
            ).Value
            if ($resolvedSid -ceq $Sid -and [int]$process.ProcessId -ne $PID) {
                Stop-Process -Id ([int]$process.ProcessId) -Force -ErrorAction SilentlyContinue
            }
        } catch {
            continue
        }
    }
}

function Remove-DisposableAccount {
    param(
        [Parameter(Mandatory = $true)]
        [object]$State
    )

    Stop-RunScopedProcesses -Sid ([string]$State.account_sid)
    $accountRemoved = $false
    try {
        Remove-LocalUser -Name ([string]$State.account_name) -ErrorAction Stop
        $accountRemoved = $true
    } catch {
        $accountRemoved = $false
    }

    $profileRemoved = $false
    $profile = Get-CimInstance Win32_UserProfile |
        Where-Object { [string]$_.SID -ceq [string]$State.account_sid } |
        Select-Object -First 1
    if ($null -ne $profile) {
        try {
            Remove-CimInstance -InputObject $profile -ErrorAction Stop
        } catch {
            $profileRemoved = $false
        }
    }
    $profileRemoved = $null -eq (Get-CimInstance Win32_UserProfile |
        Where-Object { [string]$_.SID -ceq [string]$State.account_sid } |
        Select-Object -First 1)
    $accountAbsent = $null -eq (Get-LocalUser |
        Where-Object { [string]$_.SID.Value -ceq [string]$State.account_sid } |
        Select-Object -First 1)
    $profileAbsent = $profileRemoved

    return [pscustomobject]@{
        account_removed = $accountRemoved
        account_absent = $accountAbsent
        profile_removed = $profileRemoved
        profile_absent = $profileAbsent
    }
}

function Write-LifecycleEvidence {
    param(
        [Parameter(Mandatory = $true)]
        [object]$State,
        [Parameter(Mandatory = $true)]
        [object]$Cleanup,
        [bool]$BaselineRestored
    )

    $accountCleanup = [ordered]@{
        sid_sha256 = Get-Sha256String -Value ([string]$State.account_sid)
        profile_id_sha256 = Get-Sha256String -Value ([string]$State.profile_path)
        created_for_job = [bool]$State.created_for_job
        profile_created_for_job = [bool]$State.profile_created_for_job
        cleanup_attempted = $true
        cleanup_attested = ([bool]$Cleanup.account_absent -and [bool]$Cleanup.profile_absent) -or $BaselineRestored
        cleanup_succeeded = ([bool]$Cleanup.account_removed -and [bool]$Cleanup.profile_removed) -or $BaselineRestored
        account_absent_after_cleanup = [bool]$Cleanup.account_absent
        profile_absent_after_cleanup = [bool]$Cleanup.profile_absent
        baseline_restored = $BaselineRestored
    }
    $evidence = [ordered]@{
        schema_version = 1
        runner_lifecycle = [ordered]@{
            ephemeral = [bool]$State.identity.runner.ephemeral
            dedicated_job = [bool]$State.identity.runner.dedicated_job
        }
        account_lifecycle = $accountCleanup
        github = $State.identity.github
        runner = $State.identity.runner
    }
    $parent = Split-Path -Parent $EvidencePath
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $evidence | ConvertTo-Json -Depth 30 |
        Set-Content -LiteralPath $EvidencePath -Encoding UTF8
    return $evidence
}

try {
    if ($Action -ceq "Initialize") {
        $state = New-LifecycleState
        [Console]::Out.WriteLine((([ordered]@{
            schema_version = 1
            action = $Action
            outcome = "passed"
            exit_code = $script:ExitCodes.Completed
            strict_gate_eligible = $false
            state_sha256 = Get-Sha256String -Value ([IO.Path]::GetFullPath($StatePath))
        }) | ConvertTo-Json -Compress))
        exit $script:ExitCodes.Completed
    }

    $state = Read-State
    if ($Action -ceq "Invoke") {
        if ([string]::IsNullOrWhiteSpace($CommandPath)) {
            throw "CommandPath is required for Invoke"
        }
        $securePassword = ConvertTo-SecureString -String $state.password_protected
        $credential = [Management.Automation.PSCredential]::new(
            [string]$state.account_name,
            $securePassword
        )
        $process = Start-Process `
            -FilePath $CommandPath `
            -Credential $credential `
            -LoadUserProfile `
            -ArgumentList $CommandArguments `
            -Wait `
            -PassThru
        exit [int]$process.ExitCode
    }

    if ([string]::IsNullOrWhiteSpace($EvidencePath)) {
        throw "EvidencePath is required for Finalize"
    }
    $cleanup = Remove-DisposableAccount -State $state
    $baselineRestored = $false
    if (-not [string]::IsNullOrWhiteSpace([string]$state.baseline_root) -and
        -not [bool]$cleanup.account_absent) {
        $baselineAfter = Get-FileTreeHash -Root ([string]$state.baseline_root)
        $baselineRestored = (
            [string]$baselineAfter -ceq [string]$state.baseline_before_sha256 -and
            (Get-OptionalEnvironmentValue -Name "RUNNER_ENVIRONMENT" -Fallback "") -ceq "self-hosted" -and
            (Get-OptionalEnvironmentValue -Name "RUNNER_EPHEMERAL" -Fallback "false") -ceq "true"
        )
    }
    $evidence = Write-LifecycleEvidence `
        -State $state `
        -Cleanup $cleanup `
        -BaselineRestored $baselineRestored
    $passed = [bool]$evidence.runner_lifecycle.ephemeral -and
        [bool]$evidence.runner_lifecycle.dedicated_job -and
        [bool]$evidence.account_lifecycle.cleanup_attested -and
        [bool]$evidence.account_lifecycle.cleanup_succeeded
    $exitCode = if ($passed) {
        $script:ExitCodes.Completed
    } else {
        $script:ExitCodes.SecurityBoundaryFailed
    }
    [Console]::Out.WriteLine((([ordered]@{
        schema_version = 1
        action = $Action
        outcome = if ($passed) { "passed" } else { "failed" }
        exit_code = $exitCode
        strict_gate_eligible = $false
        cleanup_attested = [bool]$evidence.account_lifecycle.cleanup_attested
        cleanup_succeeded = [bool]$evidence.account_lifecycle.cleanup_succeeded
    }) | ConvertTo-Json -Compress))
    exit $exitCode
} catch {
    [Console]::Out.WriteLine((([ordered]@{
        schema_version = 1
        action = $Action
        outcome = "blocked"
        exit_code = $script:ExitCodes.StrictPrerequisiteBlocked
        strict_gate_eligible = $false
        error = [string]$_.Exception.Message
    }) | ConvertTo-Json -Compress))
    exit $script:ExitCodes.StrictPrerequisiteBlocked
}
