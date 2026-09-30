use super::*;
use crate::event_handler::{AdmissionKind, AdmitOutcome};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, IsZoomed, PostThreadMessageW,
    ShowWindow, MSG, SW_SHOWMAXIMIZED, WM_QUIT, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_OVERLAPPEDWINDOW,
};

struct MaximizedOwner {
    window_id: u64,
    thread_id: u32,
    release: mpsc::Sender<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl MaximizedOwner {
    fn spawn() -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let join = thread::spawn(move || unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                None,
                WS_OVERLAPPEDWINDOW,
                48,
                48,
                320,
                240,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
            ready_tx
                .send((hwnd.0 as usize as u64, GetCurrentThreadId()))
                .unwrap();
            let _ = resume.recv_timeout(Duration::from_secs(2));
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                DispatchMessageW(&message);
            }
            DestroyWindow(hwnd).unwrap();
        });
        let (window_id, thread_id) = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            window_id,
            thread_id,
            release,
            join: Some(join),
        }
    }
}

impl Drop for MaximizedOwner {
    fn drop(&mut self) {
        let _ = self.release.send(());
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
    }
}

#[test]
fn test_maximized_admission_does_not_wait_for_non_pumping_owner() {
    let _serial = REAL_WINDOW_STYLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (_hooks, events) = leopardwm_platform_win32::install_event_hooks().unwrap();
    let owner = MaximizedOwner::spawn();
    let hwnd = owner.window_id;
    assert!(unsafe { IsZoomed(HWND(hwnd as *mut _)).as_bool() });
    let token = leopardwm_platform_win32::stamp_managed_lifetime_token(hwnd).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let (verify_tx, verify_rx) = mpsc::channel::<bool>();
    let admission = thread::spawn(move || {
        let mut config = test_config();
        config.behavior.disable_snap_layouts = false;
        config.behavior.focus_new_windows = false;
        config.animation.layout_duration_ms = 0;
        config.animation.scroll_duration_ms = 0;
        let mut state = AppState::new_with_config(config, test_monitors());
        state.reduce_motion = true;
        state.paused = false;
        state.next_injected_lifetime_token = token;
        state
            .injected_window_info
            .insert(hwnd, make_test_window_info(hwnd));
        let start = Instant::now();
        let outcome = state.try_admit_window(hwnd, AdmissionKind::Automatic);
        done_tx.send((outcome, start.elapsed())).unwrap();
        if !verify_rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            return;
        }
        assert!(leopardwm_platform_win32::wait_for_window_style_requests(
            Duration::from_secs(5)
        ));
        assert_eq!(outcome, AdmitOutcome::Admitted);
        assert!(!unsafe { IsZoomed(HWND(hwnd as *mut _)).as_bool() });
        assert!(state.window_last_maximized_at.contains_key(&hwnd));
        state.layout_transition = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.window_last_maximized_at.contains_key(&hwnd) && Instant::now() < deadline {
            let Ok(event) = events.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            // Feed only worker completions, not ambient desktop hook notifications.
            if matches!(
                event,
                WindowEvent::Created(..)
                    | WindowEvent::Destroyed(..)
                    | WindowEvent::Hidden(..)
                    | WindowEvent::Focused(..)
                    | WindowEvent::Minimized(..)
                    | WindowEvent::Restored(..)
                    | WindowEvent::MovedOrResized(..)
                    | WindowEvent::MoveSizeStart(..)
                    | WindowEvent::MoveSizeEnd(..)
                    | WindowEvent::DisplayChange
                    | WindowEvent::WorkAreaChanged
                    | WindowEvent::MouseEnterWindow(..)
                    | WindowEvent::MouseLeftManaged
                    | WindowEvent::TitleChanged(..)
            ) {
                continue;
            }
            state.handle_window_event(event);
        }
        assert!(
            !state.window_last_maximized_at.contains_key(&hwnd),
            "worker completion must remove grace"
        );
        let expected = state
            .focused_workspace()
            .unwrap()
            .compute_placements_animated(state.layout_viewport(1))
            .into_iter()
            .find(|placement| placement.window_id == hwnd)
            .unwrap()
            .rect;
        assert_eq!(state.last_placed_layout_rects.get(&hwnd), Some(&expected));
        let landed = leopardwm_platform_win32::get_window_visible_rect(hwnd).unwrap();
        assert!((landed.x - expected.x).abs() <= 2 && (landed.y - expected.y).abs() <= 2);
        assert!(
            (landed.width - expected.width).abs() <= 2
                && (landed.height - expected.height).abs() <= 2
        );
    });
    let fast = done_rx.recv_timeout(Duration::from_millis(400));
    let returned_while_blocked = fast.is_ok();
    let _ = owner.release.send(());
    let (outcome, elapsed) = fast.unwrap_or_else(|_| {
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("admission did not finish after owner resumed")
    });
    verify_tx.send(returned_while_blocked).unwrap();
    admission.join().unwrap();
    assert!(
        returned_while_blocked && elapsed < Duration::from_millis(400),
        "admission waited for non-pumping owner: {elapsed:?}"
    );
    assert_eq!(outcome, AdmitOutcome::Admitted);
}
