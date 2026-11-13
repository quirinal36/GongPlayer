# THIRD_PARTY_NOTICES.md를 만듭니다. 실행 파일에 링크되는 crate와 그 라이선스 원문,
# 함께 싣는 FFmpeg의 정보를 모읍니다. 의존성이나 FFmpeg 구성이 바뀌면 다시 실행합니다.
#   pwsh scripts/notices.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$target = 'x86_64-pc-windows-msvc'

# 링크되는 crate (빌드 시에만 쓰는 proc-macro와 그 의존성 제외)
$lines = cargo tree --target $target -e normal,no-proc-macro --prefix none --format '{p}' | Sort-Object -Unique
$ids = foreach ($l in $lines) {
    if ($l -match '^(\S+) v(\S+)') { if ($Matches[1] -ne 'gongplayer') { "$($Matches[1])@$($Matches[2])" } }
}
$meta = cargo metadata --format-version 1 --filter-platform $target | ConvertFrom-Json
$pkgs = $meta.packages | Where-Object { $ids -contains "$($_.name)@$($_.version)" } | Sort-Object name

$sb = New-Object Text.StringBuilder
[void]$sb.AppendLine('# 제3자 고지 (Third-Party Notices)')
[void]$sb.AppendLine()
[void]$sb.AppendLine('GongPlayer에 포함되거나 링크되는 제3자 소프트웨어와 그 라이선스입니다. `scripts/notices.ps1`로 만듭니다.')
[void]$sb.AppendLine()
[void]$sb.AppendLine('## FFmpeg')
[void]$sb.AppendLine()
$lock = @{}
Get-Content scripts/deps.lock | Where-Object { $_ -match '^[A-Z_0-9]+=' } | ForEach-Object { $k, $v = $_ -split '=', 2; $lock[$k] = $v.Trim() }
[void]$sb.AppendLine('GongPlayer는 FFmpeg(https://ffmpeg.org)를 사용합니다. FFmpeg는 GNU Lesser General Public License 2.1 이상으로 배포되며, GPL·nonfree 구성 요소 없이 빌드한 것을 씁니다. 라이선스 원문은 패키지의 `FFMPEG_LICENSE.txt`에 있습니다.')
[void]$sb.AppendLine()
[void]$sb.AppendLine("- 배포용 패키지: FFmpeg $($lock.FFMPEG_SRC_URL -replace '.*ffmpeg-([0-9.]+)\.tar\.xz','$1')을 최소 구성으로 직접 빌드해 실행 파일에 정적으로 링크합니다. 소스: $($lock.FFMPEG_SRC_URL) (sha256 $($lock.FFMPEG_SRC_SHA256)). 빌드 방법과 구성 옵션은 저장소의 ``scripts/build-ffmpeg-static.sh``에 있습니다. 외부 라이브러리는 넣지 않습니다.")
[void]$sb.AppendLine("- 개발용 패키지: $($lock.FFMPEG_URL) (BtbN 빌드, 공유 라이브러리). 이 빌드에 함께 든 외부 라이브러리의 라이선스는 그 패키지의 ``FFMPEG_LICENSE.txt``에 있습니다.")
[void]$sb.AppendLine('- 대응하는 소스 코드와 다시 빌드·링크하는 방법은 docs/BUILD.md의 "FFmpeg"와 "라이선스" 절을 따릅니다.')
[void]$sb.AppendLine()
[void]$sb.AppendLine('## Rust crate')
[void]$sb.AppendLine()
[void]$sb.AppendLine('| crate | 버전 | 라이선스 | 저장소 |')
[void]$sb.AppendLine('|---|---|---|---|')
foreach ($p in $pkgs) {
    [void]$sb.AppendLine("| $($p.name) | $($p.version) | $($p.license) | $($p.repository) |")
}
[void]$sb.AppendLine()
[void]$sb.AppendLine('## 라이선스 원문')
$seen = @{}
foreach ($p in $pkgs) {
    $dir = Split-Path -Parent $p.manifest_path
    $files = Get-ChildItem $dir -File | Where-Object { $_.Name -match '^(licen[sc]e|copying)' }
    foreach ($f in $files) {
        $text = (Get-Content $f.FullName -Raw).Trim()
        $key = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($text)))
        [void]$sb.AppendLine()
        [void]$sb.AppendLine("### $($p.name) $($p.version) — $($f.Name)")
        [void]$sb.AppendLine()
        if ($seen.ContainsKey($key)) {
            [void]$sb.AppendLine("위의 $($seen[$key])와 같은 텍스트입니다.")
        } else {
            $seen[$key] = "$($p.name) $($f.Name)"
            [void]$sb.AppendLine('```text')
            [void]$sb.AppendLine($text)
            [void]$sb.AppendLine('```')
        }
    }
    if (-not $files) {
        [void]$sb.AppendLine()
        [void]$sb.AppendLine("### $($p.name) $($p.version)")
        [void]$sb.AppendLine()
        [void]$sb.AppendLine("crate에 라이선스 파일이 없습니다. 라이선스: $($p.license), 저자: $($p.authors -join ', ')")
    }
}
[IO.File]::WriteAllText((Join-Path $root 'THIRD_PARTY_NOTICES.md'), $sb.ToString(), (New-Object Text.UTF8Encoding $false))
"THIRD_PARTY_NOTICES.md: crate $($pkgs.Count)개"
