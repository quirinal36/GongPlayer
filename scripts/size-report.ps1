# 실행 코드(exe + 함께 싣는 DLL) 합계를 재고 예산(10MB)과 비교합니다.
#   pwsh scripts/size-report.ps1            # 기록만
#   pwsh scripts/size-report.ps1 -Enforce   # 예산을 넘으면 실패
#   pwsh scripts/size-report.ps1 -Static    # 정적 FFmpeg 빌드(target/static) 기준
param([switch]$Enforce, [double]$BudgetMB = 10, [switch]$Static)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$rel = if ($Static) { Join-Path $root 'target\static\release' } else { Join-Path $root 'target\release' }
$exe = Get-Item (Join-Path $rel 'gongplayer.exe')
$dlls = Get-ChildItem $rel -Filter '*.dll' | Where-Object { $_.Name -match '^(avcodec|avformat|avutil|avfilter|swresample|swscale)-\d+\.dll$' }
$total = $exe.Length + ($dlls | Measure-Object Length -Sum).Sum
"gongplayer.exe: {0:N0} bytes" -f $exe.Length
foreach ($d in $dlls) { "{0}: {1:N0} bytes" -f $d.Name, $d.Length }
"실행 코드 합계: {0:N2} MB (예산 {1} MB)" -f ($total / 1MB), $BudgetMB
if ($Enforce -and $total -gt $BudgetMB * 1MB) { throw "실행 코드가 예산을 넘었습니다" }
