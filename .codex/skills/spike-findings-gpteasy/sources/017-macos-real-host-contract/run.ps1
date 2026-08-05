$ErrorActionPreference = 'Stop'

$spike = Split-Path -Parent $MyInvocation.MyCommand.Path
$run = Join-Path $spike '.run'
New-Item -ItemType Directory -Path $run -Force | Out-Null

Push-Location (Join-Path $spike 'src-tauri')
try {
    cargo test
    Pop-Location
    Push-Location $spike
    npm install
    npm run tauri build -- --no-bundle
    $summary = [ordered]@{
        generated_at = [DateTimeOffset]::UtcNow.ToString('o')
        host = [ordered]@{
            os = [System.Environment]::OSVersion.Platform.ToString()
            architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
            native_macos = $false
        }
        checks = [ordered]@{
            rust_tests = 'passed'
            tauri_release_build_on_windows = 'passed'
            zsh_macos_runner_syntax = 'run separately with zsh -n'
            macos_real_host = 'not_run'
            macos_bundle = 'not_run'
            launch_services = 'not_run'
            current_user_install = 'not_run'
            updater_in_place = 'not_run'
            signing_notarization = 'not_run'
        }
        verdict = 'partial'
        reason = '当前主机为 Windows；只验证跨平台契约逻辑和 Tauri Windows 编译，不能替代真实 macOS。'
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'summary.json') -Encoding utf8NoBOM
}
finally {
    Pop-Location
}

Get-Content -LiteralPath (Join-Path $run 'summary.json')
