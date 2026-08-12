[CmdletBinding()]
param(
    [string]$InstallerPath,
    [string]$CandidateManifestPath,
    [string]$SecretPath,
    [switch]$ConfirmDisposableEnvironment,
    [switch]$RequireAuthenticode
)

$ErrorActionPreference = 'Stop'

if (-not $ConfirmDisposableEnvironment) {
    throw 'Windows UAT 要求在执行任何可变更检查前传入 -ConfirmDisposableEnvironment。'
}
if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
    throw '必须提供 InstallerPath。'
}
if ([string]::IsNullOrWhiteSpace($CandidateManifestPath)) {
    throw '必须提供 CandidateManifestPath。'
}
if ([string]::IsNullOrWhiteSpace($SecretPath)) {
    $SecretPath = Join-Path $PSScriptRoot '..\.codex\skills\spike-findings-gpteasy\.secrets\provider.json'
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Get-Sha256Text([string]$Value) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Value)
        return ([System.BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
}

function Get-Sha256File([string]$Path) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
        $stream.Dispose()
        $sha256.Dispose()
    }
}

function Get-FileSignature([string]$Path) {
    if ($PSVersionTable.PSEdition -eq 'Desktop') {
        $module = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1'
        Import-Module -Name $module -Force -ErrorAction Stop
    } else {
        Import-Module -Name Microsoft.PowerShell.Security -ErrorAction Stop
    }
    return Get-AuthenticodeSignature -LiteralPath $Path
}

function Test-FileContainsBytes([string]$Path, [byte[]]$Needle) {
    $haystack = [System.IO.File]::ReadAllBytes($Path)
    if ($Needle.Length -eq 0 -or $haystack.Length -lt $Needle.Length) {
        return $false
    }
    for ($offset = 0; $offset -le $haystack.Length - $Needle.Length; $offset++) {
        $matches = $true
        for ($index = 0; $index -lt $Needle.Length; $index++) {
            if ($haystack[$offset + $index] -ne $Needle[$index]) {
                $matches = $false
                break
            }
        }
        if ($matches) {
            return $true
        }
    }
    return $false
}

function Get-InstalledRoots {
    return @(
        (Join-Path $env:LOCALAPPDATA 'Programs\GPTEasy')
        (Join-Path $env:LOCALAPPDATA 'GPTEasy')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Container }
}

function Get-GPTEasyProcesses([string]$ExecutablePath) {
    return @(Get-CimInstance Win32_Process -Filter "Name = 'gpteasy.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath.Equals($ExecutablePath, [StringComparison]::OrdinalIgnoreCase) })
}

function Confirm-UatStep(
    [System.Collections.Generic.List[object]]$Checks,
    [string]$Id,
    [string]$Prompt
) {
    Write-Host ''
    Write-Host $Prompt
    $answer = Read-Host '仅在实际观察到要求的行为后输入 PASS'
    if ($answer -cne 'PASS') {
        throw "UAT 步骤未确认：$Id"
    }
    $Checks.Add([ordered]@{ id = $Id; passed = $true })
}

$isWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
if (-not $isWindowsHost) {
    throw 'Windows UAT 必须在 Windows 上运行。'
}
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Windows UAT 要求使用 x64 操作系统。'
}
if ([System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') {
    throw 'Windows UAT 要求使用 x64 PowerShell 进程。'
}
$os = Get-CimInstance Win32_OperatingSystem
if ([int]$os.BuildNumber -lt 19045) {
    throw 'Windows UAT 要求 Windows build 19045 或更高版本。'
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$installer = Get-Item -LiteralPath (Resolve-Path -LiteralPath $InstallerPath).Path
if ($installer.Extension -ne '.exe' -or $installer.Name -notlike '*-setup.exe') {
    throw 'InstallerPath 必须指向 Tauri NSIS 安装程序。'
}
$candidateManifestFile = Get-Item -LiteralPath (Resolve-Path -LiteralPath $CandidateManifestPath).Path

$branch = (& git -C $repoRoot branch --show-current | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $branch -ne 'main') {
    throw 'Windows UAT 必须从 main 分支运行。'
}
$worktree = (& git -C $repoRoot status --porcelain | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $worktree) {
    throw 'Windows UAT 要求工作树保持干净。'
}
$commit = (& git -C $repoRoot rev-parse HEAD | Out-String).Trim()

$secretFile = Get-Item -LiteralPath (Resolve-Path -LiteralPath $SecretPath).Path
& git -C $repoRoot check-ignore --quiet -- $secretFile.FullName
if ($LASTEXITCODE -ne 0) {
    throw '供应商秘密文件必须被 Git 忽略。'
}
$secret = Get-Content -LiteralPath $secretFile.FullName -Raw | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($secret.base_url) -or
    [string]::IsNullOrWhiteSpace($secret.api_key) -or
    [string]::IsNullOrWhiteSpace($secret.model) -or
    $secret.api_key.Length -lt 8) {
    throw '供应商秘密文件必须包含非空的 base_url、api_key 和 model 字段。'
}
$providerUri = [Uri]([string]$secret.base_url)
$providerBuilder = New-Object System.UriBuilder($providerUri)
$providerBuilder.Path = $providerBuilder.Path.TrimEnd('/')
$normalizedBaseUrl = $providerBuilder.Uri.AbsoluteUri
$combinationMaterial = "gpteasy-provider-combination-v1`0$normalizedBaseUrl`0$($secret.model)`0$($secret.api_key)"
$combinationFingerprint = Get-Sha256Text $combinationMaterial

$codexCommand = Get-Command codex -ErrorAction SilentlyContinue
if (-not $codexCommand) {
    throw '必须安装当前支持的 Codex CLI。'
}
$codexVersion = (& codex --version 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $codexVersion -notmatch '^codex-cli (\d+\.\d+\.\d+)') {
    throw '无法读取受支持的 Codex CLI 版本。'
}
if ([version]$Matches[1] -lt [version]'0.147.0') {
    throw 'Windows UAT 要求 Codex CLI 0.147.0 或更高版本。'
}
$desktopPackage = @(Get-AppxPackage -Name 'OpenAI.Codex*' -ErrorAction SilentlyContinue)
if ($desktopPackage.Count -ne 1) {
    throw '必须恰好安装一个桌面 Codex 包。'
}
if ($desktopPackage[0].Architecture.ToString() -ne 'X64' -or
    $desktopPackage[0].Version -lt [version]'26.803.5235.0') {
    throw 'Windows UAT 要求安装受支持版本的 x64 桌面 Codex 包。'
}

$dataRoot = Join-Path $env:LOCALAPPDATA 'com.gpteasy.desktop'
$codexConfig = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex\config.toml'
$startMenuShortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\GPTEasy.lnk'
if (Test-Path -LiteralPath $dataRoot) {
    throw '一次性 UAT 账户中不能预先存在 GPTEasy 用户数据目录。'
}
if (Test-Path -LiteralPath $codexConfig) {
    throw '一次性 UAT 必须从当前用户不存在 Codex config.toml 的状态开始。'
}
if (@(Get-InstalledRoots).Count -ne 0) {
    throw '运行一次性 UAT 前必须先卸载 GPTEasy。'
}
if (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf) {
    throw '一次性 UAT 账户中不能预先存在 GPTEasy 开始菜单项。'
}

$treeOutput = (& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot 'scripts\test-release-tree.ps1') -RepositoryRoot $repoRoot 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw '发布树门禁执行失败。'
}
$treeReport = $treeOutput | ConvertFrom-Json
if (-not $treeReport.passed) {
    throw '发布树报告未通过。'
}

$installerHash = Get-Sha256File $installer.FullName
$signature = Get-FileSignature $installer.FullName
if ($signature.Status -ne 'Valid' -and $signature.Status -ne 'NotSigned') {
    throw "安装包的 Authenticode 状态不可接受：$($signature.Status)。"
}
if ($RequireAuthenticode -and $signature.Status -ne 'Valid') {
    throw '正式发布 UAT 要求安装包具有有效的 Authenticode 签名。'
}
$candidateManifest = Get-Content -LiteralPath $candidateManifestFile.FullName -Raw | ConvertFrom-Json
$candidateArtifactName = [System.IO.Path]::GetFileName(([string]$candidateManifest.artifact.path).Replace('/', '\'))
if ($candidateManifest.schemaVersion -ne 1 -or
    $candidateManifest.issue -ne 11 -or
    $candidateManifest.gitCommit -ne $commit -or
    $candidateManifest.platform -ne 'windows-x64-current-user' -or
    $candidateArtifactName -ne $installer.Name -or
    $candidateManifest.artifact.sha256 -ne $installerHash -or
    [int64]$candidateManifest.artifact.size -ne $installer.Length -or
    $candidateManifest.artifact.authenticodeStatus -ne $signature.Status.ToString()) {
    throw '安装包与当前提交对应的候选 manifest 不匹配。'
}
$candidateVerification = $candidateManifest.verification
if ($candidateVerification.frontendCheck -ne 'passed' -or
    $candidateVerification.frontendTests -ne 'passed' -or
    $candidateVerification.rustTests -ne 'passed' -or
    $candidateVerification.acceptanceGate -ne 'passed' -or
    $candidateVerification.releaseTree -ne 'passed') {
    throw '候选 manifest 未记录所有必要构建门禁均已通过。'
}
$candidateManifestSha256 = Get-Sha256File $candidateManifestFile.FullName
$checks = [System.Collections.Generic.List[object]]::new()
$checks.Add([ordered]@{ id = 'release_tree'; passed = $true })

$install = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($install.ExitCode -ne 0) {
    throw "安装程序执行失败，退出码：$($install.ExitCode)。"
}
Start-Sleep -Seconds 2
$installedRoots = @(Get-InstalledRoots)
if ($installedRoots.Count -ne 1) {
    throw "应找到一个当前用户安装目录，实际找到 $($installedRoots.Count) 个。"
}
$installRoot = (Resolve-Path -LiteralPath $installedRoots[0]).Path
$localRoot = (Resolve-Path -LiteralPath $env:LOCALAPPDATA).Path
if (-not $installRoot.StartsWith($localRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw '安装目录超出了当前用户的 LocalAppData。'
}
$app = Get-Item -LiteralPath (Join-Path $installRoot 'gpteasy.exe')
$uninstaller = Get-Item -LiteralPath (Join-Path $installRoot 'uninstall.exe')
if (-not (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf)) {
    throw '安装程序未创建当前用户开始菜单项。'
}
$checks.Add([ordered]@{ id = 'install_current_user'; passed = $true })

Start-Process -FilePath $app.FullName | Out-Null
Confirm-UatStep $checks 'application_launch' '确认已安装的 GPTEasy 设置窗口可见且可正常操作。'
Confirm-UatStep $checks 'real_provider_validation' '从 provider.json 手工输入真实供应商信息，完成模型发现和 Responses 流式工具调用验证。'
Confirm-UatStep $checks 'provider_save_and_switch' '保持至少一个旧 Codex 消费者正在运行，明确保存已验证供应商并应用到当前用户 Codex 环境。'
Confirm-UatStep $checks 'pending_restart' '确认 GPTEasy 显示待重启，且没有终止仍在运行的旧 Codex 消费者。'
Confirm-UatStep $checks 'cli_new_process_read' '退出旧 Codex CLI，启动新的真实 Codex CLI 进程，并确认真实请求使用目标供应商和凭据载体。'
Confirm-UatStep $checks 'desktop_new_process_read' '关闭旧桌面 Codex，启动新的桌面 Codex 进程，并确认真实请求使用目标供应商和凭据载体。'
Confirm-UatStep $checks 'restore_last_config' '使用“恢复上次配置”，确认当前用户 Codex 环境恢复到此前的完整状态。'
Confirm-UatStep $checks 'external_config_takeover' '创建有效的外部供应商配置，重新扫描并检查替换范围，明确接管后确认无关 TOML 字段仍被保留。'
Confirm-UatStep $checks 'managed_conflict' '从外部修改供应商 ID 或受管字段，确认 GPTEasy 阻止写入，直到用户明确处理管理冲突。'
Confirm-UatStep $checks 'openai_login_mode' '明确处理管理冲突后切换到 OpenAI 登录模式，确认 GPTEasy 不读取、保存或删除登录令牌。'
Confirm-UatStep $checks 'provider_combination_applied' '切回 provider.json 对应的供应商，并确认它成为当前供应商。'
Confirm-UatStep $checks 'tray_residency' '关闭设置窗口，确认 GPTEasy 继续驻留托盘；从托盘重新打开设置，最后使用托盘中的“退出 GPTEasy”。'

Start-Sleep -Seconds 1
if (@(Get-GPTEasyProcesses $app.FullName).Count -ne 0) {
    throw 'GPTEasy 仍在运行；覆盖安装前请使用托盘中的“退出 GPTEasy”。'
}
$appliedConfig = Get-Content -LiteralPath $codexConfig -Raw
if (-not $appliedConfig.Contains($normalizedBaseUrl) -or
    -not $appliedConfig.Contains([string]$secret.model) -or
    $appliedConfig.Contains([string]$secret.api_key)) {
    throw '已应用的 Codex 配置未包含供应商元数据，或错误地包含了 API Key。'
}
$credentialsPath = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex\auth.json'
$appliedCredentials = Get-Content -LiteralPath $credentialsPath -Raw | ConvertFrom-Json
if ($appliedCredentials.auth_mode -ne 'apikey' -or
    $appliedCredentials.OPENAI_API_KEY -cne [string]$secret.api_key) {
    throw 'Codex 凭据载体未包含供应商 API Key。'
}
$checks.Add([ordered]@{ id = 'provider_combination_match'; passed = $true })
$stateDatabase = Join-Path $dataRoot 'state.sqlite3'
if (-not (Test-Path -LiteralPath $stateDatabase -PathType Leaf)) {
    throw '已安装的应用未创建状态数据库。'
}
$stateHashBeforeOverwrite = Get-Sha256File $stateDatabase

$overwrite = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($overwrite.ExitCode -ne 0) {
    throw "覆盖安装失败，退出码：$($overwrite.ExitCode)。"
}
Start-Sleep -Seconds 2
if (-not (Test-Path -LiteralPath $app.FullName -PathType Leaf) -or
    -not (Test-Path -LiteralPath $uninstaller.FullName -PathType Leaf)) {
    throw '覆盖安装后应用程序或卸载程序缺失。'
}
$stateHashAfterOverwrite = Get-Sha256File $stateDatabase
if ($stateHashAfterOverwrite -ne $stateHashBeforeOverwrite) {
    throw '覆盖安装修改了 GPTEasy 用户数据。'
}
$checks.Add([ordered]@{ id = 'overwrite_install'; passed = $true })

Start-Process -FilePath $app.FullName | Out-Null
Confirm-UatStep $checks 'overwrite_launch' '确认覆盖安装后的应用可以启动，并保留供应商目录和环境状态；随后使用托盘中的“退出 GPTEasy”。'
Start-Sleep -Seconds 1
if (@(Get-GPTEasyProcesses $app.FullName).Count -ne 0) {
    throw 'GPTEasy 仍在运行；卸载前请使用托盘中的“退出 GPTEasy”。'
}
$stateHashBeforeUninstall = Get-Sha256File $stateDatabase

$uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($uninstall.ExitCode -ne 0) {
    throw "卸载程序执行失败，退出码：$($uninstall.ExitCode)。"
}
Start-Sleep -Seconds 2
if (Test-Path -LiteralPath $installRoot) {
    throw '卸载后应用安装目录仍然存在。'
}
if (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf) {
    throw '卸载后开始菜单项仍然存在。'
}
if (-not (Test-Path -LiteralPath $stateDatabase -PathType Leaf)) {
    throw '卸载过程删除了 GPTEasy 用户数据。'
}
$stateHashAfterUninstall = Get-Sha256File $stateDatabase
if ($stateHashAfterUninstall -ne $stateHashBeforeUninstall) {
    throw '卸载过程修改了 GPTEasy 用户数据。'
}
$checks.Add([ordered]@{ id = 'uninstall'; passed = $true })
$checks.Add([ordered]@{ id = 'data_retention'; passed = $true })

$sessionRoot = Join-Path $repoRoot "src-tauri\target\uat\$((Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ'))"
New-Item -ItemType Directory -Path $sessionRoot -Force | Out-Null
$pendingEvidencePath = Join-Path $sessionRoot 'evidence.pending.json'
$evidencePath = Join-Path $sessionRoot 'evidence.json'
$checks.Add([ordered]@{ id = 'credential_leak_scan'; passed = $true })
$evidence = [ordered]@{
    schemaVersion = 1
    issue = 11
    evidenceOrigin = 'interactive-windows-uat'
    completedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    gitCommit = $commit
    candidateManifestSha256 = $candidateManifestSha256
    platform = [ordered]@{
        os = 'windows'
        architecture = 'x64'
        build = [int]$os.BuildNumber
    }
    codexCliVersion = $codexVersion
    desktopCodexVersion = $desktopPackage[0].Version.ToString()
    providerCombinationFingerprint = $combinationFingerprint
    artifact = [ordered]@{
        fileName = $installer.Name
        sha256 = $installerHash
        size = $installer.Length
        authenticodeStatus = $signature.Status.ToString()
    }
    checks = @($checks)
}
$json = $evidence | ConvertTo-Json -Depth 10
if ($json.Contains([string]$secret.api_key)) {
    throw 'UAT 证据包含供应商 API Key，因此未写入证据。'
}
Write-Utf8NoBom $pendingEvidencePath $json
$apiKeyBytes = [System.Text.Encoding]::UTF8.GetBytes([string]$secret.api_key)
$leaked = Get-ChildItem -LiteralPath $sessionRoot -Recurse -File | Where-Object {
    Test-FileContainsBytes $_.FullName $apiKeyBytes
} | Select-Object -First 1
if ($leaked) {
    Remove-Item -LiteralPath $pendingEvidencePath -Force -ErrorAction SilentlyContinue
    throw 'UAT 输出包含供应商 API Key，因此未保留证据。'
}
Move-Item -LiteralPath $pendingEvidencePath -Destination $evidencePath
Write-Output $json
