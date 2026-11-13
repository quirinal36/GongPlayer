# 빌드와 테스트

개발 환경, FFmpeg를 구하는 방법, 테스트 방법, 라이선스를 적습니다. 아직 한 번도 빌드해 보지 않은 상태에서 쓴 문서입니다. "M0에서 확인"이라고 표시한 부분은 실제로 해 본 뒤 고칩니다.

## 대상

| 항목 | 값 |
|---|---|
| OS | Windows 10 이상 |
| 아키텍처 | x86_64 (ARM64는 나중 후보) |
| Rust 타깃 | `x86_64-pc-windows-msvc` (M0 스파이크 B에서 확정) |

## 개발 흐름

이 프로그램은 Windows에서만 실행됩니다. 개발 장비가 macOS라면 흐름을 정해야 합니다.

| 방식 | 장점 | 단점 |
|---|---|---|
| **Windows PC에서 직접 개발** (권장) | 고치고 바로 실행. 하드웨어 디코딩까지 확인 | 개발 환경을 Windows에 새로 갖춰야 함 |
| macOS에서 편집, Windows에서 실행 | 익숙한 환경 유지 | 실행할 때마다 옮겨야 함. 화면 관련 버그를 잡기 느림 |
| CI가 만든 실행 파일을 받아 실행 | 설정이 가장 적음 | 한 번 확인에 수 분 |

어느 쪽이든 순수 로직(시계, 큐, 배치)의 단위 테스트는 macOS에서도 돌 수 있게 유지합니다.

> **M0에서 확인.** macOS에서 Windows 타깃으로 `cargo check`가 되는지, 된다면 FFmpeg 바인딩 생성까지 되는지.

## 의존성

직접 의존 crate는 5개 이하로 유지합니다 ([PHILOSOPHY](PHILOSOPHY.md)의 예산).

| crate | 용도 |
|---|---|
| `windows` | Win32, Direct3D 11, DXGI, Direct2D, DirectWrite. COM 인터페이스가 필요해 `windows-sys`로는 안 됨 |
| `ffmpeg-the-third` | FFmpeg 바인딩. 하드웨어 컨텍스트처럼 안전 래퍼에 없는 부분은 함께 오는 sys crate로 직접 호출 |
| `cpal` | 소리 출력 (WASAPI). 직접 구현으로 바꿀지는 [DECISIONS](DECISIONS.md) 참고 |

## FFmpeg

단계에 따라 두 가지를 씁니다.

### 개발 중: 사전 빌드 (공유 라이브러리)

BtbN의 `win64-lgpl-shared` 빌드를 받아 풀고 `FFMPEG_DIR`로 가리킵니다. 받을 URL과 sha256을 저장소에 고정해 두고 CI와 개발 장비가 같은 파일을 쓰게 합니다. Concat의 `scripts/ci/deps.lock`과 같은 방식입니다.

전체 구성이라 DLL 크기는 크지만, M5까지는 빌드 시간을 아끼는 쪽이 낫습니다. 실행할 때는 DLL이 exe 옆에 있어야 합니다.

### 배포용: 최소 구성 정적 빌드

필요한 디먹서와 디코더만 넣어 직접 빌드하고 exe에 정적으로 링크합니다. 아래는 출발점이고, M0 스파이크 B와 M6에서 실제로 돌려 확정합니다.

```sh
./configure \
  --disable-everything --disable-programs --disable-doc \
  --disable-network --disable-avdevice \
  --enable-static --disable-shared \
  --enable-protocol=file \
  --enable-demuxer=mov,matroska,mpegts,avi,mp3,aac,flac,wav,ogg \
  --enable-decoder=h264,hevc,vp9,av1,aac,mp3,ac3,eac3,opus,vorbis,flac \
  --enable-decoder=pcm_s16le,pcm_s24le,pcm_f32le \
  --enable-parser=h264,hevc,vp9,av1,aac,ac3,mpegaudio,opus,vorbis,flac \
  --enable-d3d11va \
  --enable-hwaccel=h264_d3d11va2,hevc_d3d11va2,vp9_d3d11va2,av1_d3d11va2 \
  --enable-filter=atempo,aresample,aformat
```

알아 둘 점입니다.

- MP4와 MOV는 `mov` 디먹서가, MKV와 WebM은 `matroska` 디먹서가 함께 처리합니다.
- FFmpeg 자체의 `av1` 디코더는 하드웨어 가속 전용입니다. AV1을 소프트웨어로 디코딩하려면 `dav1d`를 추가로 넣어야 하고 그만큼 커집니다.
- `--enable-gpl`과 `--enable-nonfree`는 쓰지 않습니다. 디코딩에는 필요 없고, 쓰면 라이선스가 바뀝니다.
- 소프트웨어 폴백에서 색 형식 변환이 필요하면 `swscale`을 켭니다. 직접 변환으로 대체할 수 있는지는 M6에서 크기를 보고 정합니다.

### 툴체인 조합 (M0 스파이크 B)

정적 라이브러리는 그것을 만든 컴파일러와 Rust 타깃이 맞아야 링크됩니다. 후보는 셋입니다.

| 조합 | 설명 |
|---|---|
| MSVC 타깃 + MSVC로 빌드한 FFmpeg | Windows 표준. FFmpeg를 MSYS2 셸에서 `--toolchain=msvc`로 빌드 |
| GNU 타깃 + MinGW로 빌드한 FFmpeg | 다른 OS에서 크로스 빌드하기 쉬움 |
| 정적 링크 포기 | 최소 구성 DLL을 exe 옆에 둠. 단일 exe 목표는 내려놓음 |

## 릴리스 프로필

디코딩 성능은 FFmpeg 안에 있고 Rust 코드는 이음매이므로 크기 쪽으로 최적화합니다. M6에서 실측해 조정합니다.

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

- 단위 테스트: 시계, 큐와 세대 번호, 탐색 병합, 배속 단계, 컨트롤 배치와 클릭 위치 판정
- 소프트웨어 디코딩 스모크 테스트: 몇 초짜리 합성 영상을 끝까지 디코딩
- M6부터: 실행 파일 크기 검사

스모크 테스트용 영상은 FFmpeg로 직접 만들어 저작권 문제가 없게 합니다.

```sh
ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=30 -f lavfi -i sine=frequency=440 \
  -t 3 -c:v libx265 -pix_fmt yuv420p10le -tag:v hvc1 -c:a aac smoke_hevc10.mp4
```

### 수동 (실제 Windows PC)

CI 러너에는 GPU가 없어 하드웨어 디코딩과 화면 출력은 자동으로 확인할 수 없습니다. 각 단계의 완료 기준은 실제 PC에서 확인합니다.

**기준 PC**: M0에서 정해 여기에 적습니다. (CPU, GPU, 드라이버 버전, 모니터 주사율, Windows 버전)

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

## 라이선스

배포 전에 다시 확인해야 하는 사항을 정리한 것이며, 법률 자문은 아닙니다.

| 대상 | 라이선스 | 지켜야 할 것 |
|---|---|---|
| FFmpeg (GPL 옵션 없이 빌드) | LGPL 2.1 이상 | 고지문 포함. 정적 링크라면 사용자가 FFmpeg를 바꿔 다시 링크할 수 있어야 함 |
| dav1d (넣는다면) | BSD 2-Clause | 고지문 포함 |
| `windows`, `cpal` 등 crate | MIT 또는 Apache-2.0 | 고지문 포함 |
| Concat | AGPL 3.0 이상 | 코드를 복사하지 않음. 참고만 |

- GongPlayer의 소스를 공개하면 정적 링크의 재링크 조건은 자연히 충족됩니다. 소스를 공개하지 않고 배포한다면 FFmpeg를 DLL로 두는 편이 간단합니다.
- HEVC에는 특허 풀이 있습니다. 개인 용도로 쓰는 데는 문제가 없지만, 불특정 다수에게 배포할 계획이면 따로 검토합니다.
- GongPlayer 자체의 라이선스는 아직 정하지 않았습니다 ([DECISIONS](DECISIONS.md)의 미결 항목).
