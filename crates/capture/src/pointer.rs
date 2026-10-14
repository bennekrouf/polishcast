//! Pointer logging by polling the system's pointer position and button state, on every
//! platform: no hooks, so no Accessibility (macOS) or input-monitoring permission, and nothing
//! that could slow down the pointer itself.
//!
//! Positions are in the platform's global desktop coordinates (points on macOS, physical
//! pixels on Windows), turned into the take's normalized coordinates by `event_log`.

use polishcast_core::{Button, Click, CursorSample, EventLog};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use sys::{location, prepare_thread};

#[derive(Default)]
pub struct Raw {
    /// When, and where in global coordinates.
    pub moves: Vec<(Instant, f64, f64)>,
    pub clicks: Vec<(Instant, f64, f64, Button)>,
}

pub struct Logger {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Raw>,
}

/// Fast enough that a 60 fps recording gets a sample per frame or better.
const INTERVAL: Duration = Duration::from_micros(4_167);

impl Logger {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            sys::prepare_thread();
            let mut raw = Raw::default();
            let mut last = None;
            let mut down = [false; 2];
            while !flag.load(Ordering::Relaxed) {
                let now = Instant::now();
                if let Some(p) = location() {
                    if last != Some(p) {
                        raw.moves.push((now, p.0, p.1));
                        last = Some(p);
                    }
                    for (i, b) in [Button::Left, Button::Right].into_iter().enumerate() {
                        let is_down = sys::pressed(b);
                        if is_down && !down[i] {
                            raw.clicks.push((now, p.0, p.1, b));
                        }
                        down[i] = is_down;
                    }
                }
                std::thread::sleep(INTERVAL);
            }
            raw
        });
        Self { stop, thread }
    }

    pub fn finish(self) -> Raw {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap_or_default()
    }
}

/// The take's event log: times from `started` (the first frame), positions normalized to the
/// recorded display at `origin` with `size`, in the same units as `raw`. Samples from before the
/// first frame are dropped, except the last one, which is where the pointer was at time 0.
pub fn event_log(raw: &Raw, started: Instant, origin: (f64, f64), size: (f64, f64)) -> EventLog {
    let norm = |x: f64, y: f64| (((x - origin.0) / size.0) as f32, ((y - origin.1) / size.1) as f32);
    let since = |at: Instant| at.checked_duration_since(started).map(|d| d.as_secs_f64());
    let mut log = EventLog::default();
    if let Some(&(_, x, y)) = raw.moves.iter().rev().find(|m| since(m.0).is_none()) {
        let (x, y) = norm(x, y);
        log.cursor.push(CursorSample { t: 0.0, x, y });
    }
    for &(at, x, y) in &raw.moves {
        if let Some(t) = since(at) {
            let (x, y) = norm(x, y);
            log.cursor.push(CursorSample { t, x, y });
        }
    }
    for &(at, x, y, button) in &raw.clicks {
        if let Some(t) = since(at) {
            let (x, y) = norm(x, y);
            log.clicks.push(Click { t, x, y, button });
        }
    }
    log
}

#[cfg(target_os = "macos")]
mod sys {
    //! Core Graphics: positions in global display points, origin at the main display's
    //! top-left.

    use polishcast_core::Button;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    /// kCGEventSourceStateCombinedSessionState
    const COMBINED_SESSION: i32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    pub fn prepare_thread() {}

    pub fn location() -> Option<(f64, f64)> {
        // SAFETY: CGEventCreate(NULL) returns a new event (or NULL) that we own and release.
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let p = CGEventGetLocation(event);
            CFRelease(event);
            Some((p.x, p.y))
        }
    }

    pub fn pressed(button: Button) -> bool {
        let index = match button {
            Button::Left => 0,
            Button::Right => 1,
        };
        // SAFETY: a plain query with no pointers.
        unsafe { CGEventSourceButtonState(COMBINED_SESSION, index) }
    }
}

#[cfg(windows)]
mod sys {
    //! Win32: positions in physical pixels on the virtual desktop, origin at the primary
    //! monitor's top-left.

    use polishcast_core::Button;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    /// Physical pixels, whatever the display scaling: the same units as the monitor's
    /// rectangle and the captured frames.
    pub fn prepare_thread() {
        // SAFETY: changes only this thread's DPI awareness.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    pub fn location() -> Option<(f64, f64)> {
        let mut p = POINT::default();
        // SAFETY: writes into `p`, which outlives the call.
        unsafe { GetCursorPos(&mut p) }.ok()?;
        Some((p.x as f64, p.y as f64))
    }

    pub fn pressed(button: Button) -> bool {
        let key = match button {
            Button::Left => VK_LBUTTON,
            Button::Right => VK_RBUTTON,
        };
        // SAFETY: a plain query. The high bit is set while the button is down.
        unsafe { GetAsyncKeyState(key.0 as i32) < 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_timed_from_the_first_frame_and_normalized() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let raw = Raw {
            moves: vec![(at(0), 100.0, 100.0), (at(50), 150.0, 125.0), (at(300), 300.0, 200.0)],
            clicks: vec![(at(40), 0.0, 0.0, Button::Left), (at(300), 300.0, 200.0, Button::Right)],
        };
        // A 400 × 200 display whose top-left is at (100, 100); the first frame came at 100 ms.
        let log = event_log(&raw, at(100), (100.0, 100.0), (400.0, 200.0));
        let cursor: Vec<_> = log.cursor.iter().map(|s| ((s.t * 1000.0).round(), s.x, s.y)).collect();
        assert_eq!(cursor, [(0.0, 0.125, 0.125), (200.0, 0.5, 0.5)], "the move at 50 ms is where it was at 0");
        assert_eq!(log.clicks.len(), 1, "the click before the first frame is dropped");
        assert_eq!(log.clicks[0].button, Button::Right);
    }
}
