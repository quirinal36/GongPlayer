# Windows 설치와 공개 배포

2026-11-13 사용자 요구를 반영한 배포 계획입니다. 개발용 MSIX 생성까지 했고, 개발용 서명·설치와 Store 등록은 아직 하지 않았습니다 (아래 "패키지 설계"의 진행 상황).

## 목표와 기본 경로

현재 Windows PC에서 개발하는 플레이어를 다른 사람도 쉽게 설치하게 합니다. 기본 경로는 **Rust·Win32 앱 → MSIX 패키지 → Microsoft Store**입니다.

Win32 앱도 MSIX로 Store에 배포할 수 있으므로 기존 재생기 설계를 유지합니다. Store는 패키지 호스팅과 업데이트를 제공하며, 실제 심사 결과는 제출 후 확인합니다. 근거: [Microsoft의 Win32 배포 방식 비교](https://learn.microsoft.com/en-us/windows/apps/distribute-through-store/how-to-distribute-your-win32-app-through-microsoft-store).

## 사용자가 보게 될 설치 흐름

1. GongPlayer의 Microsoft Store 페이지를 엽니다.
2. 설치 버튼으로 앱을 설치합니다.
3. 시작 메뉴에서 실행하거나, 영상 파일의 "연결 프로그램"에서 선택합니다.
4. 업데이트는 Store를 통해 받고, 제거는 Windows 설정에서 합니다.

Store 설치는 기존의 "다음 → 다음 → 완료" 마법사와 다른 표준 설치 흐름입니다. 별도 다운로드 파일로 설치하는 마법사까지 필요하면 보조 배포 채널을 추가합니다.

| 경로 | 역할 | 준비 사항 |
|---|---|---|
| Microsoft Store의 MSIX | 기본 공개 배포 | 패키지, Store 계정과 등록 자료, 심사 |
| GitHub Release의 설치 프로그램(Inno Setup)과 포터블 ZIP | 보조 배포 (D29) | 서명 없음(SmartScreen 경고), 사용자 단위 설치, 업데이트는 새 버전을 받아 덮어 설치. `scripts/package-release.ps1` |
| 직접 배포하는 MSIX | 필요 시 추가 | 신뢰할 수 있는 서명과 업데이트 경로. Store에 등록하는 것과 별도 |

## "알 수 없는 게시자"와 SmartScreen

게시자 확인, SmartScreen의 앱 평판, 관리자 권한을 묻는 UAC는 서로 다른 검사입니다.

- **Store의 MSIX**: Microsoft가 심사 후 패키지에 서명합니다. Store 설치 경로에서 SmartScreen 다운로드 경고를 피할 수 있고, 이 경로만을 위해 별도의 상용 서명 인증서를 구매할 필요가 없습니다.
- **Store의 MSI/EXE 등록**: MSIX와 달리 게시자가 설치 파일과 포함 PE 파일을 서명해야 합니다. Store가 설치 파일을 대신 재서명하지 않습니다.
- **홈페이지에서 직접 받은 설치 파일**: 유효한 코드 서명은 게시자를 확인하게 해 주지만, 새 파일에 대한 SmartScreen 경고까지 즉시 없애 주지는 않습니다.
- **관리자 권한**: GongPlayer는 일반 사용자 권한으로 실행하도록 설계합니다. 서명 유무와 관리자 권한 요청은 별개입니다.

근거: [Microsoft 코드 서명 안내](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options), [SmartScreen 평판 안내](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

일반 사용자에게 개발자 모드, 테스트 인증서 설치, 보안 기능 해제를 요구하지 않습니다. 개발용 패키지 테스트와 실제 Store 설치 검증을 구분하고, 기업의 설치 제한 정책까지 해제된다고 보장하지 않습니다.

## 패키지 설계

- 앱은 관리자 승격 없는 Win32 데스크톱 프로세스로 실행합니다.
- 실행 파일, 필요한 DLL, 아이콘, 고지문과 `AppxManifest.xml`을 패키지에 포함합니다.
- Store용 `Identity`의 Name과 Publisher는 Partner Center 값에 맞춥니다. 테스트용 식별자를 Store의 실제 값으로 간주하지 않습니다.
- 패키지의 버전, x64 아키텍처, 최소 Windows 버전을 명시합니다. `runFullTrust` 등 필요한 선언과 제출 설명을 준비합니다. 이는 관리자 승격을 뜻하지 않습니다.
- 시작 메뉴 표시와 파일 연결을 매니페스트로 선언하고, 기본 앱 선택은 사용자에게 맡깁니다.
- 설치 디렉터리에 쓰거나 현재 작업 디렉터리를 고정 경로로 가정하지 않습니다.
- FFmpeg 및 CRT 의존성을 포함하거나 지원되는 패키지 의존성으로 선언해, 개발 도구 없는 PC에서도 실행되게 합니다.

Windows SDK의 `MakeAppx.exe`로 패키지를 만드는 스크립트를 M0에서 검증합니다.

**2026-11-13 진행 상황 (M0 스파이크 C)**

- `scripts/package-msix.ps1`이 실제 플레이어로 MSIX를 만듭니다. 매니페스트 원본은 `packaging/AppxManifest.xml`이고 Identity·게시자·버전을 스크립트가 채웁니다. `runFullTrust`, 최소 Windows 10.0.19041, x64, 파일 연결(`.mp4 .m4v .mov .mkv .webm .ts .m2ts .mts .avi`, 경로를 `"%1"`로 첫 인자에 전달)을 선언했습니다. 로고는 `scripts/make-assets.ps1`이 도형으로 만듭니다.
- 개발용(FFmpeg DLL 포함)은 실행 코드 143MB, MSIX 압축 62MB입니다. 배포용 `-Static`(최소 구성 FFmpeg와 dav1d 정적 링크)은 실행 파일 하나 8.13MB, MSIX 압축 3.18MB, 설치 8.21MB입니다.
- 서명 없는 패키지는 설치되지 않습니다. Windows 11의 서명 없는 패키지 설치(`Add-AppxPackage -AllowUnsigned`, 게시자에 OID `2.25.311729368913984317654407730594956997722=1`)를 시도했지만 "서명되지 않은 패키지에 실행 가능한 활성화를 포함할 수 없다"(0x80073D2B)로 거부됐습니다.
- 그래서 개발용 설치는 `scripts/dev-install.ps1`로 합니다. 현재 사용자 저장소에 자체 서명 인증서(`CN=GongPlayer Dev`, 개인 키 내보내기 불가)를 만들어 서명하고, 그 공개 인증서를 "로컬 컴퓨터\신뢰할 수 있는 사용자"에 넣습니다. 이 단계에만 처음 한 번 관리자 승인(UAC)이 필요합니다. 기본은 배포용 정적 빌드로 설치하고, `-Dev`면 개발용 DLL 빌드로 설치합니다.
- **확인함 (2026-11-13, 기준 PC, 정적 빌드 패키지)**: 서명한 패키지가 일반 사용자 권한으로 설치되고(`C:\Program Files\WindowsApps\GongPlayer.Dev_…`), 앱 ID로 실행하면 빈 상태 화면이 뜹니다(시작 메뉴와 같은 경로). 패키지가 등록한 `.mp4` 파일 연결 ProgID로 `TestVideos/a.mp4`를 열면 `"…\GongPlayer.exe" "….mp4"`로 경로가 첫 인자에 전달되어 재생됩니다. 제거하면 패키지와 설치 폴더가 모두 사라지고, 다시 설치됩니다.
- 처음 시도에서 `dev-install.ps1`이 `package-msix.ps1`에 인자를 배열로 넘겨 게시자 자리에 `GongPlayer.Dev`가 들어가, 매니페스트 검증에서 `makeappx`가 실패했습니다. 해시테이블로 넘기도록 고쳤습니다.
- 업데이트: 같은 버전 번호로 내용을 바꾸면 Windows가 설치를 거부합니다. `dev-install.ps1`은 버전의 네 번째 자리를 매번 올려 덮어쓰기 업데이트를 하며, 0.1.0.0에서 0.1.0.992로 업데이트되는 것을 확인했습니다. Store 제출용은 네 번째 자리를 0으로 둡니다 (`package-msix.ps1 -Revision`의 기본값).
- 확인 못 함: 탐색기에서 사람이 "연결 프로그램"을 고르는 것 (같은 ProgID 경로), 개발 도구가 없는 깨끗한 Windows.
- Store 제출용 패키지는 Store가 서명하므로 이 인증서와 무관합니다. Identity Name과 Publisher는 Partner Center 값으로 `-Name`, `-Publisher`, `-PublisherDisplay`를 넘겨 만듭니다. 참고: [매니페스트와 패키지 구성](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion), [MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool), [패키징 전 점검](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-prepare), [파일 연결 등 Windows 통합](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions).

## 개발 단계에서 확인할 것

**M0**에서는 빈 창 수준의 작은 앱으로 패키징 경로를 확인합니다. 개발용 서명과 실제 Store 서명을 구분하며, 설치·시작 메뉴 실행·파일 경로 전달·제거까지 확인합니다. 최소 Windows 버전 후보와 패키지 크기 목표를 이때 정하고, 최소 버전의 실제 동작은 M4까지 검증합니다.

**M6**에서는 실제 플레이어로 깨끗한 Windows에서 설치, HEVC 재생, 업데이트, 제거를 검증합니다. Windows App Certification Kit 검사와 Store 제출 자료를 준비합니다. Store 제출과 승인 여부는 별도 상태로 기록하고, 개발용 설치 성공만으로 공개 배포 완료라고 하지 않습니다.

## Partner Center 진행 상황

- 2026-11-13: 회사 계정(letscoding)으로 개발자 등록을 마쳤습니다 (사업자등록증 제출, 수동 검토).
- 앱 이름 **GongPlayer**를 예약했습니다. 예약 후 3개월 안에 제출하지 않으면 예약이 풀립니다.
- Store ID `9PGQL0Q90CGF`, 링크 https://apps.microsoft.com/detail/9PGQL0Q90CGF (승인 전에는 열리지 않음)
- 패키지 Identity: Name `letscoding.GongPlayer`, Publisher `CN=B3FB09CC-66A1-4A25-AE74-EBFF02F06617`, PublisherDisplayName `letscoding`, PFN `letscoding.GongPlayer_ykcjhq69qknkg`. `scripts/package-msix.ps1 -Store`에 들어 있습니다.
- Store 제출용 패키지: `pwsh scripts/package-msix.ps1 -Store` → `dist/GongPlayer_<버전>_x64.msix` (정적 빌드, 서명 없음, 버전 네 번째 자리 0). 0.1.0은 압축 3.43MB입니다.
- 인증 키트 검사: `pwsh scripts/run-wack.ps1` (관리자 권한 필요). 2026-11-14 0.1.0.0 Store 패키지로 실행해 **전체 PASS (24개 항목 모두 통과, 경고 없음)**, Windows 11 Pro.
- 2026-11-14: 첫 제출(Submission 1)의 모든 항목을 채웠습니다. 계정 확인 경고도 사라졌습니다. 항목별로 실제 입력한 값과 파트너 센터에서 알게 된 점은 아래와 같습니다.
  - 가격 책정 및 가용성: 대한민국만, 앞으로 생길 시장은 자동 추가하지 않음, 무료, Store에 공개되고 검색 가능.
  - 속성: 사진 및 비디오. 패키지에 `runFullTrust`가 있으면 "개인 정보에 액세스하는가"가 "예"로 고정되므로 개인정보처리방침 URL이 필수입니다. 지원 정보는 웹 사이트, 이메일, 전화, 주소를 입력했습니다.
  - 연령별 등급: IARC 설문에서 기타 앱으로 답했고 모든 질문에 "아니요"를 골라 3+ 등급을 받았습니다.
  - 패키지: Validated. 앞으로 생길 장치 패밀리는 Microsoft가 정하도록 기본값을 그대로 두었습니다.
  - Store 목록(한국어): `docs/store/listing-ko.md`의 내용을 넣었고, 첫 제출이라 "이 버전의 새로운 기능"은 비워 두었습니다. 스크린샷 4장에 캡션을 붙였고, 로고는 `scripts/make-store-art.ps1`로 만든 9:16 포스터 아트와 1:1 박스 아트를 올렸습니다.
  - 제출 옵션: 제한된 기능(`runFullTrust`)의 사유를 묻는 칸은 500자로 제한됩니다. 심사 메모를 줄여 넣었습니다(`docs/store/certification-notes.md`의 첫 블록). 게시 시점은 게시자가 정합니다.
  - 추가적인 테스트 정보: 심사자에게만 보이는 "인증에 대한 참고 사항"은 제출 옵션이 아닌 이 페이지에 있습니다(6000자 제한). 심사 메모 전문을 넣었습니다.
- 2026-11-14: Submission 1을 "인증을 위해 제출"했습니다. 상태는 "인증 중(사전 처리 중)"이고, 게시 시작 시각은 2026-11-13 16:00 UTC로 잡혀 있습니다. 이미 지난 시각이므로 인증을 통과하면 바로 게시됩니다. 심사 결과는 아직 나오지 않았습니다.

## Partner Center와 Store 등록 절차

계정 만들기와 본인 확인은 게시자 본인만 할 수 있습니다. 순서와 나눈 역할입니다.

**사용자가 할 일**

1. https://partner.microsoft.com/dashboard/registration 에서 Microsoft 계정으로 앱 개발자 계정을 만듭니다. 개인 계정은 본인 확인(신분증, 얼굴 확인)을 거칩니다. 회사 계정은 회사 확인이 따로 있고 시간이 더 걸립니다. 계정 종류와 수수료는 등록 시점의 화면을 확인합니다.
2. 대시보드의 "앱 및 게임" → "새 제품" → "MSIX 또는 PWA 앱"에서 앱 이름(예: GongPlayer)을 예약합니다. 이름이 이미 쓰이고 있으면 다른 이름을 정합니다.
3. 예약한 앱의 "제품 관리" → "제품 ID"에서 아래 세 값을 복사해 알려 줍니다.
   - `Package/Identity/Name` (예: `12345Publisher.GongPlayer`)
   - `Package/Identity/Publisher` (예: `CN=XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`)
   - `Package/Properties/PublisherDisplayName`
4. 저장소를 원격에 올려(push) 개인정보처리방침 URL을 열 수 있게 합니다. 저장소가 공개이므로 `https://github.com/quirinal36/GongPlayer/blob/main/PRIVACY.md`를 씁니다. (현재 작업은 `feat/player-m0-m6` 브랜치에 있고, main에 합친 뒤에 이 주소가 열립니다.)

**값을 받은 뒤 할 일 (Claude)**

1. Store용 패키지를 만듭니다: `pwsh scripts/package-msix.ps1 -Static -Name <Identity Name> -Publisher "<Identity Publisher>" -PublisherDisplay "<표시 이름>"`. Store가 서명하므로 이 패키지는 서명하지 않습니다.
2. Windows App Certification Kit로 패키지를 검사하고 결과를 기록합니다.
3. Store 설명, 기능 목록, 검색어, 스크린샷(합성 영상으로 찍어 저작권 문제가 없게), 심사 메모(`runFullTrust`를 쓰는 이유: Win32 데스크톱 앱이며 관리자 권한은 요청하지 않음)를 준비합니다.

**정한 것**: 가격은 무료, 앱 내 구매·광고 없음 ([DECISIONS](DECISIONS.md)의 D27).

**함께 정할 것**: 배포 지역(코덱 특허 검토와 함께), 연령 등급 설문 답변, 지원 연락처.

개인정보처리방침 원문은 저장소 맨 위의 `PRIVACY.md`입니다. 앱이 네트워크를 쓰지 않고 아무것도 저장하지 않는다는 현재 동작을 그대로 적었습니다. 동작이 바뀌면 방침을 먼저 고칩니다.

## 공개 배포 전에 정할 것

- 게시자 계정의 개인/회사 구분, Store 앱 이름과 패키지 식별자
- 배포 지역, 지원 연락처와 지원 페이지 (가격은 무료로 정함)
- 실제 앱 동작에 맞는 개인정보처리방침, Store 설명, 아이콘과 스크린샷
- GongPlayer의 라이선스와 소스 공개 여부
- 배포하는 FFmpeg의 대응 소스, 고지문, 빌드 방법과 필요한 재링크 자료
- HEVC 등 포함 코덱의 배포 조건 검토 결과

라이선스의 구체적인 검토 항목은 [BUILD](BUILD.md)에 있습니다. 이 문서에서 상용 인증서 구매나 Store 계정 등록을 완료했다고 가정하지 않습니다.
