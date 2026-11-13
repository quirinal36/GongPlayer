# 합성 테스트 영상을 .deps/media에 만듭니다. 저작권 문제가 없는 lavfi 소스만 씁니다.
# 전체 구성 ffmpeg.exe(libx264, libx265, libvpx, libsvtav1, libopus 포함)가 PATH에 있어야 합니다.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root '.deps\media'
New-Item -ItemType Directory -Force $out | Out-Null
Set-Location $out
# PowerShell 함수로 감싸면 -c:v 같은 인자가 쪼개지므로 ffmpeg를 직접 부르고 결과만 확인합니다.
function ck { if ($LASTEXITCODE -ne 0) { throw "ffmpeg 실패" } }
$src = 'testsrc2=size={0}:rate={1}'
ffmpeg -v error -y -f lavfi -i ($src -f '1920x1080', 30) -f lavfi -i 'sine=frequency=440:beep_factor=4' -t 20 -c:v libx264 -pix_fmt yuv420p -g 60 -c:a aac h264_1080p30.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1920x1080', 60) -f lavfi -i 'sine=frequency=330' -t 60 -c:v libx264 -pix_fmt yuv420p -c:a aac h264_1080p60.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i 'sine=frequency=440:beep_factor=4' -t 90 -c:v libx264 -pix_fmt yuv420p -g 60 -c:a aac long90.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '3840x2160', 30) -f lavfi -i 'sine=frequency=330' -t 10 -c:v libx265 -preset ultrafast -pix_fmt yuv420p10le -tag:v hvc1 -c:a aac -x265-params log-level=error hevc10_4k.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '3840x2160', 60) -f lavfi -i 'sine=frequency=330' -t 30 -c:v libx265 -preset ultrafast -pix_fmt yuv420p10le -tag:v hvc1 -c:a aac -x265-params log-level=error hevc10_4k60.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i 'sine=frequency=220' -t 10 -c:v libx265 -preset ultrafast -pix_fmt yuv422p10le -c:a libopus -x265-params log-level=error hevc422_10.mkv; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 60) -t 10 -c:v libx264 -pix_fmt yuv420p silent_720p60.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i 'sine=frequency=300' -t 10 -filter_complex '[1:a]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a]' -map 0:v -map '[a]' -c:v libx264 -pix_fmt yuv420p -c:a ac3 -f matroska ac3_51.mkv; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i 'sine=frequency=500' -t 8 -c:v libx264 -pix_fmt yuv420p -c:a aac rot_src.mp4; ck
ffmpeg -v error -y -display_rotation 90 -i rot_src.mp4 -c copy rotated90.mp4; ck
Remove-Item rot_src.mp4
ffmpeg -v error -y -f lavfi -i ($src -f '1920x1080', 30) -t 6 -vf format=yuv420p10le -c:v libx265 -preset ultrafast -x265-params 'log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc' -color_primaries bt2020 -color_trc arib-std-b67 -colorspace bt2020nc -tag:v hvc1 hlg.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1920x1080', 30) -t 6 -vf format=yuv420p10le -c:v libx265 -preset ultrafast -x265-params 'log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc' -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -tag:v hvc1 pq.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i sine -t 10 -vf "setpts='if(lt(N,150),PTS,PTS*2)'" -fps_mode vfr -c:v libx264 -pix_fmt yuv420p -c:a aac vfr.mp4; ck
ffmpeg -v error -y -f lavfi -i ($src -f '640x360', 30) -f lavfi -i 'sine=frequency=700' -t 6 -c:v libvpx-vp9 -b:v 1M -c:a libopus vp9.webm; ck
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -t 5 -c:v libsvtav1 -preset 12 -pix_fmt yuv420p av1.mp4; ck
# 끝이 잘린 파일: moov가 앞에 있는 MP4를 중간에서 자릅니다 (열려야 함).
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i sine -t 20 -c:v libx264 -pix_fmt yuv420p -c:a aac -movflags +faststart faststart.mp4; ck
$bytes = [IO.File]::ReadAllBytes((Join-Path $out 'faststart.mp4'))
[IO.File]::WriteAllBytes((Join-Path $out 'truncated_faststart.mp4'), $bytes[0..([int]($bytes.Length * 0.5))])
# 중간이 손상된 파일: MKV의 가운데 일부를 무작위 바이트로 덮습니다.
ffmpeg -v error -y -f lavfi -i ($src -f '1280x720', 30) -f lavfi -i sine -t 20 -c:v libx264 -pix_fmt yuv420p -c:a aac corrupt_src.mkv; ck
$bytes = [IO.File]::ReadAllBytes((Join-Path $out 'corrupt_src.mkv'))
$rng = New-Object Random 42
$mid = [int]($bytes.Length * 0.4)
for ($i = 0; $i -lt 200000; $i++) { $bytes[$mid + $i] = [byte]$rng.Next(256) }
[IO.File]::WriteAllBytes((Join-Path $out 'corrupt_middle.mkv'), $bytes)
Remove-Item corrupt_src.mkv
# moov가 끝에 있는 MP4를 자른 파일: 열 수 없어야 하고, 화면에 한 줄로 알려야 함
$bytes = [IO.File]::ReadAllBytes((Join-Path $out 'h264_1080p30.mp4'))
[IO.File]::WriteAllBytes((Join-Path $out 'truncated.mp4'), $bytes[0..5999999])
"합성 영상 준비: $out"
