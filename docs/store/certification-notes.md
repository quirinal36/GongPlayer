# 심사 메모 (Notes for certification)

심사자가 영어로 읽을 가능성이 커서 영어로 씁니다. 파트너 센터에서 두 곳에 넣습니다.

## 제출 옵션 → 제한된 기능 (500자 제한)

"runFullTrust 기능이 필요한 이유"를 묻는 필수 칸입니다.

```text
GongPlayer is a classic Win32 desktop video player (Rust, FFmpeg, Direct3D 11) packaged as MSIX. It uses D3D11 hardware video decoding, WASAPI audio and a Win32 window, so it runs as a full trust desktop process. It runs asInvoker (no elevation), installs no drivers or services, and does not write to its install folder. It makes no network connections and collects no data. It only opens local video files the user chooses.
```

## 추가적인 테스트 정보 → 인증에 대한 참고 사항 (6000자 제한)

제품 페이지 왼쪽 아래 "추가 정보 → 추가적인 테스트 정보"에 있습니다. 고객에게는 보이지 않습니다.

```text
GongPlayer is a lightweight local video player (Win32 desktop app, Rust + FFmpeg + Direct3D 11) packaged as MSIX.

Why runFullTrust:
GongPlayer is a classic Win32 desktop application. It uses Direct3D 11 hardware video decoding (D3D11VA), WASAPI audio output and a Win32 window. The runFullTrust capability is only required because it is a desktop (full trust) process. It does NOT request administrator elevation (requestedExecutionLevel = asInvoker), does not install drivers or services, and does not write to its install folder.

Network and data:
The app makes no network connections at all. No telemetry, analytics, ads or in-app purchases. It does not store any settings or history. Privacy policy: https://github.com/quirinal36/GongPlayer/blob/main/PRIVACY.md

How to test:
1. Launch GongPlayer from Start. An empty window shows "파일을 끌어다 놓거나 Ctrl+O" (drag a file here or press Ctrl+O).
2. Press Ctrl+O and open any local video (MP4/MKV/MOV; H.264, HEVC, AV1 or VP9). Playback starts immediately.
3. Keys: Space = play/pause, Left/Right = seek 5 s, [ ] = speed 0.5x-4x, Up/Down = volume, F = fullscreen, Esc = exit fullscreen.
4. The app is also registered in "Open with" for .mp4 .m4v .mov .mkv .webm .ts .m2ts .mts .avi (it does not change default apps).
No account, sign-in or test data is required.

Third-party components:
FFmpeg (LGPL 2.1+, statically linked, built without GPL/nonfree components) and dav1d (BSD 2-Clause). Corresponding sources and relinking instructions are published in the GitHub repository and releases. Additional license terms are provided in the listing.

Windows App Certification Kit: PASS (all 24 tests) on Windows 11 Pro with package 0.1.0.0.
```
