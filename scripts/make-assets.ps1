# 패키지 로고와 실행 파일 아이콘을 만듭니다. 원본은 packaging/source/icon-ball.png
# (불꽃 다섯 개가 그려진 공, 1024px, 투명 배경)이고, 크기마다 고품질로 줄여 투명 캔버스 가운데에 놓습니다.
# 사용: pwsh scripts/make-assets.ps1   결과: packaging/Assets/*.png, packaging/gongplayer.ico
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $root 'packaging/Assets'
New-Item -ItemType Directory -Force $assets | Out-Null
$source = [System.Drawing.Image]::FromFile((Join-Path $root 'packaging/source/icon-ball.png'))

function Draw-Logo([int]$w, [int]$h, [double]$fill) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.CompositingQuality = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $d = [Math]::Min($w, $h) * $fill
    $x = ($w - $d) / 2; $y = ($h - $d) / 2
    $attr = New-Object System.Drawing.Imaging.ImageAttributes
    $attr.SetWrapMode('TileFlipXY')   # 가장자리 픽셀이 바깥 투명과 섞여 흐려지지 않게 합니다
    $dest = New-Object System.Drawing.Rectangle ([int][Math]::Round($x)), ([int][Math]::Round($y)), ([int][Math]::Round($d)), ([int][Math]::Round($d))
    $g.DrawImage($source, $dest, 0, 0, $source.Width, $source.Height, [System.Drawing.GraphicsUnit]::Pixel, $attr)
    $g.Dispose()
    return $bmp
}

$targets = @{
    'Square44x44Logo.png' = @(44, 44, 1.0)
    'Square44x44Logo.targetsize-256_altform-unplated.png' = @(256, 256, 1.0)
    'Square44x44Logo.targetsize-48_altform-unplated.png' = @(48, 48, 1.0)
    'Square44x44Logo.targetsize-32_altform-unplated.png' = @(32, 32, 1.0)
    'Square44x44Logo.targetsize-16_altform-unplated.png' = @(16, 16, 1.0)
    'Square150x150Logo.png' = @(150, 150, 0.6)
    'Wide310x150Logo.png' = @(310, 150, 0.6)
    'StoreLogo.png' = @(50, 50, 1.0)
}
foreach ($k in $targets.Keys) {
    $v = $targets[$k]
    $b = Draw-Logo $v[0] $v[1] $v[2]
    $b.Save((Join-Path $assets $k), [System.Drawing.Imaging.ImageFormat]::Png)
    $b.Dispose()
}

# 실행 파일 아이콘 (PNG를 담은 ICO, 여러 크기)
$sizes = @(16, 24, 32, 48, 64, 256)
$pngs = foreach ($s in $sizes) {
    $b = Draw-Logo $s $s 1.0
    $ms = New-Object System.IO.MemoryStream
    $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $b.Dispose()
    , $ms.ToArray()
}
$ico = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $ico
$bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $len = $pngs[$i].Length
    $bw.Write([byte]($s % 256)); $bw.Write([byte]($s % 256)); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([UInt16]1); $bw.Write([UInt16]32); $bw.Write([UInt32]$len); $bw.Write([UInt32]$offset)
    $offset += $len
}
foreach ($p in $pngs) { $bw.Write($p) }
$bw.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $root 'packaging/gongplayer.ico'), $ico.ToArray())
Write-Host "assets ready"
