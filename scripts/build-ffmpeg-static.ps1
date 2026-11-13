# 배포용 최소 구성 FFmpeg를 MSVC로 정적 빌드합니다. 결과는 .deps/ffmpeg-static입니다.
# 사용: pwsh scripts/build-ffmpeg-static.ps1   이후 FFMPEG_DIR=<저장소>/.deps/ffmpeg-static, ffmpeg-sys-the-third의 static 기능
# 필요: Visual Studio(C++ 도구), MSYS2(C:\msys64) + pacman -S --needed make nasm diffutils pkgconf
param([string]$Msys = 'C:\msys64')
$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw "vswhere.exe 없음: $vswhere" }
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC x64 도구가 있는 Visual Studio를 찾지 못했습니다' }
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
Write-Host "MSVC: $vcvars"

# vcvars64.bat가 만든 환경 변수를 이 프로세스로 가져옵니다.
& cmd.exe /s /c "`"$vcvars`" >nul && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2]) }
}
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) { throw 'vcvars64 이후에도 cl.exe가 없습니다' }

$bash = Join-Path $Msys 'usr\bin\bash.exe'
if (-not (Test-Path $bash)) { throw "MSYS2 bash 없음: $bash" }
$env:MSYS2_PATH_TYPE = 'inherit'   # Windows PATH(cl, link)를 MSYS2 셸에 넘깁니다
$env:CHERE_INVOKING = '1'          # 로그인 셸이 홈으로 이동하지 않게 합니다
$env:MSYSTEM = 'MSYS'

$script = (Join-Path $PSScriptRoot 'build-ffmpeg-static.sh') -replace '\\', '/'
$sw = [Diagnostics.Stopwatch]::StartNew()
& $bash -lc "bash '$script'"
if ($LASTEXITCODE -ne 0) { throw "FFmpeg 정적 빌드 실패 (exit $LASTEXITCODE)" }
Write-Host ("빌드 시간: {0:mm\:ss}" -f $sw.Elapsed)
