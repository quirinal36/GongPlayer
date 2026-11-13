# 개발용 MSIX를 자체 서명 인증서로 서명하고 이 PC에 설치합니다. Store 서명과는 다릅니다.
#   pwsh scripts/dev-install.ps1             # 패키지 생성 → 서명 → 설치
#   pwsh scripts/dev-install.ps1 -Uninstall  # 제거
# 처음 한 번은 인증서를 "로컬 컴퓨터\신뢰할 수 있는 사용자"에 넣기 위해 관리자 승인(UAC)을 묻습니다.
# 인증서의 개인 키는 현재 사용자 저장소에만 있고 내보내지 않습니다.
# -Dev: 개발용 FFmpeg DLL 빌드로 설치합니다. 기본은 배포용 정적 빌드입니다.
param([switch]$Uninstall, [switch]$NoBuild, [switch]$Dev)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$subject = 'CN=GongPlayer Dev'
$name = 'GongPlayer.Dev'

if ($Uninstall) {
    Get-AppxPackage -Name $name | Remove-AppxPackage
    "제거했습니다: $name"
    return
}

$cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $subject } | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $subject -KeyUsage DigitalSignature `
        -FriendlyName 'GongPlayer 개발용 패키지 서명' -CertStoreLocation Cert:\CurrentUser\My `
        -KeyExportPolicy NonExportable `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
    "인증서를 만들었습니다: $($cert.Thumbprint)"
}
$trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
if (-not $trusted) {
    $cer = Join-Path $env:TEMP 'gongplayer-dev.cer'
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    "인증서를 신뢰 목록에 넣습니다 (관리자 승인 필요)"
    Start-Process pwsh -Verb RunAs -Wait -ArgumentList '-NoProfile', '-Command',
        "Import-Certificate -FilePath '$cer' -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null"
}

# 이름 있는 인자는 해시테이블로 넘깁니다. 배열로 넘기면 위치 인자로 묶여 게시자 값이 어긋납니다.
$pkgArgs = @{ Name = $name; Publisher = $subject; NoBuild = [bool]$NoBuild; Static = -not $Dev }
& (Join-Path $PSScriptRoot 'package-msix.ps1') @pkgArgs
if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) { throw "패키지 생성 실패" }
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([0-9.]+)"').Matches[0].Groups[1].Value
$msix = Join-Path $root "dist\GongPlayer_${version}_x64.msix"

$kits = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName 'x64\signtool.exe') } | Sort-Object Name | Select-Object -Last 1
$signtool = Join-Path $kits.FullName 'x64\signtool.exe'
& $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $msix
if ($LASTEXITCODE -ne 0) { throw "서명 실패" }

Add-AppxPackage -Path $msix -ForceUpdateFromAnyVersion
Get-AppxPackage -Name $name | Select-Object Name, Version, InstallLocation
