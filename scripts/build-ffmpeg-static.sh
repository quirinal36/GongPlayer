#!/usr/bin/env bash
# 배포용 최소 구성 FFmpeg를 MSVC로 정적 빌드해 .deps/ffmpeg-static에 설치합니다.
# 직접 부르지 말고 scripts/build-ffmpeg-static.ps1로 실행합니다. (MSVC 환경을 넣은 뒤 MSYS2 bash로 이 파일을 부릅니다.)
# 필요한 MSYS2 패키지: make nasm diffutils pkgconf
#
# 결과: .deps/ffmpeg-static/{include,lib}
#   lib에는 configure가 만든 libavcodec.a 등과, ffmpeg-sys-the-third가 찾는 이름(avcodec.lib 등)의 사본이 함께 있습니다.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT/scripts/deps.lock"
lock() { grep -E "^$1=" "$LOCK" | head -n1 | cut -d= -f2- | tr -d '\r'; }
SRC_URL="$(lock FFMPEG_SRC_URL)"
SRC_SHA="$(lock FFMPEG_SRC_SHA256)"
[ -n "$SRC_URL" ] && [ -n "$SRC_SHA" ] || { echo "deps.lock에 FFMPEG_SRC_URL/FFMPEG_SRC_SHA256이 없습니다" >&2; exit 1; }

DEPS="$ROOT/.deps"
TARBALL="$DEPS/src/$(basename "$SRC_URL")"
SRC_DIR="$DEPS/src/$(basename "$SRC_URL" .tar.xz)"
BUILD_DIR="$DEPS/build/ffmpeg-static"
PREFIX="$DEPS/ffmpeg-static"
JOBS="${JOBS:-$(nproc)}"

# MSYS2의 /usr/bin/link(coreutils)가 MSVC link.exe를 가리지 않게 cl.exe가 있는 폴더를 PATH 앞에 둡니다.
CL="$(command -v cl.exe || true)"
[ -n "$CL" ] || { echo "cl.exe가 PATH에 없습니다. build-ffmpeg-static.ps1로 실행하세요." >&2; exit 1; }
export PATH="$(dirname "$CL"):$PATH"
case "$(command -v link.exe)" in
    /usr/bin/*) echo "link.exe가 MSYS2 것으로 잡힙니다: $(command -v link.exe)" >&2; exit 1 ;;
esac
for t in make nasm diff pkgconf; do
    command -v "$t" >/dev/null || { echo "$t 없음: pacman -S --needed make nasm diffutils pkgconf" >&2; exit 1; }
done

# 소스 받기와 검증
mkdir -p "$DEPS/src"
if [ ! -f "$TARBALL" ] || ! echo "$SRC_SHA  $TARBALL" | sha256sum -c --status; then
    echo "Downloading $SRC_URL"
    curl -fL --retry 3 -o "$TARBALL.part" "$SRC_URL"
    mv "$TARBALL.part" "$TARBALL"
fi
echo "$SRC_SHA  $TARBALL" | sha256sum -c --status || { echo "sha256 불일치: $TARBALL" >&2; exit 1; }
rm -rf "$SRC_DIR"
tar -xf "$TARBALL" -C "$DEPS/src"

# 빌드 (소스 밖 빌드 디렉터리)
rm -rf "$BUILD_DIR" "$PREFIX"
mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

# -MT: GongPlayer는 C 런타임을 정적으로 링크합니다 (+crt-static, docs/DECISIONS.md의 D18).
#      FFmpeg도 같은 정적 CRT로 맞춥니다. 섞이면 LNK4098 경고와 런타임 충돌이 생깁니다.
# *_d3d11va도 켜는 이유: FFmpeg 9.0.2 libavcodec/Makefile은 dxva2_h264.o 등을 *_d3d11va(구형 API)가 켜졌을 때만
#      빌드하는데, 우리가 쓰는 *_d3d11va2 정의도 같은 파일에 있습니다. 2만 켜면 ff_h264_d3d11va2_hwaccel 등이 링크되지 않습니다.
# --disable-autodetect: 빌드 장비에 깔린 라이브러리(zlib, schannel 등)가 몰래 들어오지 않게 막고, 필요한 것만 켭니다.
"$SRC_DIR/configure" \
    --prefix="$PREFIX" \
    --toolchain=msvc \
    --arch=x86_64 \
    --target-os=win64 \
    --extra-cflags=-MT \
    --enable-static --disable-shared \
    --disable-autodetect \
    --disable-everything --disable-programs --disable-doc --disable-debug \
    --disable-network --disable-avdevice --disable-swscale \
    --enable-protocol=file \
    --enable-demuxer=mov,matroska,mpegts,avi,mp3,aac,flac,wav,ogg \
    --enable-decoder=h264,hevc,vp9,av1,aac,mp3,ac3,eac3,opus,vorbis,flac,pcm_s16le,pcm_s24le,pcm_f32le \
    --enable-parser=h264,hevc,vp9,av1,aac,ac3,mpegaudio,opus,vorbis,flac \
    --enable-d3d11va \
    --enable-hwaccel=h264_d3d11va,hevc_d3d11va,vp9_d3d11va,av1_d3d11va \
    --enable-hwaccel=h264_d3d11va2,hevc_d3d11va2,vp9_d3d11va2,av1_d3d11va2 \
    --enable-filter=atempo,aresample,aformat,abuffer,abuffersink,anull

make -j"$JOBS"
make install

# ffmpeg-sys-the-third는 static=avcodec로 링크하므로 MSVC 링커는 avcodec.lib를 찾습니다.
# FFmpeg 9의 msvc 툴체인은 이미 avcodec.lib로 설치하지만, libavcodec.a로 나오는 판이면 이름을 맞춥니다.
cd "$PREFIX/lib"
shopt -s nullglob
for a in lib*.a; do
    n="${a#lib}"; cp -f "$a" "${n%.a}.lib"
done
for l in avutil avcodec avformat avfilter swresample; do
    [ -f "$l.lib" ] || { echo "$l.lib가 없습니다" >&2; exit 1; }
done
echo "$SRC_SHA" > "$PREFIX/.sha256"
echo "FFmpeg static ready: $PREFIX"
ls -l "$PREFIX/lib"
