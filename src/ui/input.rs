//! 키 입력을 명령으로 바꿉니다. 단축키 표는 docs/SPEC.md의 "조작"과 같아야 합니다.
//! 가상 키 코드를 숫자로 다루므로 Windows API를 부르지 않습니다.

use crate::player::Command;

/// 창이 처리할 일까지 포함한 동작.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Player(Command),
    ToggleFullscreen,
    ExitFullscreen,
    OpenDialog,
}

pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const RETURN: u16 = 0x0D;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const F: u16 = 0x46;
    pub const K: u16 = 0x4B;
    pub const M: u16 = 0x4D;
    pub const O: u16 = 0x4F;
    /// `[`
    pub const OEM_4: u16 = 0xDB;
    /// `]`
    pub const OEM_6: u16 = 0xDD;
}

pub const SEEK_SHORT: f64 = 5.0;
pub const SEEK_LONG: f64 = 30.0;
pub const VOLUME_STEP: i32 = 5;

/// 눌린 키에 해당하는 동작. 해당하는 것이 없으면 `None`.
pub fn map_key(key: u16, shift: bool, ctrl: bool) -> Option<Action> {
    use Action::Player as P;
    use Command::*;
    if ctrl {
        return (key == vk::O).then_some(Action::OpenDialog);
    }
    let seek = if shift { SEEK_LONG } else { SEEK_SHORT };
    Some(match key {
        vk::SPACE | vk::K => P(TogglePlay),
        vk::LEFT => P(SeekBy(-seek)),
        vk::RIGHT => P(SeekBy(seek)),
        vk::HOME => P(Home),
        vk::OEM_4 => P(SpeedDown),
        vk::OEM_6 => P(SpeedUp),
        vk::BACK => P(SpeedReset),
        vk::UP => P(VolumeBy(VOLUME_STEP)),
        vk::DOWN => P(VolumeBy(-VOLUME_STEP)),
        vk::M => P(ToggleMute),
        vk::F | vk::RETURN => Action::ToggleFullscreen,
        vk::ESCAPE => Action::ExitFullscreen,
        _ => return None,
    })
}

/// 휠 한 칸(120)마다 음량을 한 단계 바꿉니다. 남은 값은 다음 입력에 이어 씁니다.
pub fn wheel_steps(accum: &mut i32, delta: i32) -> i32 {
    *accum += delta;
    let steps = *accum / 120;
    *accum -= steps * 120;
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use Command::*;

    #[test]
    fn spec_shortcuts() {
        let p = |c| Some(Action::Player(c));
        assert_eq!(map_key(vk::SPACE, false, false), p(TogglePlay));
        assert_eq!(map_key(vk::K, false, false), p(TogglePlay));
        assert_eq!(map_key(vk::LEFT, false, false), p(SeekBy(-5.0)));
        assert_eq!(map_key(vk::RIGHT, false, false), p(SeekBy(5.0)));
        assert_eq!(map_key(vk::LEFT, true, false), p(SeekBy(-30.0)));
        assert_eq!(map_key(vk::RIGHT, true, false), p(SeekBy(30.0)));
        assert_eq!(map_key(vk::HOME, false, false), p(Home));
        assert_eq!(map_key(vk::OEM_4, false, false), p(SpeedDown));
        assert_eq!(map_key(vk::OEM_6, false, false), p(SpeedUp));
        assert_eq!(map_key(vk::BACK, false, false), p(SpeedReset));
        assert_eq!(map_key(vk::UP, false, false), p(VolumeBy(5)));
        assert_eq!(map_key(vk::DOWN, false, false), p(VolumeBy(-5)));
        assert_eq!(map_key(vk::M, false, false), p(ToggleMute));
        assert_eq!(map_key(vk::F, false, false), Some(Action::ToggleFullscreen));
        assert_eq!(
            map_key(vk::RETURN, false, false),
            Some(Action::ToggleFullscreen)
        );
        assert_eq!(
            map_key(vk::ESCAPE, false, false),
            Some(Action::ExitFullscreen)
        );
        assert_eq!(map_key(vk::O, false, true), Some(Action::OpenDialog));
    }

    #[test]
    fn unknown_and_ctrl_keys_are_ignored() {
        assert_eq!(map_key(0x41, false, false), None);
        assert_eq!(map_key(vk::SPACE, false, true), None);
        assert_eq!(map_key(vk::O, false, false), None);
    }

    #[test]
    fn wheel_accumulates_partial_notches() {
        let mut acc = 0;
        assert_eq!(wheel_steps(&mut acc, 60), 0);
        assert_eq!(wheel_steps(&mut acc, 60), 1);
        assert_eq!(wheel_steps(&mut acc, -240), -2);
        assert_eq!(acc, 0);
    }
}
