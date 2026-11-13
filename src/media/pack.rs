//! 소프트웨어로 디코딩한 프레임을 NV12(8bit)나 P010(10bit 이상)의 연속 메모리로 옮깁니다.
//! `swscale` 없이 평면 YUV, NV12/P010, 회색조를 다룹니다 (docs/DECISIONS.md의 D19).
//! FFmpeg의 화소 형식 설명을 숫자로 받으므로 Windows API 없이 테스트할 수 있습니다.

/// 색 성분 하나가 어디에 어떻게 있는지 (`AVComponentDescriptor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    pub plane: usize,
    /// 다음 화소까지의 바이트 수.
    pub step: usize,
    /// 화소 안에서의 바이트 위치.
    pub offset: usize,
    /// 값이 2바이트 안에서 왼쪽으로 밀려 있는 비트 수 (P010은 6).
    pub shift: u32,
}

/// 화소 형식 (`AVPixFmtDescriptor`에서 필요한 것만).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// Y, U, V. 회색조면 U, V는 쓰지 않습니다.
    pub comps: [Component; 3],
    pub components: usize,
    /// 성분의 비트 수.
    pub depth: u32,
    pub log2_chroma_w: u32,
    pub log2_chroma_h: u32,
}

/// 원본 평면들. `stride`는 한 줄의 바이트 수입니다.
pub struct Planes<'a> {
    pub data: [&'a [u8]; 4],
    pub stride: [usize; 4],
}

/// 원본 평면 `plane`이 가진 줄 수. 밝기 평면은 높이만큼, 나머지는 색차 높이만큼입니다.
pub fn plane_rows(fmt: &Format, plane: usize, height: usize) -> usize {
    if plane == fmt.comps[0].plane {
        height
    } else {
        (height + (1 << fmt.log2_chroma_h) - 1) >> fmt.log2_chroma_h
    }
}

/// 출력 크기 (짝수로 올린 너비와 높이).
pub fn padded(w: usize, h: usize) -> (usize, usize) {
    ((w + 1) & !1, (h + 1) & !1)
}

/// `out`에 NV12 또는 P010을 씁니다. 10bit 이상이면 참을 돌려줍니다.
/// 출력은 짝수 크기이고, 늘어난 가장자리는 마지막 화소를 되풀이합니다.
pub fn pack(fmt: &Format, src: &Planes, w: usize, h: usize, out: &mut Vec<u8>) -> bool {
    let ten_bit = fmt.depth > 8;
    let bpp = if ten_bit { 2 } else { 1 };
    let (aw, ah) = padded(w, h);
    let y_size = aw * ah * bpp;
    out.resize(y_size + aw * (ah / 2) * bpp, 0);
    let (yplane, uvplane) = out.split_at_mut(y_size);

    // 2바이트 표본은 P010처럼 위쪽 비트에 맞춥니다.
    let up = if ten_bit { 16 - fmt.depth.min(16) } else { 0 };
    let sample = |c: &Component, x: usize, y: usize| -> u16 {
        let i = y * src.stride[c.plane] + x * c.step + c.offset;
        let d = src.data[c.plane];
        if bpp == 2 {
            (u16::from_le_bytes([d[i], d[i + 1]]) >> c.shift) << up
        } else {
            d[i] as u16
        }
    };
    let put = |buf: &mut [u8], idx: usize, v: u16| {
        if bpp == 2 {
            buf[idx * 2..idx * 2 + 2].copy_from_slice(&v.to_le_bytes());
        } else {
            buf[idx] = v as u8;
        }
    };

    // 밝기. 흔한 경우(8bit, 한 칸에 한 바이트)는 줄째 복사합니다.
    let c0 = &fmt.comps[0];
    for y in 0..ah {
        let sy = y.min(h - 1);
        let dst = &mut yplane[y * aw * bpp..(y + 1) * aw * bpp];
        if bpp == 1 && c0.step == 1 && c0.offset == 0 {
            let start = sy * src.stride[c0.plane];
            let row = &src.data[c0.plane][start..start + w];
            dst[..w].copy_from_slice(row);
            if aw > w {
                dst[w] = row[w - 1];
            }
        } else {
            for x in 0..aw {
                put(dst, x, sample(c0, x.min(w - 1), sy));
            }
        }
    }

    // 색차. 출력의 한 칸은 원본 밝기 좌표 (2x, 2y)에 해당하는 색차 표본입니다.
    let neutral: u16 = if ten_bit { 0x8000 } else { 0x80 };
    let (cw, ch) = (aw / 2, ah / 2);
    let (lw, lh) = (fmt.log2_chroma_w as usize, fmt.log2_chroma_h as usize);
    let src_cw = (w + (1 << lw) - 1) >> lw;
    let src_ch = (h + (1 << lh) - 1) >> lh;
    for y in 0..ch {
        let sy = ((y * 2) >> lh).min(src_ch - 1);
        for x in 0..cw {
            let sx = ((x * 2) >> lw).min(src_cw - 1);
            let (u, v) = if fmt.components < 3 {
                (neutral, neutral)
            } else {
                (sample(&fmt.comps[1], sx, sy), sample(&fmt.comps[2], sx, sy))
            };
            let idx = (y * cw + x) * 2;
            put(uvplane, idx, u);
            put(uvplane, idx + 1, v);
        }
    }
    ten_bit
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn comp(plane: usize, step: usize, offset: usize, shift: u32) -> Component {
        Component {
            plane,
            step,
            offset,
            shift,
        }
    }

    fn planar(depth: u32, lw: u32, lh: u32) -> Format {
        let step = if depth > 8 { 2 } else { 1 };
        Format {
            comps: [
                comp(0, step, 0, 0),
                comp(1, step, 0, 0),
                comp(2, step, 0, 0),
            ],
            components: 3,
            depth,
            log2_chroma_w: lw,
            log2_chroma_h: lh,
        }
    }

    fn le16(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn read16(b: &[u8]) -> Vec<u16> {
        b.chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn yuv420p_to_nv12() {
        // 4x2 밝기, 2x1 색차
        let y = [1, 2, 3, 4, 5, 6, 7, 8];
        let u = [10, 20];
        let v = [30, 40];
        let src = Planes {
            data: [&y, &u, &v, &[]],
            stride: [4, 2, 2, 0],
        };
        let mut out = Vec::new();
        assert!(!pack(&planar(8, 1, 1), &src, 4, 2, &mut out));
        assert_eq!(&out[..8], &y);
        assert_eq!(&out[8..], &[10, 30, 20, 40]);
    }

    #[test]
    fn yuv420p10_is_shifted_to_high_bits() {
        let y = le16(&[1023, 0, 512, 1]);
        let u = le16(&[100]);
        let v = le16(&[200]);
        let src = Planes {
            data: [&y, &u, &v, &[]],
            stride: [4, 2, 2, 0],
        };
        let mut out = Vec::new();
        assert!(pack(&planar(10, 1, 1), &src, 2, 2, &mut out));
        let o = read16(&out);
        assert_eq!(&o[..4], &[1023 << 6, 0, 512 << 6, 1 << 6]);
        assert_eq!(&o[4..], &[100 << 6, 200 << 6]);
    }

    #[test]
    fn p010_input_is_not_shifted_twice() {
        // P010: 밝기 평면 + UV 교차 평면, 값은 이미 위쪽 비트에 있음 (shift 6)
        let fmt = Format {
            comps: [comp(0, 2, 0, 6), comp(1, 4, 0, 6), comp(1, 4, 2, 6)],
            components: 3,
            depth: 10,
            log2_chroma_w: 1,
            log2_chroma_h: 1,
        };
        let y = le16(&[1000 << 6, 1 << 6, 2 << 6, 3 << 6]);
        let uv = le16(&[5 << 6, 7 << 6]);
        let src = Planes {
            data: [&y, &uv, &[], &[]],
            stride: [4, 4, 0, 0],
        };
        let mut out = Vec::new();
        pack(&fmt, &src, 2, 2, &mut out);
        let o = read16(&out);
        assert_eq!(o, vec![1000 << 6, 1 << 6, 2 << 6, 3 << 6, 5 << 6, 7 << 6]);
    }

    #[test]
    fn yuv422_takes_every_other_chroma_row() {
        // 2x4 영상, 4:2:2 → 색차는 1x4. 출력 색차는 1x2이며 0, 2번째 줄을 씁니다.
        let y = [0u8; 8];
        let u = [1, 2, 3, 4];
        let v = [5, 6, 7, 8];
        let src = Planes {
            data: [&y, &u, &v, &[]],
            stride: [2, 1, 1, 0],
        };
        let mut out = Vec::new();
        pack(&planar(8, 1, 0), &src, 2, 4, &mut out);
        assert_eq!(&out[8..], &[1, 5, 3, 7]);
    }

    #[test]
    fn yuv444_subsamples_both_ways() {
        let y = [0u8; 4];
        let u = [1, 2, 3, 4];
        let v = [5, 6, 7, 8];
        let src = Planes {
            data: [&y, &u, &v, &[]],
            stride: [2, 2, 2, 0],
        };
        let mut out = Vec::new();
        pack(&planar(8, 0, 0), &src, 2, 2, &mut out);
        assert_eq!(&out[4..], &[1, 5]);
    }

    #[test]
    fn gray_gets_neutral_chroma() {
        let fmt = Format {
            comps: [comp(0, 1, 0, 0), comp(0, 0, 0, 0), comp(0, 0, 0, 0)],
            components: 1,
            depth: 8,
            log2_chroma_w: 0,
            log2_chroma_h: 0,
        };
        let y = [9u8; 4];
        let src = Planes {
            data: [&y, &[], &[], &[]],
            stride: [2, 0, 0, 0],
        };
        let mut out = Vec::new();
        pack(&fmt, &src, 2, 2, &mut out);
        assert_eq!(out, vec![9, 9, 9, 9, 0x80, 0x80]);
    }

    #[test]
    fn odd_size_repeats_edge_and_respects_stride() {
        // 3x1 영상, 줄 간격 8 (덧붙은 바이트는 무시)
        let y = [1, 2, 3, 99, 99, 99, 99, 99];
        let u = [10, 20, 99, 99];
        let v = [30, 40, 99, 99];
        let src = Planes {
            data: [&y, &u, &v, &[]],
            stride: [8, 4, 4, 0],
        };
        let mut out = Vec::new();
        pack(&planar(8, 1, 1), &src, 3, 1, &mut out);
        // 4x2로 늘어남
        assert_eq!(&out[..8], &[1, 2, 3, 3, 1, 2, 3, 3]);
        assert_eq!(&out[8..], &[10, 30, 20, 40]);
    }

    #[test]
    fn plane_rows_for_chroma() {
        let f = planar(8, 1, 1);
        assert_eq!(plane_rows(&f, 0, 5), 5);
        assert_eq!(plane_rows(&f, 1, 5), 3);
    }
}
