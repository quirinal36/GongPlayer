# 릴리스 빌드를 MSIX로 묶습니다.
#   pwsh scripts/package-msix.ps1                 # 개발용(서명 없음, 테스트 Identity)
#   pwsh scripts/package-msix.ps1 -Store            # Store 제출용 (Partner Center의 Identity, 정적 빌드, 서명 없음, 버전 x.y.z.0)
# 결과: dist/GongPlayer_<버전>_x64.msix, 압축 크기와 설치(풀린) 크기를 출력합니다.
# 개발용 설치는 scripts/dev-install.ps1 (자체 서명 인증서로 서명 후 설치, 관리자 승인 필요).
# 서명 없는 패키지는 실행 파일을 활성화할 수 없어 설치되지 않습니다 (0x80073D2B).
param(
    [string]$Name = 'GongPlayer.Dev',
    # 개발용 게시자. dev-install.ps1의 인증서 주체와 같아야 합니다.
    [string]$Publisher = 'CN=GongPlayer Dev',
    [string]$PublisherDisplay = 'GongPlayer',
    [switch]$NoBuild,
    # 최소 구성 정적 FFmpeg로 빌드합니다 (scripts/build-ffmpeg-static.ps1 먼저). 실행 파일 하나만 싣습니다.
    [switch]$Static,
    # 패키지 버전의 네 번째 자리. Store 제출용은 0이어야 합니다. 개발용 설치는 매번 올려 덮어쓰기(업데이트)를 합니다.
    [int]$Revision = 0,
    # Microsoft Store 제출용. 파트너 센터 "제품 ID" 화면의 값입니다 (Store ID 9PGQL0Q90CGF).
    [switch]$Store
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if ($Store) {
    $Name = 'letscoding.GongPlayer'
    $Publisher = 'CN=B3FB09CC-66A1-4A25-AE74-EBFF02F06617'
    $PublisherDisplay = 'letscoding'
    $Static = $true
    $Revision = 0   # Store는 버전의 네 번째 자리가 0이어야 합니다
}

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([0-9.]+)"').Matches[0].Groups[1].Value
$msixVersion = "$version.$Revision"
$targetDir = if ($Static) { Join-Path $root 'target\static' } else { Join-Path $root 'target' }
if (-not $NoBuild) {
    if ($Static) {
        $env:FFMPEG_DIR = Join-Path $root '.deps\ffmpeg-static'
        $env:CARGO_TARGET_DIR = $targetDir
        cargo build --release --features static-ffmpeg
        Remove-Item Env:\CARGO_TARGET_DIR; Remove-Item Env:\FFMPEG_DIR
    } else {
        cargo build --release
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build 실패" }
}

$kits = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName 'x64\makeappx.exe') } | Sort-Object Name | Select-Object -Last 1
if (-not $kits) { throw "Windows SDK의 makeappx.exe를 찾을 수 없습니다" }
$makeappx = Join-Path $kits.FullName 'x64\makeappx.exe'

$layout = Join-Path $root 'dist\msix-layout'
if (Test-Path $layout) { Remove-Item -Recurse -Force $layout }
New-Item -ItemType Directory -Force (Join-Path $layout 'Assets') | Out-Null
Copy-Item (Join-Path $targetDir 'release\gongplayer.exe') (Join-Path $layout 'GongPlayer.exe')
# 개발용 빌드는 FFmpeg DLL을 함께 넣습니다. 정적 빌드에는 DLL이 없습니다.
$ffbin = Join-Path $targetDir 'release'
foreach ($dll in $(if ($Static) { @() } else { Get-ChildItem $ffbin -Filter '*.dll' })) {
    if ($dll.Name -match '^(avcodec|avformat|avutil|avfilter|swresample|swscale)-\d+\.dll$') {
        Copy-Item $dll.FullName $layout
    }
}
Copy-Item packaging\Assets\* (Join-Path $layout 'Assets')
if (Test-Path THIRD_PARTY_NOTICES.md) { Copy-Item THIRD_PARTY_NOTICES.md $layout }
Copy-Item LICENSE (Join-Path $layout 'LICENSE.txt')   # GongPlayer 자체 라이선스 (MPL-2.0)
if ($Static) {
    # 최소 구성은 외부 라이브러리 없이 FFmpeg만 들어 있으므로 LGPL 2.1 원문을 싣습니다.
    $lic = Get-ChildItem .deps\src -Recurse -Filter 'COPYING.LGPLv2.1' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($lic) { Copy-Item $lic.FullName (Join-Path $layout 'FFMPEG_LICENSE.txt') }
    # dav1d(BSD-2-Clause)는 원문을 함께 실어야 합니다.
    $dlic = Join-Path $root '.deps\src\dav1d\COPYING'
    if (Test-Path $dlic) { Copy-Item $dlic (Join-Path $layout 'DAV1D_LICENSE.txt') } else { throw "dav1d 라이선스 원문이 없습니다: $dlic" }
} elseif (Test-Path .deps\ffmpeg\LICENSE.txt) {
    Copy-Item .deps\ffmpeg\LICENSE.txt (Join-Path $layout 'FFMPEG_LICENSE.txt')
}

$manifest = Get-Content packaging\AppxManifest.xml -Raw -Encoding UTF8
$manifest = $manifest.Replace('{{IDENTITY_NAME}}', $Name).Replace('{{PUBLISHER}}', [Security.SecurityElement]::Escape($Publisher)).
    Replace('{{PUBLISHER_DISPLAY}}', [Security.SecurityElement]::Escape($PublisherDisplay)).Replace('{{VERSION}}', $msixVersion)
[IO.File]::WriteAllText((Join-Path $layout 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding $false))

New-Item -ItemType Directory -Force dist | Out-Null
$out = Join-Path $root "dist\GongPlayer_${version}_x64.msix"
& $makeappx pack /o /d $layout /p $out | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makeappx 실패" }

$code = (Get-ChildItem $layout -File | Where-Object { $_.Extension -in '.exe', '.dll' } | Measure-Object Length -Sum).Sum
$unpacked = (Get-ChildItem $layout -Recurse -File | Measure-Object Length -Sum).Sum
$packed = (Get-Item $out).Length
"패키지: $out"
"실행 코드(exe+dll) 합계: {0:N2} MB" -f ($code / 1MB)
"설치(풀린) 크기: {0:N2} MB" -f ($unpacked / 1MB)
"MSIX 압축 크기: {0:N2} MB" -f ($packed / 1MB)
