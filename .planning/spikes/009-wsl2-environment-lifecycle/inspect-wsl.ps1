param(
    [Parameter(Mandatory = $true)]
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$wsl = Join-Path $env:SystemRoot 'System32\wsl.exe'

function Invoke-WslText {
    param([string[]]$Arguments)

    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $wsl
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [System.Text.Encoding]::Unicode
    $info.StandardErrorEncoding = [System.Text.Encoding]::Unicode
    $info.CreateNoWindow = $true
    foreach ($argument in $Arguments) {
        $info.ArgumentList.Add($argument)
    }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $info
    [void]$process.Start()
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    [pscustomobject]@{
        ExitCode = $process.ExitCode
        Stdout = $stdout
        Stderr = $stderr
    }
}

function Get-Names {
    param([string[]]$Arguments)

    $result = Invoke-WslText -Arguments $Arguments
    if ($result.ExitCode -ne 0) {
        throw "wsl.exe $($Arguments -join ' ') failed: $($result.Stderr.Trim())"
    }
    @($result.Stdout -split "`r?`n" |
        ForEach-Object { $_.Trim([char]0xFEFF, [char]0, ' ', "`t", "`r", "`n") } |
        Where-Object { $_ })
}

$versionResult = Invoke-WslText -Arguments @('--version')
$all = @(Get-Names -Arguments @('--list', '--quiet'))
$runningBefore = @(Get-Names -Arguments @('--list', '--running', '--quiet'))

$registry = @()
$registryRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
if (Test-Path -LiteralPath $registryRoot) {
    $registry = @(Get-ChildItem -LiteralPath $registryRoot | ForEach-Object {
        $properties = Get-ItemProperty -LiteralPath $_.PSPath
        [ordered]@{
            id = $_.PSChildName
            distribution = $properties.DistributionName
            default_uid = $properties.DefaultUid
            version = $properties.Version
            base_path_present = -not [string]::IsNullOrWhiteSpace($properties.BasePath)
        }
    })
}

$runningAfter = @(Get-Names -Arguments @('--list', '--running', '--quiet'))
$evidence = [ordered]@{
    timestamp = [DateTimeOffset]::UtcNow.ToString('O')
    probe_ok = $true
    version_exit_code = $versionResult.ExitCode
    version_lines = @($versionResult.Stdout -split "`r?`n" |
        ForEach-Object { $_.Trim([char]0xFEFF, [char]0, ' ', "`t", "`r", "`n") } |
        Where-Object { $_ })
    distributions = $all
    running_before = $runningBefore
    running_after = $runningAfter
    running_set_unchanged = (@(Compare-Object $runningBefore $runningAfter).Count -eq 0)
    registry = $registry
    commands_used = @(
        'wsl.exe --version',
        'wsl.exe --list --quiet',
        'wsl.exe --list --running --quiet'
    )
    commands_that_entered_a_distribution = 0
}

$parent = Split-Path -Parent $Output
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding utf8NoBOM
$evidence | ConvertTo-Json -Depth 8
