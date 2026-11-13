# 빌드와 테스트

개발 환경, FFmpeg를 구하는 방법, 테스트 방법, 라이선스를 적습니다. 2026-11-13에 기준 PC에서 실제로 빌드하고 실행해 확인한 내용으로 고쳤습니다.

## 빠른 시작

```powershell
pwsh scripts/fetch-ffmpeg.ps1        # deps.lock에 고정한 FFmpeg 사전 빌드를 .deps/ffmpeg에 받음
cargo test                           # 순수 로직 단위 테스트
cargo run -- <동영상 파일>            # 디버그 빌드 실행 (콘솔에 로그와 2초마다 재생 통계)
cargo build --release                # 릴리스 빌드 (target/release에 FFmpeg DLL이 함께 복사됨)
pwsh scripts/package-msix.ps1        # MSIX 생성 (dist/)
```

`.cargo/config.toml`이 `FFMPEG_DIR`(=.deps/ffmpeg)과 `LIBCLANG_PATH`를 정해 둡니다. 환경 변수로 이미 지정했다면 그 값이 우선합니다.

진단용 환경 변수입니다. 설정이 아니며 사용자에게 노출하지 않습니다.

| 변수 | 하는 일 |
|---|---|
| `GONG_TIMING` | 프로세스 생성부터 각 단계(GPU 준비, 파일 열기, 첫 프레임 표시 등)까지의 시간을 찍음 |
| `GONG_STATS` | 릴리스 빌드에서도 2초마다 재생 통계(진행 속도, 프레임 누락, 소리 시계 오차, 큐 상태)를 찍음 |
| `GONG_FORCE_SW` | 디버그 빌드에서 하드웨어 디코딩을 끄고 소프트웨어 경로를 시험함 |

## 대상

| 항목 | 값 |
|---|---|
| OS | Windows 10 이상 |
| 아키텍처 | x86_64 (ARM64는 나중 후보) |
| Rust 타깃 | `x86_64-pc-windows-msvc` |
| Rust 툴체인 | 1.93.1 (`rust-toolchain.toml`로 고정) |
| C 런타임 | 정적 링크 (`+crt-static`, [DECISIONS](DECISIONS.md)의 D18). VC++ 재배포 패키지 없이 실행. UCRT는 Windows 10부터 기본 포함 |

## 개발 흐름

현재 Windows PC에서 직접 개발하고 실행합니다 (2026-11-13 확정). CI는 Windows 러너에서 빌드와 자동 테스트를 반복하고, GPU와 출력 장치가 필요한 검증은 실제 PC에서 합니다.

2026-11-13 기준 PC에서 확인한 개발 도구입니다.

| 도구 | 버전 | 용도 |
|---|---|---|
| Rust, Cargo | 1.93.1, `x86_64-pc-windows-msvc` | 빌드 |
| MSVC | Visual Studio Community 2026 (MSVC 14.51), Build Tools 2022 (MSVC 14.44) | 링커, C 컴파일러 |
| Windows SDK | 10.0.26100.0 | `rc.exe`(아이콘·매니페스트·버전 리소스), `MakeAppx.exe`, `SignTool.exe` |
| LLVM | 23.1.5 (`winget install LLVM.LLVM`) | FFmpeg 바인딩 생성(bindgen)에 libclang 필요 |
| MSYS2 | msys2-runtime 3.6.10 (`winget install MSYS2.MSYS2`) | 최소 구성 FFmpeg 정적 빌드 (스파이크 B) |
| FFmpeg (개발용) | 9.0.2, BtbN `win64-lgpl-shared-9.0` | `scripts/deps.lock`으로 고정 |

순수 로직(시계, 큐, 링 버퍼, 상태 기계, 배치, 키 매핑, 색 판정)은 플랫폼 API와 분리합니다. 다른 OS에서도 단위 테스트할 수 있게 유지하되, 별도 크로스 빌드 흐름은 현재 필수 작업이 아닙니다.

Store 패키징과 설치 검증은 [DISTRIBUTION](DISTRIBUTION.md)을 따릅니다.

## 의존성

직접 의존 crate는 5개 이하로 유지합니다 ([PHILOSOPHY](PHILOSOPHY.md)의 예산). 현재 4개입니다.

| crate | 용도 |
|---|---|
| `windows` 0.62 | Win32, Direct3D 11, DXGI, Direct2D, DirectWrite, COM. COM 인터페이스가 필요해 `windows-sys`로는 안 됨 |
| `windows-numerics` 0.3 | Direct2D 좌표 타입(`Vector2`). `windows`가 이미 쓰는 crate라 크기가 늘지 않음 (D17) |
| `ffmpeg-sys-the-third` 6.0 | FFmpeg 9.0 원시 바인딩. 안전 래퍼(`ffmpeg-the-third`)는 쓰지 않음 (D16) |
| `cpal` 0.18 | 소리 출력 (WASAPI). 직접 구현으로 바꿀지는 [DECISIONS](DECISIONS.md)의 D12 참고 |

링크되는 crate 전체와 라이선스는 `THIRD_PARTY_NOTICES.md`에 있습니다 (`scripts/notices.ps1`로 생성).

## 스크립트

| 스크립트 | 하는 일 |
|---|---|
| `scripts/fetch-ffmpeg.ps1` | 개발용 FFmpeg 사전 빌드를 받고 sha256을 확인 |
| `scripts/make-test-media.ps1` | 합성 테스트 영상을 `.deps/media`에 생성 |
| `scripts/make-assets.ps1` | `packaging/source/icon-ball.png`로 패키지 로고와 실행 파일 아이콘을 생성 (`packaging/`, D28) |
| `scripts/build-ffmpeg-static.ps1` | 배포용 최소 구성 FFmpeg를 MSVC로 정적 빌드 (`.deps/ffmpeg-static`) |
| `scripts/package-msix.ps1` | 릴리스 빌드를 MSIX로 묶고 실행 코드·설치·압축 크기를 출력. `-Static`이면 정적 FFmpeg로 빌드 |
| `scripts/dev-install.ps1` | 개발용 자체 서명 인증서로 서명해 이 PC에 설치, `-Uninstall`로 제거 (처음 한 번 관리자 승인) |
| `scripts/size-report.ps1` | 실행 코드 합계를 예산(10MB)과 비교, `-Enforce`면 넘을 때 실패 |
| `scripts/package-release.ps1` | GitHub Release 파일 생성 (`dist/release`): 설치 프로그램, 포터블 ZIP, FFmpeg·dav1d 소스, SHA256SUMS. Inno Setup 6 필요 (`winget install JRSoftware.InnoSetup`, D29) |
| `scripts/notices.ps1` | `THIRD_PARTY_NOTICES.md` 생성 |

## FFmpeg

단계에 따라 두 가지를 씁니다.

### 개발 중: 사전 빌드 (공유 라이브러리)

BtbN의 `win64-lgpl-shared-9.0` 빌드를 `scripts/fetch-ffmpeg.ps1`로 받아 `.deps/ffmpeg`에 풉니다. URL과 sha256은 `scripts/deps.lock`에 고정해 CI와 개발 장비가 같은 파일을 씁니다. Concat의 `scripts/ci/deps.lock`과 같은 방식입니다. BtbN은 오래된 autobuild 릴리스를 지우므로, 받기가 실패하면 새 빌드로 바꾸고 sha256을 함께 고칩니다.

전체 구성이라 DLL 합계가 약 143MB입니다. M5까지는 빌드 시간을 아끼는 쪽이 낫습니다. 실행할 때는 DLL이 exe 옆에 있어야 하며, `build.rs`가 `target/<프로필>/`과 `deps/`에 복사합니다.

### 배포용: 최소 구성 정적 빌드

필요한 디먹서와 디코더만 넣어 MSVC로 직접 빌드하고 exe에 정적으로 링크합니다 (스파이크 B에서 확인, [DECISIONS](DECISIONS.md)의 D24).

```powershell
pwsh scripts/build-ffmpeg-static.ps1     # .deps/ffmpeg-static (약 4분). MSYS2에 make nasm diffutils pkgconf 필요
pwsh scripts/package-msix.ps1 -Static    # FFMPEG_DIR=.deps/ffmpeg-static, --features static-ffmpeg, target/static에 빌드 후 MSIX
pwsh scripts/size-report.ps1 -Static -Enforce
```

소스는 `scripts/deps.lock`의 `FFMPEG_SRC_URL`(ffmpeg-9.0.2.tar.xz)과 sha256으로 고정합니다. `build-ffmpeg-static.ps1`은 vswhere로 `vcvars64.bat`를 찾아 그 환경으로 MSYS2 bash를 띄우고, MSYS2의 `/usr/bin/link`가 MSVC `link.exe`를 가리지 않게 PATH를 정리합니다. 실제 configure는 다음과 같습니다.

```sh
configure --toolchain=msvc --arch=x86_64 --target-os=win64 --extra-cflags=-MT \
  --enable-static --disable-shared --disable-autodetect \
  --disable-everything --disable-programs --disable-doc --disable-debug \
  --disable-network --disable-avdevice --disable-swscale \
  --enable-protocol=file \
  --enable-demuxer=mov,matroska,mpegts,avi,mp3,aac,flac,wav,ogg \
  --enable-decoder=h264,hevc,vp9,av1,aac,mp3,ac3,eac3,opus,vorbis,flac,pcm_s16le,pcm_s24le,pcm_f32le \
  --enable-parser=h264,hevc,vp9,av1,aac,ac3,mpegaudio,opus,vorbis,flac \
  --enable-d3d11va \
  --enable-libdav1d --enable-decoder=libdav1d --pkg-config-flags=--static \
  --enable-hwaccel=h264_d3d11va,hevc_d3d11va,vp9_d3d11va,av1_d3d11va \
  --enable-hwaccel=h264_d3d11va2,hevc_d3d11va2,vp9_d3d11va2,av1_d3d11va2 \
  --enable-filter=atempo,aresample,aformat,abuffer,abuffersink,anull
```

정적 라이브러리 크기는 avcodec 11.2MB, avutil 2.4MB, avformat 2.2MB, avfilter 0.5MB, swresample 0.4MB이고, dav1d 3.8MB를 더해 LTO로 링크한 실행 파일은 8.13MB입니다 (dav1d 없이 6.35MB).

알아 둘 점입니다.

- MP4와 MOV는 `mov` 디먹서가, MKV와 WebM은 `matroska` 디먹서가 함께 처리합니다.
- FFmpeg 9.0.2에서 `*_d3d11va2`만 켜면 링크 오류(`LNK2001 ff_h264_d3d11va2_hwaccel`)가 납니다. 구현 파일이 `*_d3d11va`를 켤 때만 컴파일되므로 두 이름을 모두 켭니다.
- CRT는 `-MT`로 맞춥니다. Rust 쪽이 `+crt-static`이기 때문입니다 (D18).
- 정적 FFmpeg는 `bcrypt`가 필요합니다. `static-ffmpeg` 기능에서 `build.rs`가 링크합니다.
- FFmpeg 자체의 `av1` 디코더는 하드웨어 가속 전용입니다. 플레이어는 GPU의 AV1 디코더 프로파일을 먼저 확인하고, 없으면 `libdav1d`로 엽니다. 배포용 빌드는 dav1d 1.5.4를 meson과 MSVC로 정적 빌드해 넣습니다 (D25). `build-ffmpeg-static.ps1`이 meson·ninja를 `.deps/pytools` 가상 환경에 설치하고 MSYS2의 nasm을 씁니다. 기준 PC(RTX 2060, AV1 하드웨어 없음)에서 4K 10bit AV1을 소프트웨어로 30fps 재생했습니다.
- `--enable-gpl`과 `--enable-nonfree`는 쓰지 않습니다. 디코딩에는 필요 없고, 쓰면 라이선스가 바뀝니다.
- 소프트웨어 폴백의 색 형식 변환은 직접 하므로 `swscale`이 필요 없습니다 (D19).
- 필터 그래프에는 `abuffer`, `abuffersink`, `atempo`, `aresample`, `aformat`이 필요합니다.

### 툴체인 조합 (M0 스파이크 B)

정적 라이브러리는 그것을 만든 컴파일러와 Rust 타깃이 맞아야 링크됩니다. 후보는 셋입니다.

| 조합 | 설명 |
|---|---|
| MSVC 타깃 + MSVC로 빌드한 FFmpeg | Windows 표준. FFmpeg를 MSYS2 셸에서 `--toolchain=msvc`로 빌드 |
| GNU 타깃 + MinGW로 빌드한 FFmpeg | 다른 OS에서 크로스 빌드하기 쉬움 |
| 정적 링크 포기 | 최소 구성 DLL을 exe 옆에 두고 같은 MSIX에 포함. 사용자에게는 하나의 앱으로 설치 |

**결론 (2026-11-13)**: 첫 번째 조합(MSVC 타깃 + MSVC로 빌드한 정적 FFmpeg)으로 링크와 재생을 확인했습니다 (D24).

## 릴리스 프로필

디코딩 성능은 FFmpeg 안에 있고 Rust 코드는 이음매이므로 크기 쪽으로 최적화합니다. M6에서 실측해 조정합니다. 2026-11-13 현재 릴리스 실행 파일(정적 CRT, 리소스 포함)은 457KB입니다.

```toml
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

## 테스트

### 자동 (CI, 어느 OS에서나)

- 단위 테스트: 시계, 큐와 세대 번호, 링 버퍼, 탐색 병합, 배속 단계, 컨트롤 배치와 클릭 위치 판정, 키 매핑, 색공간 판정, 소프트웨어 프레임 변환(NV12/P010) (`cargo test`, 2026-11-13 기준 65개)
- 소프트웨어 디코딩 스모크 테스트: 몇 초짜리 합성 영상을 끝까지 디코딩 (아직 없음)
- M6부터: 실행 코드 크기 예산 검사, MSIX 생성과 패키지 크기 기록 (`.github/workflows/ci.yml`에 기록 단계까지 작성)

스모크 테스트용 영상은 FFmpeg로 직접 만들어 저작권 문제가 없게 합니다. `scripts/make-test-media.ps1`이 합성 영상들을 `.deps/media`에 만듭니다 (전체 구성 ffmpeg.exe가 PATH에 있어야 함). 실제 파일의 까다로운 성질(아이폰 HDR, 카메라 4:2:2 등)은 대신하지 못합니다.

```sh
ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=30 -f lavfi -i sine=frequency=440 \
  -t 3 -c:v libx265 -pix_fmt yuv420p10le -tag:v hvc1 -c:a aac smoke_hevc10.mp4
```

### 수동 (실제 Windows PC)

CI 러너에는 GPU가 없어 하드웨어 디코딩과 화면 출력은 자동으로 확인할 수 없습니다. 각 단계의 완료 기준은 실제 PC에서 확인합니다.

**기준 PC** (2026-11-13, 개발 PC):

| 항목 | 값 |
|---|---|
| CPU | AMD Ryzen 7 5700X (8코어 16스레드) |
| GPU | NVIDIA GeForce RTX 2060, 드라이버 32.0.15.9649. AV1 하드웨어 디코딩 없음 |
| 메모리 | 32GB |
| 모니터 | 2560x1440, 60Hz (Windows 보고 59), 배율 100% |
| Windows | Windows 11 Pro 10.0.26300 |
| 저장 장치 | SSD |

패키징은 M0에서 최소 앱으로 미리 검증합니다. M6에서는 개발 도구와 별도 코덱이 없는 Windows에서 설치, 시작 메뉴 실행, 파일 연결, 업데이트, 제거를 확인합니다. 개발용 인증서로 설치한 결과와 Store에서 설치한 결과는 별도로 기록합니다.

### 테스트 영상 세트

합성 영상에는 실제 파일의 까다로운 성질이 없습니다. 아래는 실제 파일로 모읍니다. 저장소에는 넣지 않습니다.

| 번호 | 영상 | 확인하는 것 |
|---|---|---|
| 1 | H.264 8bit 1080p 30fps, MP4 | 기준 |
| 2 | HEVC 8bit 4K, MP4 | 기본 목표 |
| 3 | HEVC 10bit HDR, 아이폰 촬영 MOV | HLG와 Dolby Vision의 SDR 변환 |
| 4 | HEVC 10bit 4:2:2, 카메라 촬영 | 소프트웨어 폴백 |
| 5 | AV1, MP4 또는 MKV | 하드웨어 지원 여부에 따른 분기 |
| 6 | VP9, WebM | |
| 7 | 세로로 찍은 휴대폰 영상 | 회전 메타데이터 |
| 8 | 화면 녹화 영상 | 가변 프레임레이트 |
| 9 | 1080p 60fps | 프레임 누락 측정 |
| 10 | 5.1 채널 AC-3, 소리 트랙 2개 이상인 MKV | 다운믹스, 기본 트랙 선택 |
| 11 | 소리 없는 영상 | 벽시계 재생 |
| 12 | 2시간 이상인 파일 | 탐색, 장시간 동기 |
| 13 | 중간에 잘리거나 손상된 파일 | 멈추지 않는지 |
| 14 | 립싱크 테스트 영상 | 동기 오차 측정 |

2026-11-13 현재 실제 파일은 모으지 못했고, 같은 성질을 흉내 낸 합성 영상으로만 확인했습니다 (`scripts/make-test-media.ps1`). 합성 영상으로 확인한 범위는 [ROADMAP](ROADMAP.md)의 각 단계 "확인 기록"에 적습니다.

## 라이선스

배포 전에 다시 확인해야 하는 사항을 정리한 것이며, 법률 자문은 아닙니다.

| 대상 | 라이선스 | 지켜야 할 것 |
|---|---|---|
| FFmpeg (GPL 옵션 없이 빌드) | LGPL 2.1 이상 | 고지문 포함. 정적 링크라면 사용자가 FFmpeg를 바꿔 다시 링크할 수 있어야 함 |
| dav1d 1.5.4 | BSD 2-Clause | 고지문과 원문 포함 (`DAV1D_LICENSE.txt`) |
| `windows`, `cpal` 등 crate | MIT 또는 Apache-2.0 | 고지문 포함 (`THIRD_PARTY_NOTICES.md`) |
| `ffmpeg-sys-the-third` | WTFPL | 고지문 포함 |
| Concat | AGPL 3.0 이상 | 코드를 복사하지 않음. 참고만 |

- 소스 공개만으로 LGPL 조건이 자동 충족된다고 판단하지 않습니다. 배포한 FFmpeg에 대응하는 소스, 변경 사항, 빌드 방법, 고지문을 준비하고, 정적 링크를 선택하면 사용자가 수정한 라이브러리로 다시 링크할 수 있는 자료와 절차도 검증합니다. DLL 방식에도 라이선스 의무는 남습니다.
- 공개 배포가 목표이므로 HEVC 등 포함 코덱의 특허·라이선스 검토를 M0부터 시작합니다. 배포 지역과 유·무료 여부 등 실제 조건을 기준으로 확인하며, FFmpeg의 오픈소스 라이선스와 Store 심사가 코덱 특허 이용 허락을 대신한다고 가정하지 않습니다.
- GongPlayer 자체는 MPL-2.0입니다 (`LICENSE`, [DECISIONS](DECISIONS.md)의 D26). 소스가 공개되어 있어 FFmpeg를 바꿔 다시 링크할 수 있습니다. 배포할 때는 대응하는 FFmpeg·dav1d 소스 압축 파일을 같은 Release에 올립니다.
- 개발용 패키지는 BtbN 빌드를 그대로 싣고 그 `LICENSE.txt`를 `FFMPEG_LICENSE.txt`로 함께 넣습니다. 배포용 정적 패키지(`-Static`)는 FFmpeg 소스의 `COPYING.LGPLv2.1`을 `FFMPEG_LICENSE.txt`로 넣고, `THIRD_PARTY_NOTICES.md`에 소스 URL, sha256, 빌드 스크립트 위치를 적습니다.
- 정적 링크는 재링크 자료가 필요합니다. GongPlayer는 소스를 공개하므로 소스와 빌드 방법으로 제공합니다 (D24, D26).

참고: [FFmpeg 공식 라이선스 안내](https://ffmpeg.org/legal.html). 공개 배포 전에 적용 조건을 별도로 확인합니다.
