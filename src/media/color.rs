//! 프레임의 색 정보를 VideoProcessor가 알아듣는 종류로 나눕니다.
//! FFmpeg 열거값을 숫자로 받으므로 Windows API 없이 테스트할 수 있습니다.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primaries {
    Bt601,
    Bt709,
    Bt2020,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Sdr,
    /// SMPTE ST 2084 (HDR10, Dolby Vision 프로파일 8.1 등)
    Pq,
    /// ARIB STD-B67 (아이폰 HDR 영상)
    Hlg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorKind {
    pub primaries: Primaries,
    pub transfer: Transfer,
    pub full_range: bool,
}

impl ColorKind {
    pub fn is_hdr(&self) -> bool {
        self.transfer != Transfer::Sdr
    }
}

// FFmpeg 열거값 (libavutil/pixfmt.h).
mod av {
    pub const SPC_BT709: i32 = 1;
    pub const SPC_BT470BG: i32 = 5;
    pub const SPC_SMPTE170M: i32 = 6;
    pub const SPC_BT2020_NCL: i32 = 9;
    pub const SPC_BT2020_CL: i32 = 10;
    pub const PRI_BT709: i32 = 1;
    pub const PRI_BT470BG: i32 = 5;
    pub const PRI_SMPTE170M: i32 = 6;
    pub const PRI_BT2020: i32 = 9;
    pub const TRC_SMPTE2084: i32 = 16;
    pub const TRC_ARIB_STD_B67: i32 = 18;
    pub const RANGE_JPEG: i32 = 2;
}

/// 색공간·원색·전달 특성·범위 값과 영상 높이로 색 종류를 정합니다.
/// 지정되지 않은 값은 해상도로 짐작합니다 (SD는 BT.601, 그 이상은 BT.709).
pub fn classify(space: i32, primaries: i32, transfer: i32, range: i32, height: u32) -> ColorKind {
    let transfer = match transfer {
        av::TRC_SMPTE2084 => Transfer::Pq,
        av::TRC_ARIB_STD_B67 => Transfer::Hlg,
        _ => Transfer::Sdr,
    };
    let primaries = match (space, primaries) {
        (av::SPC_BT2020_NCL | av::SPC_BT2020_CL, _) | (_, av::PRI_BT2020) => Primaries::Bt2020,
        (av::SPC_BT709, _) | (_, av::PRI_BT709) => Primaries::Bt709,
        (av::SPC_BT470BG | av::SPC_SMPTE170M, _) | (_, av::PRI_BT470BG | av::PRI_SMPTE170M) => {
            Primaries::Bt601
        }
        _ if transfer != Transfer::Sdr => Primaries::Bt2020,
        _ if height < 720 => Primaries::Bt601,
        _ => Primaries::Bt709,
    };
    ColorKind {
        primaries,
        transfer,
        full_range: range == av::RANGE_JPEG,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspecified_guesses_by_height() {
        assert_eq!(classify(2, 2, 2, 0, 480).primaries, Primaries::Bt601);
        assert_eq!(classify(2, 2, 2, 0, 1080).primaries, Primaries::Bt709);
    }

    #[test]
    fn hdr_variants() {
        let pq = classify(9, 9, 16, 1, 2160);
        assert_eq!(
            (pq.primaries, pq.transfer),
            (Primaries::Bt2020, Transfer::Pq)
        );
        assert!(pq.is_hdr());
        let hlg = classify(9, 9, 18, 1, 2160);
        assert_eq!(hlg.transfer, Transfer::Hlg);
        // 원색이 비어 있어도 HDR이면 BT.2020으로 봅니다.
        assert_eq!(classify(2, 2, 16, 1, 2160).primaries, Primaries::Bt2020);
    }

    #[test]
    fn explicit_values_win() {
        assert_eq!(classify(1, 1, 1, 1, 480).primaries, Primaries::Bt709);
        assert_eq!(classify(6, 6, 6, 1, 1080).primaries, Primaries::Bt601);
        assert!(classify(1, 1, 1, 2, 1080).full_range);
    }
}
