# deps.lock에 고정한 FFmpeg 사전 빌드를 .deps/ffmpeg에 받아 풉니다.
# 사용: pwsh scripts/fetch-ffmpeg.ps1   이후 FFMPEG_DIR=<저장소>/.deps/ffmpeg
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$lock = @{}
Get-Content (Join-Path $PSScriptRoot 'deps.lock') | Where-Object { $_ -match '^[A-Z_0-9]+=' } | ForEach-Object {
    $k, $v = $_ -split '=', 2; $lock[$k] = $v.Trim()
}
$deps = Join-Path $root '.deps'
$dest = Join-Path $deps 'ffmpeg'
$stamp = Join-Path $dest '.sha256'
if ((Test-Path $stamp) -and ((Get-Content $stamp) -eq $lock.FFMPEG_SHA256)) {
    Write-Host "FFmpeg already present: $dest"; exit 0
}
New-Item -ItemType Directory -Force $deps | Out-Null
$zip = Join-Path $deps 'ffmpeg.zip'
Write-Host "Downloading $($lock.FFMPEG_URL)"
Invoke-WebRequest -Uri $lock.FFMPEG_URL -OutFile $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($hash -ne $lock.FFMPEG_SHA256) { throw "sha256 mismatch: $hash" }
if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
$tmp = Join-Path $deps 'ffmpeg-extract'
if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
Expand-Archive $zip $tmp
Move-Item (Get-ChildItem $tmp | Select-Object -First 1).FullName $dest
Remove-Item -Recurse -Force $tmp, $zip
Set-Content $stamp $lock.FFMPEG_SHA256
Write-Host "FFmpeg ready: $dest"
