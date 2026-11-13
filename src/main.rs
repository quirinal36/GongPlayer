//! GongPlayer: Windows용 초경량 동영상 플레이어.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_out;
mod clock;
mod gfx;
mod media;
mod player;
mod queue;
mod render;
mod ring;
mod shared;
mod ui;

fn main() {
    shared::timing("main 시작");
    // 탐색기의 "연결 프로그램"과 패키지 파일 연결도 첫 인자로 경로를 넘깁니다.
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if let Err(e) = ui::window::run(path) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
