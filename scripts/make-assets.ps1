# 패키지 로고와 실행 파일 아이콘을 만듭니다. 이미지 파일을 손으로 그리지 않고 도형으로 생성합니다.
# 사용: pwsh scripts/make-assets.ps1   결과: packaging/Assets/*.png, packaging/gongplayer.ico
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $root 'packaging/Assets'
New-Item -ItemType Directory -Force $assets | Out-Null

function Draw-Logo([int]$w, [int]$h, [double]$fill) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $d = [Math]::Min($w, $h) * $fill
    $x = ($w - $d) / 2; $y = ($h - $d) / 2
    # 공(원) 안에 재생 삼각형
    $bg = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 24, 28, 36))
    $g.FillEllipse($bg, [single]$x, [single]$y, [single]$d, [single]$d)
    $ring = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 255, 196, 0)), ([single]($d * 0.06))
    $g.DrawEllipse($ring, [single]($x + $d * 0.03), [single]($y + $d * 0.03), [single]($d * 0.94), [single]($d * 0.94))
    $cx = $w / 2 + $d * 0.05; $cy = $h / 2; $r = $d * 0.26
    $pts = @(
        (New-Object System.Drawing.PointF ([single]($cx - $r * 0.8)), ([single]($cy - $r))),
        (New-Object System.Drawing.PointF ([single]($cx + $r)), ([single]$cy)),
        (New-Object System.Drawing.PointF ([single]($cx - $r * 0.8)), ([single]($cy + $r)))
    )
    $g.FillPolygon([System.Drawing.Brushes]::White, $pts)
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
