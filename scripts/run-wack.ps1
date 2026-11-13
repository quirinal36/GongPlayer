# Windows App Certification Kit(인증 키트)로 Store 제출용 MSIX를 검사합니다.
# 관리자 권한이 필요합니다. 관리자가 아니면 UAC 승인을 받아 다시 실행합니다.
#   pwsh scripts/run-wack.ps1            # dist/GongPlayer_<버전>_x64.msix 검사
# 결과: dist/wack-report.xml (요약을 화면에도 출력)
param([string]$Package)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Package) {
    $version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([0-9.]+)"').Matches[0].Groups[1].Value
    $Package = Join-Path $root "dist\GongPlayer_${version}_x64.msix"
}
if (-not (Test-Path $Package)) { throw "패키지가 없습니다: $Package (pwsh scripts/package-msix.ps1 -Store 먼저)" }

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    Write-Host '관리자 권한으로 다시 실행합니다 (UAC 승인 필요)'
    Start-Process pwsh -Verb RunAs -Wait -ArgumentList '-NoProfile', '-NoExit', '-File', $PSCommandPath, '-Package', $Package
    return
}

$appcert = 'C:\Program Files (x86)\Windows Kits\10\App Certification Kit\appcert.exe'
if (-not (Test-Path $appcert)) { throw "인증 키트가 없습니다: $appcert (Windows SDK 설치 필요)" }
$report = Join-Path $root 'dist\wack-report.xml'
if (Test-Path $report) { Remove-Item $report }

& $appcert reset | Out-Null
& $appcert test -appxpackagepath $Package -reportoutputpath $report
if (-not (Test-Path $report)) { throw "보고서가 만들어지지 않았습니다 (종료 코드 $LASTEXITCODE)" }

[xml]$x = Get-Content $report
"전체 결과: $($x.REPORT.OVERALL_RESULT)"
foreach ($t in $x.SelectNodes('//TEST')) {
    $r = $t.RESULT.'#cdata-section'
    if (-not $r) { $r = $t.RESULT }
    if ($r -ne 'PASS') { "  {0}: {1}" -f $t.NAME, $r }
}
"보고서: $report"
