use super::*;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, PostThreadMessageW, MSG,
    WM_QUIT, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

struct PlacementOwner {
    window_id: u64,
    thread_id: u32,
    release: mpsc::Sender<()>,
    pumping: mpsc::Receiver<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl PlacementOwner {
    fn spawn() -> Self {
        Self::spawn_at(Rect::new(48, 48, 160, 120))
    }

    fn spawn_at(rect: Rect) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let (pumping_tx, pumping) = mpsc::channel();
        let join = thread::spawn(move || unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("display-change placement fixture"),
                WS_POPUP,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            ready_tx
                .send((hwnd.0 as usize as u64, GetCurrentThreadId()))
                .unwrap();
            let _ = resume.recv_timeout(Duration::from_secs(20));
            pumping_tx.send(()).unwrap();
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
            pumping,
            join: Some(join),
        }
    }

    fn resume(&self) {
        self.release.send(()).unwrap();
        self.pumping.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

impl Drop for PlacementOwner {
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

fn placement_state(window_ids: &[u64]) -> AppState {
    let monitors = vec![MonitorInfo {
        id: 1,
        rect: Rect::new(0, 0, 800, 600),
        work_area: Rect::new(0, 0, 800, 560),
        is_primary: true,
        device_name: "DISPLAY1".to_string(),
        scale_factor: 1.0,
    }];
    let mut config = test_config();
    config.appearance.active_border = false;
    config.animation.layout_duration_ms = 0;
    config.animation.scroll_duration_ms = 0;
    let mut state = AppState::new_with_config(config, monitors.clone());
    state.reduce_motion = true;
    state.paused = false;
    state.injected_display_monitors = Some(monitors);
    for &hwnd in window_ids {
        state.workspaces.get_mut(&1).unwrap()[0]
            .insert_window(hwnd, Some(320))
            .unwrap();
        state
            .injected_window_info
            .insert(hwnd, make_test_window_info(hwnd));
    }
    state
}

#[test]
fn test_display_change_places_responsive_windows_while_owner_is_not_pumping() {
    let _serial = REAL_WINDOW_STYLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let blocked = PlacementOwner::spawn();
    let responsive = PlacementOwner::spawn();
    responsive.resume();
    let mut state = placement_state(&[blocked.window_id, responsive.window_id]);
    let before = leopardwm_platform_win32::get_window_visible_rect(blocked.window_id).unwrap();
    let start = Instant::now();
    state.handle_window_event(WindowEvent::DisplayChange);
    if let Some(retry) = state.display_change_apply_retry.as_ref() {
        let generation = retry.generation;
        let _ = state.run_display_change_apply_retry(generation);
    }
    let remained_active = !state.paused && state.pending_apply_workers.is_empty();
    let elapsed = start.elapsed();
    if !remained_active {
        blocked.resume();
        join_pending_test_apply_workers(&mut state);
    }
    assert!(
        remained_active,
        "display reconciliation paused behind a non-pumping owner"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "display apply waited on owner: {elapsed:?}"
    );
    assert_eq!(
        leopardwm_platform_win32::get_window_visible_rect(blocked.window_id).unwrap(),
        before
    );
    let expected: HashMap<_, _> = state.workspaces[&1][0]
        .compute_placements_animated(state.layout_viewport(1))
        .into_iter()
        .map(|placement| (placement.window_id, placement.rect))
        .collect();
    assert_eq!(
        leopardwm_platform_win32::get_window_visible_rect(responsive.window_id),
        Some(expected[&responsive.window_id])
    );
    assert!(!state.last_physical_presentations[&blocked.window_id].confirmed);

    // Outlive both the production apply/retry interval and the ordinary async expiry.
    thread::sleep(Duration::from_millis(10_100).saturating_sub(start.elapsed()));
    state.apply_layout().unwrap();
    assert!(!state.paused);
    assert!(!state.last_physical_presentations[&blocked.window_id].confirmed);
    blocked.resume();
    let deadline = Instant::now() + Duration::from_secs(2);
    while leopardwm_platform_win32::get_window_visible_rect(blocked.window_id)
        != Some(expected[&blocked.window_id])
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        leopardwm_platform_win32::get_window_visible_rect(blocked.window_id),
        Some(expected[&blocked.window_id])
    );
    state.apply_layout().unwrap();
    assert!(state.last_physical_presentations[&blocked.window_id].confirmed);
    assert!(!state.paused);
}

#[test]
fn test_display_change_retry_defers_non_pumping_owner() {
    let _serial = REAL_WINDOW_STYLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let blocked = PlacementOwner::spawn();
    let responsive = PlacementOwner::spawn();
    responsive.resume();
    let mut state = placement_state(&[blocked.window_id, responsive.window_id]);
    state.layout_apply_timeout = Duration::from_millis(10);
    state.injected_apply_placements_behavior = Some(TestApplyPlacementsBehavior::SleepAndSucceed(
        Duration::from_millis(50),
    ));
    state.handle_window_event(WindowEvent::DisplayChange);
    let generation = pending_display_change_retry_generation(&state);
    for worker in state.pending_apply_workers.drain(..) {
        worker.join().unwrap();
    }
    state.injected_apply_placements_behavior = None;
    state.layout_apply_timeout = Duration::from_secs(5);
    let start = Instant::now();
    let result = state.run_display_change_apply_retry(generation);
    let elapsed = start.elapsed();
    let remained_active = result.is_ok() && !state.paused && state.pending_apply_workers.is_empty();
    if !remained_active {
        blocked.resume();
        join_pending_test_apply_workers(&mut state);
    }
    assert!(
        remained_active,
        "display-change retry blocked or paused: {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "retry waited on owner: {elapsed:?}"
    );
    let expected = state.workspaces[&1][0]
        .compute_placements_animated(state.layout_viewport(1))
        .into_iter()
        .find(|placement| placement.window_id == responsive.window_id)
        .unwrap()
        .rect;
    assert_eq!(
        leopardwm_platform_win32::get_window_visible_rect(responsive.window_id),
        Some(expected)
    );
    assert!(!state.last_physical_presentations[&blocked.window_id].confirmed);
    assert!(state.display_change_apply_retry.is_none());
}

#[test]
fn test_display_change_size_only_deferral_waits_for_owner() {
    let _serial = REAL_WINDOW_STYLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let template = placement_state(&[100]);
    let expected =
        template.workspaces[&1][0].compute_placements_animated(template.layout_viewport(1))[0].rect;
    let before = Rect::new(
        expected.x,
        expected.y,
        expected.width - 20,
        expected.height - 20,
    );
    let blocked = PlacementOwner::spawn_at(before);
    let mut state = placement_state(&[blocked.window_id]);
    state.handle_window_event(WindowEvent::DisplayChange);
    assert!(!state.paused);
    let start = Instant::now();
    let result = state.apply_layout();
    let elapsed = start.elapsed();
    let stayed_deferred = result.is_ok() && !state.paused;
    if !stayed_deferred {
        blocked.resume();
        join_pending_test_apply_workers(&mut state);
    }
    assert!(
        stayed_deferred,
        "same-origin async resize drained before owner pumped: {result:?}"
    );
    assert!(elapsed < Duration::from_secs(2));
    assert_eq!(
        leopardwm_platform_win32::get_window_visible_rect(blocked.window_id),
        Some(before)
    );
    assert!(!state.last_physical_presentations[&blocked.window_id].confirmed);
    blocked.resume();
    let deadline = Instant::now() + Duration::from_secs(2);
    while leopardwm_platform_win32::get_window_visible_rect(blocked.window_id) != Some(expected)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        leopardwm_platform_win32::get_window_visible_rect(blocked.window_id),
        Some(expected)
    );
    state.apply_layout().unwrap();
    assert!(state.last_physical_presentations[&blocked.window_id].confirmed);
}
