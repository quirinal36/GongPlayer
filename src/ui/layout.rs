//! 컨트롤 바 배치와 클릭 위치 판정, 영상 출력 사각형 계산.
//!
//! 모든 좌표는 창의 클라이언트 영역 기준 물리 화소입니다. 크기는 DIP로 정하고
//! `scale`(DPI / 96)을 곱합니다. Windows API를 부르지 않습니다.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    pub fn center_y(&self) -> f32 {
        (self.top + self.bottom) * 0.5
    }
    pub fn center_x(&self) -> f32 {
        (self.left + self.right) * 0.5
    }
}

/// 클릭할 수 있는 요소.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    SeekBar,
    Back,
    Play,
    Forward,
    Repeat,
    Speed,
    Mute,
    Volume,
    Fullscreen,
    /// 컨트롤 바의 빈 곳. 클릭해도 아무 일도 하지 않습니다.
    Panel,
    /// 영상 위. 클릭하면 재생/일시정지입니다.
    Video,
}

// DIP 단위 크기.
const BAR_H: f32 = 44.0;
const SEEK_H: f32 = 20.0;
const PANEL_PAD: f32 = 28.0;
const MARGIN: f32 = 12.0;
const BUTTON: f32 = 36.0;
const GAP: f32 = 4.0;
const TIME_W: f32 = 140.0;
const SPEED_W: f32 = 52.0;
const VOLUME_W: f32 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub scale: f32,
    /// 반투명 배경을 깔 영역 (탐색 바와 버튼 줄을 함께 덮음).
    pub panel: Rect,
    /// 탐색 바의 클릭 영역.
    pub seek_hit: Rect,
    /// 그려지는 탐색 바 막대.
    pub seek_track: Rect,
    pub back: Rect,
    pub play: Rect,
    pub forward: Rect,
    pub repeat: Rect,
    pub time: Rect,
    pub speed: Rect,
    pub mute: Rect,
    /// 음량 슬라이더의 클릭 영역.
    pub volume_hit: Rect,
    /// 그려지는 음량 막대.
    pub volume_track: Rect,
    pub fullscreen: Rect,
    /// 왼쪽 위 OSD 글자의 기준점.
    pub osd: Rect,
}

impl Layout {
    pub fn new(width: f32, height: f32, scale: f32) -> Self {
        let s = scale.max(0.5);
        let bar_bottom = height;
        let bar_top = bar_bottom - BAR_H * s;
        let seek_bottom = bar_top;
        let seek_top = seek_bottom - SEEK_H * s;
        let panel = Rect::new(0.0, seek_top - PANEL_PAD * s, width, height);
        let m = MARGIN * s;
        let seek_hit = Rect::new(m, seek_top, (width - m).max(m), seek_bottom);
        let track_h = 4.0 * s;
        let seek_track = Rect::new(
            seek_hit.left,
            seek_hit.center_y() - track_h * 0.5,
            seek_hit.right,
            seek_hit.center_y() + track_h * 0.5,
        );

        let cy = (bar_top + bar_bottom) * 0.5;
        let bh = BUTTON * s;
        let row = |l: f32, w: f32| Rect::new(l, cy - bh * 0.5, l + w, cy + bh * 0.5);

        let mut x = m - 6.0 * s;
        let back = row(x, bh);
        x += bh + GAP * s;
        let play = row(x, bh);
        x += bh + GAP * s;
        let forward = row(x, bh);
        x += bh + GAP * s;
        let repeat = row(x, bh);
        x += bh + GAP * s * 3.0;
        let time = row(x, TIME_W * s);

        let mut r = width - m + 6.0 * s;
        r -= bh;
        let fullscreen = row(r, bh);
        r -= GAP * s * 2.0 + VOLUME_W * s;
        let volume_hit = row(r, VOLUME_W * s);
        r -= bh;
        let mute = row(r, bh);
        r -= GAP * s * 2.0 + SPEED_W * s;
        let speed = row(r, SPEED_W * s);
        let vol_h = 4.0 * s;
        let volume_track = Rect::new(
            volume_hit.left + 4.0 * s,
            cy - vol_h * 0.5,
            volume_hit.right - 4.0 * s,
            cy + vol_h * 0.5,
        );

        let osd = Rect::new(20.0 * s, 16.0 * s, width - 20.0 * s, 16.0 * s + 40.0 * s);

        Self {
            scale: s,
            panel,
            seek_hit,
            seek_track,
            back,
            play,
            forward,
            repeat,
            time,
            speed,
            mute,
            volume_hit,
            volume_track,
            fullscreen,
            osd,
        }
    }

    /// 창이 좁으면 시간 표시를 숨깁니다. 오른쪽 버튼과 겹치기 때문입니다.
    pub fn show_time(&self) -> bool {
        self.time.right <= self.speed.left
    }

    /// 창이 너무 좁으면 오른쪽 버튼 무리를 숨깁니다.
    pub fn show_right(&self) -> bool {
        self.speed.left >= self.repeat.right
    }

    /// `(x, y)`에 있는 요소. 컨트롤 바가 보이지 않으면 모두 영상입니다.
    pub fn hit(&self, x: f32, y: f32, controls_visible: bool) -> Element {
        if !controls_visible || !self.panel.contains(x, y) {
            return Element::Video;
        }
        if self.seek_hit.contains(x, y) {
            return Element::SeekBar;
        }
        let mut items = vec![
            (self.back, Element::Back),
            (self.play, Element::Play),
            (self.forward, Element::Forward),
            (self.repeat, Element::Repeat),
        ];
        if self.show_right() {
            items.extend([
                (self.speed, Element::Speed),
                (self.mute, Element::Mute),
                (self.volume_hit, Element::Volume),
                (self.fullscreen, Element::Fullscreen),
            ]);
        }
        items
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, e)| e)
            .unwrap_or(Element::Panel)
    }

    /// 탐색 바 위의 x 좌표를 0.0~1.0 비율로.
    pub fn seek_fraction(&self, x: f32) -> f32 {
        fraction(&self.seek_track, x)
    }

    /// 음량 슬라이더 위의 x 좌표를 0~100으로. 5 단위로 맞춥니다.
    pub fn volume_at(&self, x: f32) -> u32 {
        let f = fraction(&self.volume_track, x);
        ((f * 20.0).round() as u32 * 5).min(100)
    }
}

fn fraction(track: &Rect, x: f32) -> f32 {
    if track.width() <= 0.0 {
        return 0.0;
    }
    ((x - track.left) / track.width()).clamp(0.0, 1.0)
}

/// 화면 비율을 지키며 `(w, h)` 영상을 `(dst_w, dst_h)` 창 가운데에 맞춘 사각형.
/// `sar`는 화소 비율(가로/세로), `rotated`가 참이면 90도나 270도 회전입니다.
pub fn fit_video(w: u32, h: u32, sar: f64, rotated: bool, dst_w: u32, dst_h: u32) -> Rect {
    if w == 0 || h == 0 || dst_w == 0 || dst_h == 0 {
        return Rect::default();
    }
    let sar = if sar.is_finite() && sar > 0.0 {
        sar
    } else {
        1.0
    };
    let (mut vw, mut vh) = (w as f64 * sar, h as f64);
    if rotated {
        std::mem::swap(&mut vw, &mut vh);
    }
    let scale = (dst_w as f64 / vw).min(dst_h as f64 / vh);
    let (ow, oh) = ((vw * scale).round(), (vh * scale).round());
    let left = ((dst_w as f64 - ow) / 2.0).floor();
    let top = ((dst_h as f64 - oh) / 2.0).floor();
    Rect::new(
        left as f32,
        top as f32,
        (left + ow) as f32,
        (top + oh) as f32,
    )
}

/// 처음 열 때의 창 클라이언트 크기. 원본 크기로 열되, 작업 영역의 80%를 넘으면
/// 비율을 지키며 줄입니다.
pub fn initial_window_size(
    w: u32,
    h: u32,
    sar: f64,
    rotated: bool,
    work_w: u32,
    work_h: u32,
) -> (u32, u32) {
    let sar = if sar.is_finite() && sar > 0.0 {
        sar
    } else {
        1.0
    };
    let (mut vw, mut vh) = ((w as f64 * sar).round(), h as f64);
    if rotated {
        std::mem::swap(&mut vw, &mut vh);
    }
    let (max_w, max_h) = (work_w as f64 * 0.8, work_h as f64 * 0.8);
    let scale = (max_w / vw).min(max_h / vh).min(1.0);
    (
        ((vw * scale).round() as u32).max(320),
        ((vh * scale).round() as u32).max(180),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_sit_at_bottom() {
        let l = Layout::new(1280.0, 720.0, 1.0);
        assert_eq!(l.panel.bottom, 720.0);
        assert!(l.seek_hit.bottom <= l.play.top);
        assert!(l.play.left > l.back.left);
        assert!(l.fullscreen.right <= 1280.0);
    }

    #[test]
    fn hit_finds_each_element() {
        let l = Layout::new(1280.0, 720.0, 1.0);
        for (r, e) in [
            (l.back, Element::Back),
            (l.play, Element::Play),
            (l.forward, Element::Forward),
            (l.repeat, Element::Repeat),
            (l.speed, Element::Speed),
            (l.mute, Element::Mute),
            (l.volume_hit, Element::Volume),
            (l.fullscreen, Element::Fullscreen),
            (l.seek_hit, Element::SeekBar),
        ] {
            assert_eq!(l.hit(r.center_x(), r.center_y(), true), e, "{e:?}");
        }
        assert_eq!(l.hit(640.0, 100.0, true), Element::Video);
        assert_eq!(
            l.hit(l.play.center_x(), l.play.center_y(), false),
            Element::Video
        );
    }

    #[test]
    fn empty_bar_area_is_panel() {
        let l = Layout::new(1280.0, 720.0, 1.0);
        let x = (l.time.right + l.speed.left) / 2.0;
        assert_eq!(l.hit(x, l.play.center_y(), true), Element::Panel);
    }

    #[test]
    fn scale_grows_controls() {
        let a = Layout::new(1920.0, 1080.0, 1.0);
        let b = Layout::new(1920.0, 1080.0, 2.0);
        assert!((b.play.width() - a.play.width() * 2.0).abs() < 0.01);
        assert!(b.panel.top < a.panel.top);
    }

    #[test]
    fn seek_fraction_is_clamped() {
        let l = Layout::new(1000.0, 600.0, 1.0);
        assert_eq!(l.seek_fraction(-50.0), 0.0);
        assert_eq!(l.seek_fraction(5000.0), 1.0);
        let mid = l.seek_track.center_x();
        assert!((l.seek_fraction(mid) - 0.5).abs() < 0.001);
    }

    #[test]
    fn volume_snaps_to_five() {
        let l = Layout::new(1000.0, 600.0, 1.0);
        assert_eq!(l.volume_at(l.volume_track.left - 10.0), 0);
        assert_eq!(l.volume_at(l.volume_track.right + 10.0), 100);
        assert_eq!(l.volume_at(l.volume_track.center_x()), 50);
    }

    #[test]
    fn narrow_window_hides_right_group() {
        let l = Layout::new(200.0, 300.0, 1.0);
        assert!(!l.show_right());
        assert!(!l.show_time());
        assert_eq!(
            l.hit(l.repeat.center_x(), l.repeat.center_y(), true),
            Element::Repeat
        );
        assert_ne!(
            l.hit(l.fullscreen.center_x(), l.fullscreen.center_y(), true),
            Element::Fullscreen
        );
    }

    #[test]
    fn fit_letterboxes_wide_video() {
        let r = fit_video(1920, 1080, 1.0, false, 1000, 1000);
        assert_eq!(r.width(), 1000.0);
        assert!((r.height() - 563.0).abs() <= 1.0);
        assert!((r.top - (1000.0 - r.height()) / 2.0).abs() <= 1.0);
    }

    #[test]
    fn fit_handles_rotation_and_sar() {
        let r = fit_video(1920, 1080, 1.0, true, 1000, 1000);
        assert_eq!(r.height(), 1000.0);
        assert!((r.width() - 563.0).abs() <= 1.0);
        // 화소 비율 4:3인 720x576은 960x576처럼 보여야 합니다.
        let r = fit_video(720, 576, 4.0 / 3.0, false, 960, 576);
        assert_eq!((r.width(), r.height()), (960.0, 576.0));
    }

    #[test]
    fn fit_degenerate_is_empty() {
        assert_eq!(fit_video(0, 10, 1.0, false, 10, 10), Rect::default());
    }

    #[test]
    fn initial_size_keeps_small_video_and_shrinks_big() {
        assert_eq!(
            initial_window_size(640, 360, 1.0, false, 1920, 1040),
            (640, 360)
        );
        let (w, h) = initial_window_size(3840, 2160, 1.0, false, 1920, 1040);
        assert!(w as f64 <= 1920.0 * 0.8 && h as f64 <= 1040.0 * 0.8 + 1.0);
        assert!(((w as f64 / h as f64) - 16.0 / 9.0).abs() < 0.01);
        let (w, h) = initial_window_size(1920, 1080, 1.0, true, 1920, 1040);
        assert!(h > w);
    }
}
