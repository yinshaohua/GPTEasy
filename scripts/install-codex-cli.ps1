[CmdletBinding()]
param(
    [string]$NodeVersion = 'v24.21.0',
    [string]$NodeMirror = 'https://registry.npmmirror.com/-/binary/node',
    [string]$NpmRegistry = 'https://registry.npmmirror.com',
    [switch]$ForceNodeInstall
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$MinimumNodeVersion = [version]'22.20.0'
$NodeInstallRoot = Join-Path $env:LOCALAPPDATA 'Programs\nodejs'
$NpmGlobalBin = Join-Path $env:APPDATA 'npm'

function Add-UserPathEntry {
    param([Parameter(Mandatory)][string]$Entry)

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($userPath -split ';' | Where-Object { $_ })
    if ($entries -notcontains $Entry) {
        $newPath = (($entries + $Entry) -join ';')
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    }

    $currentEntries = @($env:Path -split ';' | Where-Object { $_ })
    if ($currentEntries -notcontains $Entry) {
        $env:Path = "$Entry;$env:Path"
    }
}

function Test-DirectoryWritable {
    param([Parameter(Mandatory)][string]$Directory)

    try {
        New-Item -ItemType Directory -Path $Directory -Force | Out-Null
        $probe = Join-Path $Directory ('.codex-write-test-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType File -Path $probe -Force | Out-Null
        Remove-Item -LiteralPath $probe -Force
        return $true
    }
    catch {
        return $false
    }
}

function Get-CommandPath {
    param([Parameter(Mandatory)][string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandType -in @('Application', 'ExternalScript') } |
        Sort-Object @{ Expression = {
            switch ([IO.Path]::GetExtension($_.Source).ToLowerInvariant()) {
                '.cmd' { 0 }
                '.exe' { 1 }
                '.ps1' { 2 }
                default { 3 }
            }
        } }, Source |
        Select-Object -First 1
    if ($null -eq $command) {
        return $null
    }
    return $command.Source
}

function Get-NodeVersion {
    param([Parameter(Mandatory)][string]$NodePath)

    $output = (& $NodePath --version 2>&1 | Select-Object -First 1).ToString().Trim()
    if ($output -notmatch '^v?(\d+\.\d+\.\d+)$') {
        throw "无法识别 Node.js 版本：$output"
    }
    return [version]$Matches[1]
}

function Invoke-DownloadWithRetry {
    param(
        [Parameter(Mandatory)][string]$Uri,
        [Parameter(Mandatory)][string]$OutFile,
        [int]$RetryCount = 3
    )

    for ($attempt = 1; $attempt -le $RetryCount; $attempt++) {
        try {
            Write-Host "下载：$Uri"
            Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing
            return
        }
        catch {
            if ($attempt -eq $RetryCount) {
                throw "下载失败（已重试 $RetryCount 次）：$Uri`n$($_.Exception.Message)"
            }
            Start-Sleep -Seconds 2
        }
    }
}

function Get-NodeArchitecture {
    $architecture = if ($env:PROCESSOR_ARCHITEW6432) {
        $env:PROCESSOR_ARCHITEW6432
    }
    else {
        $env:PROCESSOR_ARCHITECTURE
    }

    switch -Regex ($architecture) {
        '^ARM64$' { return 'win-arm64' }
        '^(AMD64|x86_64)$' { return 'win-x64' }
        default {
            if ([Environment]::Is64BitOperatingSystem) {
                return 'win-x64'
            }
            return 'win-x86'
        }
    }
}

function Install-UserNode {
    $nodeArchitecture = Get-NodeArchitecture
    $nodeDirectoryName = "node-$NodeVersion-$nodeArchitecture"
    $nodeDirectory = Join-Path $NodeInstallRoot $nodeDirectoryName
    $nodeExecutable = Join-Path $nodeDirectory 'node.exe'

    if (-not (Test-Path -LiteralPath $nodeExecutable)) {
        $temporaryDirectory = Join-Path $env:TEMP ("codex-node-" + [guid]::NewGuid().ToString('N'))
        $archivePath = Join-Path $temporaryDirectory "$nodeDirectoryName.zip"
        $extractDirectory = Join-Path $temporaryDirectory 'extract'

        try {
            New-Item -ItemType Directory -Path $temporaryDirectory, $extractDirectory -Force | Out-Null
            $downloadUri = "$NodeMirror/$NodeVersion/$nodeDirectoryName.zip"
            Invoke-DownloadWithRetry -Uri $downloadUri -OutFile $archivePath
            Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDirectory -Force

            $extractedDirectory = Join-Path $extractDirectory $nodeDirectoryName
            if (-not (Test-Path -LiteralPath (Join-Path $extractedDirectory 'node.exe'))) {
                throw "Node.js 压缩包内容不符合预期：缺少 $nodeDirectoryName\node.exe"
            }

            New-Item -ItemType Directory -Path $NodeInstallRoot -Force | Out-Null
            if (Test-Path -LiteralPath $nodeDirectory) {
                Remove-Item -LiteralPath $nodeDirectory -Recurse -Force
            }
            Move-Item -LiteralPath $extractedDirectory -Destination $nodeDirectory
        }
        finally {
            if (Test-Path -LiteralPath $temporaryDirectory) {
                Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
            }
        }
    }

    Add-UserPathEntry -Entry $nodeDirectory
    Add-UserPathEntry -Entry $NpmGlobalBin
    return [pscustomobject]@{
        NodePath = $nodeExecutable
        NpmPath = (Join-Path $nodeDirectory 'npm.cmd')
    }
}

Write-Host '检查现有 Node.js 和 npm ...'
$nodePath = Get-CommandPath -Name 'node'
$npmPath = Get-CommandPath -Name 'npm'
$usableNode = $false
$usingDownloadedNode = $false

if (-not $ForceNodeInstall -and $nodePath -and $npmPath) {
    try {
        $installedNodeVersion = Get-NodeVersion -NodePath $nodePath
        if ($installedNodeVersion -ge $MinimumNodeVersion) {
            $usableNode = $true
            Write-Host "复用现有 Node.js $installedNodeVersion。"
        }
        else {
            Write-Host "现有 Node.js $installedNodeVersion 低于要求的 $MinimumNodeVersion，将安装用户态 Node.js $NodeVersion。"
        }
    }
    catch {
        Write-Host "现有 Node.js 无法使用，将安装用户态 Node.js $NodeVersion。"
    }
}

if (-not $usableNode) {
    $usingDownloadedNode = $true
    $nodeTools = Install-UserNode
    $nodePath = $nodeTools.NodePath
    $npmPath = $nodeTools.NpmPath
    $installedNodeVersion = Get-NodeVersion -NodePath $nodePath
    if ($installedNodeVersion -lt $MinimumNodeVersion) {
        throw "安装的 Node.js 版本 $installedNodeVersion 低于要求的 $MinimumNodeVersion。"
    }
    Write-Host "已安装用户态 Node.js $installedNodeVersion。"
}

if (-not $npmPath -or -not (Test-Path -LiteralPath $npmPath)) {
    $npmPath = Get-CommandPath -Name 'npm'
}
if (-not $npmPath) {
    throw '找不到 npm，请确认 Node.js 安装完整。'
}

if (-not $usingDownloadedNode) {
    $existingPrefix = (& $npmPath prefix -g 2>$null | Select-Object -First 1).ToString().Trim()
    if ($existingPrefix -and (Test-DirectoryWritable -Directory $existingPrefix)) {
        $NpmGlobalBin = $existingPrefix
        Write-Host "保留现有 npm 全局目录：$NpmGlobalBin"
    }
    else {
        Write-Host "现有 npm 全局目录不可写，将使用用户目录：$NpmGlobalBin"
    }
}

New-Item -ItemType Directory -Path $NpmGlobalBin -Force | Out-Null
Add-UserPathEntry -Entry $NpmGlobalBin

Write-Host "配置 npm 镜像：$NpmRegistry"
& $npmPath config set prefix $NpmGlobalBin --location=user
if ($LASTEXITCODE -ne 0) {
    throw '配置 npm 全局安装目录失败。'
}
& $npmPath config set registry $NpmRegistry --location=user
if ($LASTEXITCODE -ne 0) {
    throw '配置 npm 镜像失败。'
}

Write-Host '检查 npm 镜像连通性 ...'
& $npmPath ping --registry $NpmRegistry
if ($LASTEXITCODE -ne 0) {
    throw "npm 镜像不可访问：$NpmRegistry"
}

Write-Host '安装 @openai/codex@latest ...'
& $npmPath install --global '@openai/codex@latest' --registry $NpmRegistry
if ($LASTEXITCODE -ne 0) {
    throw 'Codex CLI 安装失败。'
}

$codexPath = Get-CommandPath -Name 'codex'
if (-not $codexPath) {
    $candidate = Join-Path $NpmGlobalBin 'codex.cmd'
    if (Test-Path -LiteralPath $candidate) {
        $codexPath = $candidate
    }
}
if (-not $codexPath) {
    throw "安装完成但找不到 codex 命令。请重新打开 PowerShell，或确认 PATH 包含：$NpmGlobalBin"
}

Write-Host '安装完成，版本信息：'
& $codexPath --version
if ($LASTEXITCODE -ne 0) {
    throw 'codex 命令已找到，但版本检查失败。'
}

Write-Host "`n当前 PowerShell 已可直接运行 codex；新开的 PowerShell 也会自动读取用户 PATH。"
