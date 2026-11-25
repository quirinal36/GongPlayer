//! Win32 창: 메시지 루프, 입력을 명령으로 바꾸기, 전체화면, DPI, 끌어다 놓기.
//!
//! UI 스레드는 FFmpeg와 Direct3D를 직접 부르지 않습니다. 세션과 렌더 스레드에
//! 명령과 상태만 넘깁니다.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use super::input::{Action, map_key, wheel_steps};
use super::layout::{Element, Layout, initial_window_size};
use crate::gfx::device::Gpu;
use crate::media::{MediaInfo, Session};
use crate::player::{Command, Effect, Player, State};
use crate::shared::{Event, EventSink, Shared, now};

const WM_APP_EVENT: u32 = WM_APP + 1;
/// UI_Controls 기능을 켜지 않으려고 직접 선언합니다.
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_APP_OPEN_DIALOG: u32 = WM_APP + 2;
const TIMER_UI: usize = 1;
/// 재생 중 입력이 없으면 컨트롤과 커서를 숨기기까지의 시간 (초).
const HIDE_AFTER: f64 = 2.0;
const OSD_SECS: f64 = 1.0;
const WARNING_SECS: f64 = 3.0;
const EMPTY_MESSAGE: &str = "파일을 끌어다 놓거나 Ctrl+O";
const APP_NAME: &str = "GongPlayer";

/// 다른 스레드가 UI 스레드를 깨울 때 씁니다.
pub fn post_wake(hwnd: isize, msg: u32) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
    }
}

enum Drag {
    Seek,
    Volume,
}

struct App {
    hwnd: HWND,
    shared: Arc<Shared>,
    events: Receiver<Event>,
    sink: EventSink,
    gpu: Option<Arc<Gpu>>,
    player: Player,
    session: Option<Session>,
    info: Option<MediaInfo>,
    path: Option<PathBuf>,
    pending_open: Option<PathBuf>,
    /// 디바이스를 잃은 뒤 다시 열 때 돌아갈 위치와 재생 여부.
    resume: Option<(f64, bool)>,
    error: Option<String>,
    fullscreen: Option<(WINDOWPLACEMENT, isize)>,
    last_input: f64,
    last_mouse: (i32, i32),
    controls: bool,
    cursor_hidden: bool,
    osd: Option<(String, f64)>,
    drag: Option<Drag>,
    drag_preview: Option<f64>,
    last_seek_epoch: u64,
    wheel: i32,
    hover: Option<Element>,
    tracking_leave: bool,
    timer_on: bool,
    awake: bool,
    render: Option<JoinHandle<()>>,
}

struct WindowData {
    shared: Arc<Shared>,
    scale: Cell<f32>,
    app: RefCell<Option<App>>,
}

pub fn run(initial: Option<PathBuf>) -> Result<(), String> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("GongPlayerWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: class,
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1 as _)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let shared = Arc::new(Shared::new());
        let data = Box::new(WindowData {
            shared: shared.clone(),
            scale: Cell::new(1.0),
            app: RefCell::new(None),
        });
        let data_ptr = Box::into_raw(data);

        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            class,
            &HSTRING::from(APP_NAME),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            960,
            600,
            None,
            None,
            Some(instance.into()),
            Some(data_ptr as *const _),
        )
        .map_err(|e| format!("창을 만들 수 없습니다: {e}"))?;
        let data = &*data_ptr;

        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = dpi as f32 / 96.0;
        data.scale.set(scale);
        // 빈 창의 크기를 DPI에 맞춥니다.
        let (cw, ch) = ((960.0 * scale) as i32, (540.0 * scale) as i32);
        set_client_size(hwnd, cw, ch, dpi, true);
        {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            *shared.size.lock().unwrap() = (rc.right.max(1) as u32, rc.bottom.max(1) as u32);
        }

        let (tx, rx) = channel();
        let sink = EventSink::new(tx, hwnd.0 as isize, WM_APP_EVENT);
        let render = crate::render::spawn(hwnd.0 as isize, shared.clone(), sink.clone());
        *data.app.borrow_mut() = Some(App {
            hwnd,
            shared: shared.clone(),
            events: rx,
            sink,
            gpu: None,
            player: Player::new(),
            session: None,
            info: None,
            path: None,
            pending_open: initial,
            resume: None,
            error: None,
            fullscreen: None,
            last_input: now(),
            last_mouse: (-1, -1),
            controls: false,
            cursor_hidden: false,
            osd: None,
            drag: None,
            drag_preview: None,
            last_seek_epoch: 0,
            wheel: 0,
            hover: None,
            tracking_leave: false,
            timer_on: false,
            awake: false,
            render: Some(render),
        });
        if let Some(app) = data.app.borrow_mut().as_mut() {
            app.sync_overlay();
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        crate::shared::timing("창 표시");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(Box::from_raw(data_ptr));
        CoUninitialize();
    }
    Ok(())
}

/// 클라이언트 영역이 `w`x`h`가 되도록 창 크기를 정합니다. `center`면 모니터 가운데로 옮깁니다.
fn set_client_size(hwnd: HWND, w: i32, h: i32, dpi: u32, center: bool) {
    unsafe {
        let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
        let ex = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        let _ = AdjustWindowRectExForDpi(&mut rc, style, false, ex, dpi);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        if center {
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let wa = mi.rcWork;
            let x = wa.left + ((wa.right - wa.left) - ww) / 2;
            let y = wa.top + ((wa.bottom - wa.top) - wh) / 2;
            let _ = SetWindowPos(
                hwnd,
                None,
                x.max(wa.left),
                y.max(wa.top),
                ww,
                wh,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        } else {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                ww,
                wh,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
            );
        }
    }
}

fn work_area(hwnd: HWND) -> (u32, u32) {
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        (
            (wa.right - wa.left).max(1) as u32,
            (wa.bottom - wa.top).max(1) as u32,
        )
    }
}

fn lparam_xy(l: LPARAM) -> (i32, i32) {
    let x = (l.0 & 0xffff) as u16 as i16 as i32;
    let y = ((l.0 >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

/// 파일 열기 대화상자. 앱 상태를 빌리지 않은 채로 부릅니다 (대화상자는 메시지 루프를 돌립니다).
fn open_dialog(hwnd: HWND) -> Option<PathBuf> {
    unsafe {
        let dlg: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filters = [
            COMDLG_FILTERSPEC {
                pszName: w!("동영상"),
                pszSpec: w!("*.mp4;*.m4v;*.mov;*.mkv;*.webm;*.ts;*.m2ts;*.mts;*.avi"),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("모든 파일"),
                pszSpec: w!("*.*"),
            },
        ];
        let _ = dlg.SetFileTypes(&filters);
        dlg.Show(Some(hwnd)).ok()?;
        let item = dlg.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path.map(PathBuf::from)
    }
}

fn dropped_file(hdrop: HDROP) -> Option<PathBuf> {
    unsafe {
        let n = DragQueryFileW(hdrop, 0, None);
        let mut buf = vec![0u16; n as usize + 1];
        let got = DragQueryFileW(hdrop, 0, Some(&mut buf));
        DragFinish(hdrop);
        (got > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..got as usize])))
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowData;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let data = &*ptr;

        // 앱 상태 없이 처리하는 메시지. 창 크기 조절 중의 재진입에도 안전합니다.
        match msg {
            WM_SIZE => {
                let (w, h) = ((lp.0 & 0xffff) as u32, ((lp.0 >> 16) & 0xffff) as u32);
                if w > 0 && h > 0 {
                    *data.shared.size.lock().unwrap() = (w, h);
                    data.shared.render_wake.notify();
                }
                // 앱이 쉬고 있으면 배치를 다시 맞춥니다.
                if let Ok(mut a) = data.app.try_borrow_mut()
                    && let Some(app) = a.as_mut()
                {
                    app.sync_overlay();
                }
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as u32;
                data.scale.set(dpi as f32 / 96.0);
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                data.shared.update_overlay(|o| o.scale = dpi as f32 / 96.0);
                return LRESULT(0);
            }
            WM_GETMINMAXINFO => {
                let mm = &mut *(lp.0 as *mut MINMAXINFO);
                let s = data.scale.get();
                mm.ptMinTrackSize = POINT {
                    x: (320.0 * s) as i32,
                    y: (200.0 * s) as i32,
                };
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                data.shared.render_wake.notify();
                return LRESULT(0);
            }
            WM_APP_OPEN_DIALOG => {
                if let Some(path) = open_dialog(hwnd)
                    && let Ok(mut a) = data.app.try_borrow_mut()
                    && let Some(app) = a.as_mut()
                {
                    app.open(path);
                }
                return LRESULT(0);
            }
            _ => {}
        }

        let handled = match data.app.try_borrow_mut() {
            Ok(mut a) => match a.as_mut() {
                Some(app) => app.handle(msg, wp, lp, data.scale.get()),
                None => None,
            },
            // 앱이 다른 메시지를 처리하는 중 (SetWindowPos 등에서 재진입)
            Err(_) => None,
        };
        match handled {
            Some(r) => r,
            None => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

impl App {
    /// 메시지를 처리했으면 `Some`.
    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM, scale: f32) -> Option<LRESULT> {
        match msg {
            WM_APP_EVENT => {
                self.drain_events();
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = wp.0 as u16;
                // Alt 조합은 시스템에 맡깁니다 (Alt+F4 등).
                if msg == WM_SYSKEYDOWN {
                    return None;
                }
                let action = map_key(vk, key_down(VK_SHIFT), key_down(VK_CONTROL))?;
                self.poke();
                self.act(action);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_xy(lp);
                if (x, y) != self.last_mouse {
                    self.last_mouse = (x, y);
                    self.poke();
                    self.track_leave();
                    self.mouse_move(x as f32, y as f32, scale);
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.tracking_leave = false;
                self.hover = None;
                self.last_mouse = (-1, -1);
                self.sync_overlay();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_xy(lp);
                self.poke();
                self.mouse_down(x as f32, y as f32, scale, false);
                Some(LRESULT(0))
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_xy(lp);
                self.poke();
                self.mouse_down(x as f32, y as f32, scale, true);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_xy(lp);
                self.mouse_up(x as f32, y as f32, scale);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16 as i32;
                let steps = wheel_steps(&mut self.wheel, delta);
                if steps != 0 {
                    self.poke();
                    self.command(Command::VolumeBy(steps * super::input::VOLUME_STEP));
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                if self.cursor_hidden && (lp.0 & 0xffff) as u32 == HTCLIENT {
                    unsafe { SetCursor(None) };
                    return Some(LRESULT(1));
                }
                None
            }
            WM_DROPFILES => {
                if let Some(path) = dropped_file(HDROP(wp.0 as *mut _)) {
                    self.open(path);
                    unsafe {
                        let _ = SetForegroundWindow(self.hwnd);
                    }
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_UI => {
                self.tick();
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                self.shutdown();
                unsafe {
                    let _ = DestroyWindow(self.hwnd);
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn shutdown(&mut self) {
        self.session = None;
        *self.shared.session.lock().unwrap() = None;
        self.shared.quit.store(true, Ordering::SeqCst);
        self.shared.render_wake.notify();
        if let Some(r) = self.render.take() {
            let _ = r.join();
        }
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }

    fn layout(&self) -> Layout {
        let (w, h) = *self.shared.size.lock().unwrap();
        let scale = self.shared.overlay.lock().unwrap().scale;
        Layout::new(w as f32, h as f32, scale)
    }

    fn controls_allowed(&self) -> bool {
        self.player.has_media()
    }

    fn track_leave(&mut self) {
        if self.tracking_leave {
            return;
        }
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut tme);
        }
        self.tracking_leave = true;
    }

    /// 입력이 있었습니다. 컨트롤과 커서를 보이고 숨김 타이머를 다시 셉니다.
    fn poke(&mut self) {
        self.last_input = now();
        self.cursor_hidden = false;
        if self.controls_allowed() && !self.controls {
            self.controls = true;
            self.sync_overlay();
        }
        self.update_timer();
    }

    fn mouse_move(&mut self, x: f32, y: f32, _scale: f32) {
        let l = self.layout();
        match self.drag {
            Some(Drag::Seek) => {
                let pos = l.seek_fraction(x) as f64 * self.player.duration();
                self.drag_preview = Some(pos);
                self.command(Command::SeekTo {
                    pos,
                    precise: false,
                });
                return;
            }
            Some(Drag::Volume) => {
                let v = l.volume_at(x);
                if v != self.player.volume() || self.player.muted() {
                    self.command(Command::SetVolume(v));
                }
                return;
            }
            None => {}
        }
        let hover = Some(l.hit(x, y, self.controls))
            .filter(|e| *e != Element::Video && *e != Element::Panel);
        if hover != self.hover {
            self.hover = hover;
            self.sync_overlay();
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32, _scale: f32, double: bool) {
        let l = self.layout();
        let el = l.hit(x, y, self.controls && self.controls_allowed());
        match el {
            Element::Video => {
                if double {
                    // 첫 클릭의 재생/일시정지를 되돌리고 전체화면을 바꿉니다.
                    self.command(Command::TogglePlay);
                    self.toggle_fullscreen();
                } else {
                    self.command(Command::TogglePlay);
                }
            }
            Element::SeekBar => {
                self.drag = Some(Drag::Seek);
                unsafe { SetCapture(self.hwnd) };
                let pos = l.seek_fraction(x) as f64 * self.player.duration();
                self.drag_preview = Some(pos);
                self.command(Command::SeekTo {
                    pos,
                    precise: false,
                });
            }
            Element::Volume => {
                self.drag = Some(Drag::Volume);
                unsafe { SetCapture(self.hwnd) };
                self.command(Command::SetVolume(l.volume_at(x)));
            }
            Element::Back => self.command(Command::SeekBy(-super::input::SEEK_SHORT)),
            Element::Forward => self.command(Command::SeekBy(super::input::SEEK_SHORT)),
            Element::Play => self.command(Command::TogglePlay),
            Element::Repeat => self.command(Command::ToggleRepeat),
            Element::Speed => self.command(Command::SpeedCycle),
            Element::Mute => self.command(Command::ToggleMute),
            Element::Fullscreen => self.toggle_fullscreen(),
            Element::Panel => {}
        }
    }

    fn mouse_up(&mut self, x: f32, _y: f32, _scale: f32) {
        let Some(drag) = self.drag.take() else { return };
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Drag::Seek = drag {
            let l = self.layout();
            let pos = l.seek_fraction(x) as f64 * self.player.duration();
            self.command(Command::SeekTo { pos, precise: true });
            self.drag_preview = None;
        }
        self.sync_overlay();
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Player(c) => self.command(c),
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::ExitFullscreen => {
                if self.fullscreen.is_some() {
                    self.toggle_fullscreen();
                }
            }
            Action::OpenDialog => post_wake(self.hwnd.0 as isize, WM_APP_OPEN_DIALOG),
        }
    }

    fn command(&mut self, c: Command) {
        let fx = self.player.handle(c, self.shared.position());
        self.apply(fx);
    }

    fn apply(&mut self, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::Seek { target, precise } => {
                    if let Some(s) = &self.session {
                        self.last_seek_epoch = s.seek(target, precise);
                    }
                }
                Effect::Pause(p) => {
                    if let Some(s) = &self.session {
                        s.set_paused(p);
                    }
                }
                Effect::Speed(v) => {
                    if let Some(s) = &self.session {
                        s.set_speed(v);
                    }
                }
                Effect::Gain(g) => {
                    if let Some(s) = &self.session {
                        s.set_gain(g);
                    }
                }
                Effect::Osd(text) => self.osd = Some((text, now() + OSD_SECS)),
            }
        }
        self.update_power();
        self.update_timer();
        self.sync_overlay();
    }

    fn update_power(&mut self) {
        let want = self.player.is_playing();
        if want != self.awake {
            self.awake = want;
            unsafe {
                if want {
                    SetThreadExecutionState(
                        ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED,
                    );
                } else {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
            }
        }
    }

    /// 자동 숨김이나 OSD가 필요할 때만 타이머를 켭니다. 일시정지 중에는 깨어나지 않습니다.
    fn update_timer(&mut self) {
        let need = (self.player.is_playing() && (self.controls || !self.cursor_hidden))
            || self.osd.is_some();
        if need != self.timer_on {
            self.timer_on = need;
            unsafe {
                if need {
                    SetTimer(Some(self.hwnd), TIMER_UI, 200, None);
                } else {
                    let _ = KillTimer(Some(self.hwnd), TIMER_UI);
                }
            }
        }
    }

    fn tick(&mut self) {
        let t = now();
        let mut changed = false;
        if let Some((_, until)) = &self.osd
            && t >= *until
        {
            self.osd = None;
            changed = true;
        }
        if self.player.is_playing() && self.drag.is_none() && t - self.last_input >= HIDE_AFTER {
            let over_controls = self.hover.is_some();
            if self.controls && !over_controls {
                self.controls = false;
                changed = true;
            }
            if !self.cursor_hidden && !over_controls {
                self.cursor_hidden = true;
                unsafe {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    // 커서가 창 안에 있을 때만 숨깁니다. WM_SETCURSOR가 다시 오게 합니다.
                    let _ = SetCursorPos(p.x, p.y);
                }
            }
        }
        if changed {
            self.sync_overlay();
        }
        self.update_timer();
    }

    fn sync_overlay(&mut self) {
        let has_media = self.player.has_media();
        if !has_media {
            self.controls = false;
        } else if !self.player.is_playing() {
            // 일시정지와 끝에서는 숨기지 않습니다.
            self.controls = true;
        }
        let message = match self.player.state() {
            State::Empty => Some(EMPTY_MESSAGE.to_string()),
            State::Error => self.error.clone(),
            _ => None,
        };
        let osd = self.osd.as_ref().map(|(s, _)| s.clone());
        let preview = self.drag_preview.or(self.player.pending_seek());
        let st = &self.player;
        let fullscreen = self.fullscreen.is_some();
        self.shared.update_overlay(|o| {
            o.controls = self.controls;
            o.hover = self.hover;
            o.playing = st.is_playing();
            o.repeat = st.repeat();
            o.volume = st.volume();
            o.muted = st.muted();
            o.speed = st.speed();
            o.duration = st.duration();
            o.seek_preview = preview;
            o.osd = osd;
            o.message = message;
            o.fullscreen = fullscreen;
        });
    }

    fn open(&mut self, path: PathBuf) {
        let Some(gpu) = self.gpu.clone() else {
            self.pending_open = Some(path);
            return;
        };
        *self.shared.session.lock().unwrap() = None;
        self.session = None;
        *self.shared.clock.lock().unwrap() = crate::clock::Clock::new();
        self.shared.audio_obs.clear();
        self.player.opening();
        self.last_seek_epoch = 0;
        self.info = None;
        self.error = None;
        self.drag_preview = None;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| APP_NAME.into());
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
        }
        let s = Session::open(path.clone(), gpu, self.shared.clone(), self.sink.clone());
        *self.shared.session.lock().unwrap() = Some(s.shared.clone());
        self.session = Some(s);
        self.path = Some(path);
        self.sync_overlay();
    }

    fn current_sid(&self) -> u64 {
        self.session.as_ref().map(|s| s.id()).unwrap_or(0)
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::Gpu(Ok(gpu)) => {
                    self.gpu = Some(gpu);
                    if let Some(p) = self.pending_open.take() {
                        self.open(p);
                    }
                }
                Event::Gpu(Err(e)) => {
                    self.player.failed();
                    self.error = Some(format!("화면을 표시할 수 없습니다: {e}"));
                    self.sync_overlay();
                }
                Event::GpuLost => {
                    // 새 디바이스가 오면 같은 위치에서 다시 엽니다.
                    if let Some(p) = self.path.clone()
                        && self.player.has_media()
                    {
                        self.resume = Some((self.shared.position(), self.player.is_playing()));
                        self.pending_open = Some(p);
                    }
                    self.gpu = None;
                    *self.shared.session.lock().unwrap() = None;
                    self.session = None;
                }
                Event::Opened { sid, info } if sid == self.current_sid() => self.opened(info),
                Event::OpenFailed { sid, message } if sid == self.current_sid() => {
                    self.player.failed();
                    self.error = Some(message);
                    *self.shared.session.lock().unwrap() = None;
                    self.session = None;
                    self.apply(Vec::new());
                }
                Event::SeekSettled { sid, epoch } if sid == self.current_sid() => {
                    if epoch >= self.last_seek_epoch {
                        self.player.seek_settled();
                        self.sync_overlay();
                    }
                }
                Event::Ended { sid, epoch } if sid == self.current_sid() => {
                    if epoch >= self.last_seek_epoch && self.player.pending_seek().is_none() {
                        let fx = self.player.ended();
                        self.apply(fx);
                    }
                }
                Event::Warning(text) => {
                    eprintln!("{text}");
                    self.osd = Some((text, now() + WARNING_SECS));
                    self.update_timer();
                    self.sync_overlay();
                }
                _ => {}
            }
        }
    }

    fn opened(&mut self, info: MediaInfo) {
        eprintln!(
            "열림: {}x{} {} 회전 {}° 화소비 {:.3} 길이 {:.1}s 소리 {}",
            info.width,
            info.height,
            info.codec,
            info.rotation,
            info.sar,
            info.duration,
            info.has_audio
        );
        // 창 크기를 영상에 맞춥니다. 전체화면이나 최대화 중이면 그대로 둡니다.
        let maximized = unsafe { IsZoomed(self.hwnd).as_bool() };
        if self.fullscreen.is_none() && !maximized && self.resume.is_none() {
            let (ww, wh) = work_area(self.hwnd);
            let (w, h) =
                initial_window_size(info.width, info.height, info.sar, info.rotated(), ww, wh);
            let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
            set_client_size(self.hwnd, w as i32, h as i32, dpi, true);
        }
        self.info = Some(info.clone());
        let mut fx = self.player.opened(info.duration);
        if let Some((pos, playing)) = self.resume.take() {
            fx.extend(
                self.player
                    .handle(Command::SeekTo { pos, precise: true }, 0.0),
            );
            if !playing {
                fx.extend(self.player.handle(Command::TogglePlay, pos));
            }
        }
        self.controls = true;
        self.last_input = now();
        self.apply(fx);
    }

    fn toggle_fullscreen(&mut self) {
        unsafe {
            if let Some((wp, style)) = self.fullscreen.take() {
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, style);
                let _ = SetWindowPlacement(self.hwnd, &wp);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
                );
            } else {
                let mut wp = WINDOWPLACEMENT {
                    length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                    ..Default::default()
                };
                let _ = GetWindowPlacement(self.hwnd, &mut wp);
                let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
                let mon = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let _ = GetMonitorInfoW(mon, &mut mi);
                let r = mi.rcMonitor;
                SetWindowLongPtrW(
                    self.hwnd,
                    GWL_STYLE,
                    style & !(WS_OVERLAPPEDWINDOW.0 as isize),
                );
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOP),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
                );
                self.fullscreen = Some((wp, style));
            }
        }
        self.sync_overlay();
    }
}
