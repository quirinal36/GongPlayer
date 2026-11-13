# Microsoft Store 목록용 로고 그림을 만듭니다. 원본은 packaging/source/icon-ball.png입니다.
# 9:16 포스터 아트(1440x2160)는 공 아래에 제품 이름을 쓰고, 1:1 박스 아트(2160x2160)는 공만 놓습니다.
# 배경은 스크린샷과 같은 톤의 어두운 남색에서 보라색으로 가는 그라데이션입니다.
# 사용: pwsh scripts/make-store-art.ps1   결과: docs/store/art/poster-9x16.png, docs/store/art/box-1x1.png
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'docs/store/art'
New-Item -ItemType Directory -Force $out | Out-Null
$ball = [System.Drawing.Image]::FromFile((Join-Path $root 'packaging/source/icon-ball.png'))

function New-Canvas([int]$w, [int]$h) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.CompositingQuality = 'HighQuality'
    $g.TextRenderingHint = 'AntiAliasGridFit'

    # 위는 짙은 남색, 아래는 어두운 보라
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $top = [System.Drawing.Color]::FromArgb(255, 14, 24, 40)
    $bottom = [System.Drawing.Color]::FromArgb(255, 38, 18, 52)
    $lin = New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, $top, $bottom, 90.0
    $g.FillRectangle($lin, $rect)
    $lin.Dispose()
    return @($bmp, $g)
}

# 공 뒤에 은은한 주황빛을 깔아 어두운 배경에서 공이 떠 보이게 합니다.
function Draw-Glow($g, [double]$cx, [double]$cy, [double]$r) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddEllipse([single]($cx - $r), [single]($cy - $r), [single](2 * $r), [single](2 * $r))
    $pgb = New-Object System.Drawing.Drawing2D.PathGradientBrush $path
    $pgb.CenterColor = [System.Drawing.Color]::FromArgb(110, 255, 140, 60)
    $pgb.SurroundColors = @([System.Drawing.Color]::FromArgb(0, 255, 140, 60))
    $g.FillPath($pgb, $path)
    $pgb.Dispose(); $path.Dispose()
}

function Draw-Ball($g, [double]$cx, [double]$cy, [double]$d) {
    $attr = New-Object System.Drawing.Imaging.ImageAttributes
    $attr.SetWrapMode('TileFlipXY')
    $dest = New-Object System.Drawing.Rectangle ([int]($cx - $d / 2)), ([int]($cy - $d / 2)), ([int]$d), ([int]$d)
    $g.DrawImage($ball, $dest, 0, 0, $ball.Width, $ball.Height, [System.Drawing.GraphicsUnit]::Pixel, $attr)
}

# 9:16 포스터 아트
$W = 1440; $H = 2160
$c = New-Canvas $W $H; $bmp = $c[0]; $g = $c[1]
$d = $W * 0.72; $cy = $H * 0.40
Draw-Glow $g ($W / 2) $cy ($d * 0.85)
Draw-Ball $g ($W / 2) $cy $d
$font = New-Object System.Drawing.Font 'Segoe UI Semibold', 150, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
$fmt = New-Object System.Drawing.StringFormat
$fmt.Alignment = 'Center'; $fmt.LineAlignment = 'Center'
$textRect = New-Object System.Drawing.RectangleF 0, ([single]($H * 0.72)), ([single]$W), 260
$g.DrawString('GongPlayer', $font, [System.Drawing.Brushes]::White, $textRect, $fmt)
$g.Dispose()
$bmp.Save((Join-Path $out 'poster-9x16.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

# 1:1 박스 아트 (제품 이름 없음)
$S = 2160
$c = New-Canvas $S $S; $bmp = $c[0]; $g = $c[1]
$d = $S * 0.62
Draw-Glow $g ($S / 2) ($S / 2) ($d * 0.85)
Draw-Ball $g ($S / 2) ($S / 2) $d
$g.Dispose()
$bmp.Save((Join-Path $out 'box-1x1.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

$ball.Dispose()
Write-Host "store art ready: $out"
