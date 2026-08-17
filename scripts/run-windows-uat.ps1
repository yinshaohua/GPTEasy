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

function Wait-GPTEasyProcessesExit([string]$ExecutablePath, [int]$TimeoutSeconds = 2) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (@(Get-GPTEasyProcesses $ExecutablePath).Count -eq 0) {
            return $true
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
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

function Assert-UatCheckContract(
    [System.Collections.Generic.List[object]]$Checks,
    [object]$ReleaseContract
) {
    $seen = @{}
    foreach ($check in @($Checks)) {
        $id = [string]$check.id
        if ($seen.ContainsKey($id)) {
            throw "UAT 检查重复：$id。"
        }
        $seen[$id] = $true
    }
    $requiredChecks = @($ReleaseContract.requiredUatChecks | ForEach-Object { [string]$_.id })
    foreach ($required in $requiredChecks) {
        if (-not $seen.ContainsKey([string]$required)) {
            throw "UAT 缺少发布合同要求的检查：$required。"
        }
    }
    if ($seen.Count -ne $requiredChecks.Count) {
        throw 'UAT 包含发布合同之外的检查。'
    }
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
$releaseContract = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot 'windows-release-contract.json')) | ConvertFrom-Json
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
    $candidateManifest.issue -ne $releaseContract.issue -or
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
    $candidateVerification.layoutTests -ne 'passed' -or
    $candidateVerification.rustTests -ne 'passed' -or
    $candidateVerification.acceptanceGate -ne 'passed' -or
    $candidateVerification.releaseTree -ne 'passed' -or
    $candidateVerification.releaseContract -ne 'passed') {
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
$primaryProcesses = @(Get-GPTEasyProcesses $app.FullName)
if ($primaryProcesses.Count -ne 1) {
    throw "首次启动后应只有一个同路径 GPTEasy 进程，实际找到 $($primaryProcesses.Count) 个。"
}
$primaryProcessId = [uint32]$primaryProcesses[0].ProcessId
Confirm-UatStep $checks 'single_instance_precondition' '最小化 GPTEasy 设置窗口，确认窗口已不在前台后输入 PASS。'
Start-Process -FilePath $app.FullName | Out-Null
Start-Sleep -Seconds 2
$reactivatedProcesses = @(Get-GPTEasyProcesses $app.FullName)
if ($reactivatedProcesses.Count -ne 1 -or [uint32]$reactivatedProcesses[0].ProcessId -ne $primaryProcessId) {
    throw '第二次启动必须唤醒原实例，且不能留下第二个同路径 GPTEasy 进程。'
}
$checks.Add([ordered]@{ id = 'single_instance_process'; passed = $true })
Confirm-UatStep $checks 'single_instance_activation' '确认第二次启动已显示、取消最小化并聚焦原设置窗口，且系统托盘中仍只有一个 GPTEasy 入口。'
Confirm-UatStep $checks 'dayway_lifecycle' '打开 DayWay 官网入口，确认 URL 固定为 https://dayway.site；使用 provider.json 真实凭据配置 DayWay，确认名称和 https://dayway.site/v1 预填、实时模型发现及完整 Responses 流式工具调用验证。'
Confirm-UatStep $checks 'real_provider_validation' '完成 DayWay 之外的真实供应商模型发现和 Responses 流式工具调用验证。'
Confirm-UatStep $checks 'base_url_suggestion' '输入路径错误但同源的 BASE_URL，确认只在安全范围内串行探测并在完整验证后展示建议；明确拒绝或采用建议均不会静默保存。'
Confirm-UatStep $checks 'provider_order_and_tray_sync' '创建至少三个已验证供应商并拖拽排序，确认 DayWay 固定首位、目录顺序持久化且托盘顺序一致。'
Confirm-UatStep $checks 'settings_and_tray_switch_confirmation' '分别从设置页和托盘选择非当前供应商，确认两处复用同一个简短确认；确认只显示目标供应商、旧消费者风险以及“取消 / 切换”，不显示 Codex 工件、字段或重启选项。'
Confirm-UatStep $checks 'switch_success_updates_current_provider' '保持至少一个旧 Codex 消费者正在运行并完成供应商切换，确认成功后目录立即把目标供应商标为“当前使用”。'
Confirm-UatStep $checks 'switch_failure_refreshes_environment' '通过外部并发修改制造一次安全失败的供应商切换，确认界面重新读取环境实际状态、不保留请求前的过期当前供应商，且不自动重试或覆盖外部修改；随后按界面流程恢复可继续验收的受管状态。'
Confirm-UatStep $checks 'pending_restart_passive' '确认运行中的切换前消费者只产生被动待重启状态，且 GPTEasy 没有启动、关闭、终止、激活或重启消费者。'
Confirm-UatStep $checks 'consumers_not_controlled' '确认 GPTEasy 没有启动、关闭、终止、激活或重启桌面版和 Codex CLI 的入口；运行中的消费者始终由用户在原入口处理。'
Confirm-UatStep $checks 'pending_restart_auto_clear' '从原入口自然退出所有切换前消费者，确认待重启状态自动清除，且没有出现重启弹窗或重启按钮。'
Confirm-UatStep $checks 'cli_new_process_read' '启动新的真实 Codex CLI 进程，并确认真实请求使用目标供应商和凭据载体。'
Confirm-UatStep $checks 'session_app_server_contract' '进入“会话管理”，确认页面使用实际安装的 Codex App Server 加载用户交互会话，列表、详情、cursor、项目和会话供应商筛选均可用；exec、子代理和自动化来源不出现。'
Confirm-UatStep $checks 'session_mutation_safety' '在外部桌面版、Codex CLI 或 IDE Codex 运行时确认会话仍可浏览但归档、取消归档和永久删除全部禁用；停止外部消费者后确认归档和取消归档逐项显示结果，部分失败不会伪装成整体成功。'
Confirm-UatStep $checks 'session_protocol_degradation' '在协议能力不足或 App Server 异常退出场景确认页面显示明确不可用/恢复失败状态；只读请求最多自动恢复一次，归档、取消归档和永久删除不会自动重发。'
Confirm-UatStep $checks 'session_process_lifecycle' '进入会话页时确认自有会话服务按需启动且没有控制台窗口或闪烁；离开页面后宽限关闭，宽限内返回可复用同一进程，托盘驻留期间不会预先启动。'
Confirm-UatStep $checks 'session_process_recovery' '使用托盘明确退出、强制终止或模拟 GPTEasy 崩溃后确认自有会话服务进程树在合理窗口内归零；重启恢复只处理 PID、创建时间、规范化路径、命令身份和所有权代次全部匹配的精确残留，其他 App Server 不受影响。'
Confirm-UatStep $checks 'restore_last_config' '使用“恢复上次配置”，确认当前用户 Codex 环境恢复到此前的完整状态。'
Confirm-UatStep $checks 'external_config_takeover' '创建有效的外部供应商配置，重新扫描并检查替换范围，明确接管后确认无关 TOML 字段仍被保留。'
Confirm-UatStep $checks 'managed_conflict' '从外部修改供应商 ID 或受管字段，确认 GPTEasy 阻止写入，直到用户明确处理管理冲突。'
Confirm-UatStep $checks 'openai_login_mode' '明确处理管理冲突后切换到 OpenAI 登录模式，确认 GPTEasy 不读取、保存或删除登录令牌。'
Confirm-UatStep $checks 'provider_combination_applied' '切回 provider.json 对应的供应商，并确认它成为当前供应商。'
Confirm-UatStep $checks 'tray_residency' '关闭设置窗口，确认 GPTEasy 继续驻留且不会退出；再从托盘重新打开设置。'
Confirm-UatStep $checks 'layout_default_1120x620' '将窗口设为默认 1120 × 620，确认底部环境操作可到达，长名称、标签、BASE_URL、模型和行级按钮不重叠，行操作未被旧断点强制换行。'
Confirm-UatStep $checks 'layout_minimum_680x520' '将窗口缩到最小 680 × 520，确认底部操作可通过页面滚动到达，所有文字、标签和按钮不重叠且完整操作仍可完成。'
Confirm-UatStep $checks 'usability_200_percent' '将 Windows 缩放设为 200%，使用长 BASE_URL 和长模型 ID，确认目录、详情、验证弹窗和确认对话框无重叠且文本完整。'
Confirm-UatStep $checks 'usability_reduced_motion' '启用减少动态效果，确认验证进度使用静态状态且所有操作仍可完成。'
Confirm-UatStep $checks 'usability_high_contrast' '启用高对比度主题，确认状态、错误、按钮和焦点仍清晰可辨。'
Confirm-UatStep $checks 'usability_keyboard' '仅用键盘完成除排序外的目录、详情、验证、保存、切换、恢复和托盘前设置操作。'
Confirm-UatStep $checks 'explicit_tray_exit' '使用托盘中的“退出 GPTEasy”，确认窗口和托盘入口均已关闭。'

if (-not (Wait-GPTEasyProcessesExit $app.FullName)) {
    throw '明确退出后 2 秒内同路径 GPTEasy 进程未归零。'
}
$checks.Add([ordered]@{ id = 'explicit_exit_process_cleanup'; passed = $true })
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
if (-not (Wait-GPTEasyProcessesExit $app.FullName)) {
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
Assert-UatCheckContract $checks $releaseContract
$evidence = [ordered]@{
    schemaVersion = 1
    issue = [int]$releaseContract.issue
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
