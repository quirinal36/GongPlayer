#!/bin/sh
# Jetson 데스크톱용 패키지. Debian 또는 Ubuntu에서 실행합니다.
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
install -Dm755 linux/gongplayer "$stage/usr/bin/gongplayer"
install -Dm644 linux/gongplayer.desktop "$stage/usr/share/applications/gongplayer.desktop"
install -Dm644 packaging/Assets/Square44x44Logo.targetsize-256_altform-unplated.png "$stage/usr/share/icons/hicolor/256x256/apps/gongplayer.png"
install -Dm644 LICENSE "$stage/usr/share/doc/gongplayer/copyright"
mkdir -p "$stage/DEBIAN" dist
cat > "$stage/DEBIAN/control" <<EOF
Package: gongplayer
Version: $version
Architecture: arm64
Maintainer: GongPlayer <contact@letscoding.kr>
Section: video
Priority: optional
Depends: python3, python3-gi, gir1.2-gtk-3.0, gir1.2-gstreamer-1.0, gstreamer1.0-gtk3, gstreamer1.0-plugins-base, gstreamer1.0-plugins-good, gstreamer1.0-plugins-bad, gstreamer1.0-libav, gstreamer1.0-pulseaudio
Description: Local video player for the Jetson Linux desktop
 Play, seek and repeat a single video using GTK and GStreamer.
EOF
dpkg-deb --root-owner-group -Zxz --build "$stage" "dist/GongPlayer-Jetson-arm64.deb"
