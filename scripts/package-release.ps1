# GitHub Release에 올릴 파일을 dist/release에 만듭니다.
#   pwsh scripts/package-release.ps1            # 정적 빌드부터
#   pwsh scripts/package-release.ps1 -NoBuild   # target/static의 빌드를 그대로 씀
# 먼저 scripts/build-ffmpeg-static.ps1로 .deps/ffmpeg-static을 만들어 두어야 합니다.
# 필요: Inno Setup 6 (winget install JRSoftware.InnoSetup)
#
# 결과
#   GongPlayer-<버전>-setup.exe        사용자 폴더에 설치 (관리자 권한 없음, 서명 없음)
#   GongPlayer-<버전>-portable.zip     실행 파일 하나와 라이선스 문서
#   ffmpeg-<버전>.tar.xz, dav1d-<버전>.tar.xz   정적으로 넣은 라이브러리의 대응 소스 (LGPL, D26)
#   SHA256SUMS.txt
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([0-9.]+)"').Matches[0].Groups[1].Value
$lock = @{}
Get-Content scripts\deps.lock | Where-Object { $_ -match '^[A-Z_0-9]+=' } | ForEach-Object { $k, $v = $_ -split '=', 2; $lock[$k] = $v.Trim() }

$target = Join-Path $root 'target\static'
if (-not $NoBuild) {
    if (-not (Test-Path .deps\ffmpeg-static\lib\avcodec.lib)) { throw '.deps/ffmpeg-static이 없습니다. scripts/build-ffmpeg-static.ps1을 먼저 실행하세요.' }
    $env:FFMPEG_DIR = Join-Path $root '.deps\ffmpeg-static'
    $env:CARGO_TARGET_DIR = $target
    cargo build --release --features static-ffmpeg
    $code = $LASTEXITCODE
    Remove-Item Env:\CARGO_TARGET_DIR, Env:\FFMPEG_DIR
    if ($code -ne 0) { throw 'cargo build 실패' }
}

$out = Join-Path $root 'dist\release'
$stage = Join-Path $out 'stage'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $stage | Out-Null

# 실행 파일과 라이선스 문서
Copy-Item (Join-Path $target 'release\gongplayer.exe') (Join-Path $stage 'GongPlayer.exe')
Copy-Item LICENSE (Join-Path $stage 'LICENSE.txt')
Copy-Item THIRD_PARTY_NOTICES.md $stage
$ffsrc = Join-Path '.deps\src' ([IO.Path]::GetFileName($lock.FFMPEG_SRC_URL))
$dvsrc = Join-Path '.deps\src' ([IO.Path]::GetFileName($lock.DAV1D_SRC_URL))
$fflic = Get-ChildItem .deps\src -Recurse -Filter 'COPYING.LGPLv2.1' | Select-Object -First 1
if (-not $fflic) { throw 'FFmpeg 라이선스 원문(COPYING.LGPLv2.1)이 없습니다' }
Copy-Item $fflic.FullName (Join-Path $stage 'FFMPEG_LICENSE.txt')
Copy-Item .deps\src\dav1d\COPYING (Join-Path $stage 'DAV1D_LICENSE.txt')

# 포터블 ZIP
$zip = Join-Path $out "GongPlayer-$version-portable.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip

# 설치 프로그램
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe', 'C:\Program Files\Inno Setup 6\ISCC.exe') |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6이 없습니다: winget install JRSoftware.InnoSetup' }
& $iscc /Q "/DAppVersion=$version" "/DStageDir=$stage" "/DOutDir=$out" (Join-Path $root 'packaging\installer\gongplayer.iss')
if ($LASTEXITCODE -ne 0) { throw '설치 프로그램 생성 실패' }

# 정적으로 넣은 라이브러리의 대응 소스 (deps.lock의 sha256으로 확인한 파일 그대로)
foreach ($s in @(@($ffsrc, $lock.FFMPEG_SRC_SHA256), @($dvsrc, $lock.DAV1D_SRC_SHA256))) {
    if ((Get-FileHash $s[0] -Algorithm SHA256).Hash.ToLower() -ne $s[1]) { throw "소스 sha256 불일치: $($s[0])" }
    Copy-Item $s[0] $out
}

Remove-Item -Recurse -Force $stage
$files = Get-ChildItem $out -File | Sort-Object Name
# sha256sum -c가 바로 읽도록 LF 줄바꿈으로 씁니다.
$sums = ($files | ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name }) -join "`n"
[IO.File]::WriteAllText((Join-Path $out 'SHA256SUMS.txt'), "$sums`n", (New-Object Text.ASCIIEncoding))
Get-ChildItem $out -File | ForEach-Object { "{0,-40} {1,10:N0} bytes" -f $_.Name, $_.Length }
