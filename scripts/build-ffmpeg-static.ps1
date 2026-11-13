# 배포용 최소 구성 FFmpeg를 MSVC로 정적 빌드합니다. 결과는 .deps/ffmpeg-static입니다.
# 사용: pwsh scripts/build-ffmpeg-static.ps1   이후 FFMPEG_DIR=<저장소>/.deps/ffmpeg-static, ffmpeg-sys-the-third의 static 기능
# 필요: Visual Studio(C++ 도구), MSYS2(C:\msys64) + pacman -S --needed make nasm diffutils pkgconf, Python 3 (meson·ninja를 .deps/pytools에 설치)
# dav1d(AV1 소프트웨어 디코더)를 먼저 정적으로 빌드해 FFmpeg에 넣습니다.
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
$sw = [Diagnostics.Stopwatch]::StartNew()

# ---- dav1d (AV1 소프트웨어 디코더): meson으로 MSVC 정적 빌드 ----
$root = Split-Path -Parent $PSScriptRoot
$deps = Join-Path $root '.deps'
$lock = @{}
Get-Content (Join-Path $PSScriptRoot 'deps.lock') | Where-Object { $_ -match '^[A-Z_0-9]+=' } | ForEach-Object {
    $k, $v = $_ -split '=', 2; $lock[$k] = $v.Trim()
}
$dav1dPrefix = Join-Path $deps 'dav1d-static'
$py = Join-Path $deps 'pytools'
if (-not (Test-Path (Join-Path $py 'Scripts\meson.exe'))) {
    python -m venv $py
    & (Join-Path $py 'Scripts\python.exe') -m pip install --quiet "meson==$($lock.MESON_VERSION)" "ninja==$($lock.NINJA_VERSION)"
    if ($LASTEXITCODE -ne 0) { throw 'meson/ninja 설치 실패' }
}
# meson·ninja와 MSYS2의 nasm을 쓰되, MSYS2의 link가 MSVC link.exe를 가리지 않게 cl.exe 폴더를 맨 앞에 둡니다.
$clDir = Split-Path (Get-Command cl.exe).Source
$env:PATH = "$clDir;$(Join-Path $py 'Scripts');$env:PATH;$(Join-Path $Msys 'usr\bin')"

$tar = Join-Path $deps ('src\' + [IO.Path]::GetFileName($lock.DAV1D_SRC_URL))
New-Item -ItemType Directory -Force (Split-Path $tar) | Out-Null
if (-not (Test-Path $tar) -or (Get-FileHash $tar -Algorithm SHA256).Hash.ToLower() -ne $lock.DAV1D_SRC_SHA256) {
    Invoke-WebRequest -Uri $lock.DAV1D_SRC_URL -OutFile $tar
}
if ((Get-FileHash $tar -Algorithm SHA256).Hash.ToLower() -ne $lock.DAV1D_SRC_SHA256) { throw 'dav1d sha256 불일치' }
$dav1dSrc = Join-Path $deps 'src\dav1d'
$dav1dBuild = Join-Path $deps 'build\dav1d'
foreach ($d in $dav1dSrc, $dav1dBuild, $dav1dPrefix) { if (Test-Path $d) { Remove-Item -Recurse -Force $d } }
New-Item -ItemType Directory -Force $dav1dSrc | Out-Null
# Windows의 tar를 씁니다. MSYS2의 tar는 C: 경로를 원격 호스트로 해석합니다.
& (Join-Path $env:SystemRoot 'System32\tar.exe') -xf $tar -C $dav1dSrc --strip-components=1
if ($LASTEXITCODE -ne 0) { throw 'dav1d 압축 풀기 실패' }
# b_vscrt=mt: 실행 파일의 정적 CRT(D18)와 맞춥니다. 8bit와 10bit(16)만 켭니다.
meson setup $dav1dBuild $dav1dSrc --prefix=$dav1dPrefix --buildtype=release --default-library=static `
    -Db_vscrt=mt -Db_ndebug=true -Denable_tools=false -Denable_tests=false -Denable_examples=false '-Dbitdepths=8,16'
if ($LASTEXITCODE -ne 0) { throw 'dav1d meson setup 실패' }
meson install -C $dav1dBuild
if ($LASTEXITCODE -ne 0) { throw 'dav1d 빌드 실패' }
# FFmpeg의 msvc 툴체인은 -ldav1d를 dav1d.lib로 찾습니다.
$lib = Join-Path $dav1dPrefix 'lib'
if (-not (Test-Path (Join-Path $lib 'dav1d.lib'))) { Copy-Item (Join-Path $lib 'libdav1d.a') (Join-Path $lib 'dav1d.lib') }
$env:DAV1D_PREFIX = $dav1dPrefix -replace '\\', '/'

$bash = Join-Path $Msys 'usr\bin\bash.exe'
if (-not (Test-Path $bash)) { throw "MSYS2 bash 없음: $bash" }
$env:MSYS2_PATH_TYPE = 'inherit'   # Windows PATH(cl, link)를 MSYS2 셸에 넘깁니다
$env:CHERE_INVOKING = '1'          # 로그인 셸이 홈으로 이동하지 않게 합니다
$env:MSYSTEM = 'MSYS'

$script = (Join-Path $PSScriptRoot 'build-ffmpeg-static.sh') -replace '\\', '/'
& $bash -lc "bash '$script'"
if ($LASTEXITCODE -ne 0) { throw "FFmpeg 정적 빌드 실패 (exit $LASTEXITCODE)" }
Write-Host ("빌드 시간: {0:mm\:ss}" -f $sw.Elapsed)
