use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};

const IDLE: u8 = 0;
const ARMED: u8 = 1;
const COMPLETED: u8 = 2;
const TAKEN_OVER: u8 = 3;
const STARTED: u8 = 4;
const IDLE_STARTED: u8 = 5;
pub(crate) const QUIT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const RESTORE_BUDGET: Duration = Duration::from_secs(2);

pub(crate) struct QuitTiming {
    pub(crate) timeout: Duration,
    pub(crate) graceful_cap: Duration,
    pub(crate) budget: Duration,
}

impl Default for QuitTiming {
    fn default() -> Self {
        Self {
            timeout: QUIT_TIMEOUT,
            graceful_cap: Duration::from_secs(30),
            budget: RESTORE_BUDGET,
        }
    }
}

type Recovery = dyn Fn(Instant) + Send + Sync;
type Exit = dyn Fn(i32) + Send + Sync;

pub(crate) struct QuitFallback {
    state: Arc<AtomicU8>,
    completed: mpsc::Sender<()>,
    completion: Mutex<Option<mpsc::Receiver<()>>>,
    cancelled: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    stop_tray: Arc<dyn Fn() + Send + Sync>,
    recover: Arc<Recovery>,
    exit: Arc<Exit>,
    timing: QuitTiming,
}

impl QuitFallback {
    pub(crate) fn new(
        cancelled: Arc<AtomicBool>,
        epoch: Arc<AtomicU64>,
        stop_tray: impl Fn() + Send + Sync + 'static,
        recover: impl Fn(Instant) + Send + Sync + 'static,
        exit: impl Fn(i32) + Send + Sync + 'static,
        timing: QuitTiming,
    ) -> Self {
        let (completed, completion) = mpsc::channel();
        Self {
            state: Arc::new(AtomicU8::new(IDLE)),
            completed,
            completion: Mutex::new(Some(completion)),
            cancelled,
            epoch,
            stop_tray: Arc::new(stop_tray),
            recover: Arc::new(recover),
            exit: Arc::new(exit),
            timing,
        }
    }

    pub(crate) fn arm(&self) {
        if self
            .state
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |state| match state {
                IDLE => Some(ARMED),
                IDLE_STARTED => Some(STARTED),
                _ => None,
            })
            .is_err()
        {
            return;
        }
        let completion = self.completion.lock().unwrap().take().unwrap();
        let state = self.state.clone();
        let cancelled = self.cancelled.clone();
        let epoch = self.epoch.clone();
        let stop_tray = self.stop_tray.clone();
        let recover = self.recover.clone();
        let exit = self.exit.clone();
        let armed_at = Instant::now();
        let short_deadline = armed_at + self.timing.timeout;
        let graceful_deadline = armed_at + self.timing.graceful_cap;
        let budget = self.timing.budget;
        if let Err(error) = std::thread::Builder::new()
            .name("tray-quit-deadline".into())
            .spawn(move || {
                loop {
                    let current = state.load(Ordering::SeqCst);
                    let deadline = match current {
                        ARMED => short_deadline,
                        STARTED => graceful_deadline,
                        _ => return,
                    };
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        if state
                            .compare_exchange(
                                current,
                                TAKEN_OVER,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            )
                            .is_ok()
                        {
                            break;
                        }
                        continue;
                    }
                    if matches!(
                        completion.recv_timeout(remaining),
                        Err(mpsc::RecvTimeoutError::Disconnected)
                    ) {
                        std::thread::sleep(remaining);
                    }
                }
                cancelled.store(true, Ordering::SeqCst);
                epoch.fetch_add(1, Ordering::SeqCst);
                stop_tray();
                let deadline = Instant::now() + budget;
                let (done, restored) = mpsc::channel();
                let _ = std::thread::Builder::new()
                    .name("tray-quit-restore".into())
                    .spawn(move || {
                        recover(deadline);
                        let _ = done.send(());
                    });
                let _ = restored.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                exit(0);
            })
        {
            tracing::warn!(
                "Failed to start tray Quit deadline controller: {error}; forwarding ordinary Exit"
            );
        }
    }

    pub(crate) fn started(&self) {
        if self
            .state
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |state| match state {
                IDLE => Some(IDLE_STARTED),
                ARMED => Some(STARTED),
                _ => None,
            })
            .is_ok()
        {
            let _ = self.completed.send(());
        }
    }

    pub(crate) fn complete(&self) -> bool {
        let won = self
            .state
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |state| {
                matches!(state, IDLE | ARMED | IDLE_STARTED | STARTED).then_some(COMPLETED)
            })
            .is_ok();
        if won {
            let _ = self.completed.send(());
        }
        won
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::{dispatch_menu_event, TrayEvent};
    use std::sync::Barrier;

    const SHORT: Duration = Duration::from_millis(30);
    const MARGIN: Duration = Duration::from_secs(2);

    fn coordinator(
        restore: impl Fn(Instant) + Send + Sync + 'static,
        timeout: Duration,
        budget: Duration,
    ) -> (Arc<QuitFallback>, mpsc::Receiver<i32>) {
        let (exit, exited) = mpsc::channel();
        (
            Arc::new(QuitFallback::new(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicU64::new(7)),
                || {},
                restore,
                move |code| {
                    exit.send(code).unwrap();
                },
                QuitTiming {
                    timeout,
                    graceful_cap: timeout * 6,
                    budget,
                },
            )),
            exited,
        )
    }

    #[test]
    fn blocked_event_loop_publishes_cancellation_restores_and_exits() {
        let (restored, restore_seen) = mpsc::channel();
        let (quit, exited) = coordinator(
            move |_| {
                restored.send(()).unwrap();
            },
            SHORT,
            SHORT,
        );
        let started = Instant::now();
        quit.arm();
        restore_seen.recv_timeout(MARGIN).unwrap();
        assert!(quit.cancelled.load(Ordering::SeqCst));
        assert_eq!(quit.epoch.load(Ordering::SeqCst), 8);
        assert_eq!(exited.recv_timeout(MARGIN).unwrap(), 0);
        assert!(started.elapsed() < SHORT + SHORT + MARGIN);
        assert!(!quit.complete());
    }

    #[test]
    fn graceful_completion_prevents_restore_and_exit() {
        let (restored, restore_seen) = mpsc::channel();
        let (quit, exited) = coordinator(
            move |_| {
                restored.send(()).unwrap();
            },
            SHORT,
            SHORT,
        );
        quit.arm();
        assert!(quit.complete());
        assert!(exited.recv_timeout(SHORT + SHORT).is_err());
        assert!(restore_seen.try_recv().is_err());
        assert!(!quit.cancelled.load(Ordering::SeqCst));
        assert_eq!(quit.epoch.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn started_shutdown_can_complete_after_short_timeout() {
        let (restored, restore_seen) = mpsc::channel();
        let timeout = Duration::from_millis(100);
        let (quit, exited) = coordinator(
            move |_| {
                restored.send(()).unwrap();
            },
            timeout,
            SHORT,
        );
        quit.started();
        quit.arm();
        assert!(matches!(
            exited.recv_timeout(timeout * 2),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(quit.complete());
        assert!(exited.recv_timeout(timeout * 6).is_err());
        assert!(restore_seen.try_recv().is_err());
        assert!(!quit.cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn started_shutdown_still_restores_and_exits_at_hard_cap() {
        let (restored, restore_seen) = mpsc::channel();
        let timeout = Duration::from_millis(100);
        let (quit, exited) = coordinator(
            move |_| {
                restored.send(()).unwrap();
            },
            timeout,
            SHORT,
        );
        let armed_at = Instant::now();
        quit.arm();
        quit.started();
        assert_eq!(
            exited.recv_timeout(timeout * 6 + SHORT + MARGIN).unwrap(),
            0
        );
        assert!(armed_at.elapsed() >= timeout * 6);
        assert!(armed_at.elapsed() < timeout * 6 + SHORT + MARGIN);
        restore_seen.recv_timeout(MARGIN).unwrap();
        assert!(quit.cancelled.load(Ordering::SeqCst));
        assert!(!quit.complete());
    }

    #[test]
    fn stuck_restore_cannot_hold_exit_past_budget() {
        let (entered, entered_rx) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let released = Mutex::new(released);
        let (quit, exited) = coordinator(
            move |_| {
                entered.send(()).unwrap();
                released.lock().unwrap().recv().unwrap();
            },
            SHORT,
            SHORT,
        );
        let started = Instant::now();
        quit.arm();
        entered_rx.recv_timeout(MARGIN).unwrap();
        let result = exited.recv_timeout(MARGIN);
        release.send(()).unwrap();
        assert_eq!(result.unwrap(), 0);
        assert!(started.elapsed() < SHORT + SHORT + MARGIN);
    }

    #[test]
    fn exit_dispatch_arms_once_even_when_forwarding_fails() {
        let (quit, exited) = coordinator(|_| {}, SHORT, SHORT);
        let (sender, receiver) = mpsc::channel::<TrayEvent>();
        drop(receiver);
        assert!(dispatch_menu_event("exit", &sender, &quit).is_err());
        assert!(dispatch_menu_event("exit", &sender, &quit).is_err());
        assert_eq!(exited.recv_timeout(MARGIN).unwrap(), 0);
        assert_eq!(quit.epoch.load(Ordering::SeqCst), 8);
        assert!(exited.try_recv().is_err());
    }

    #[test]
    fn completion_and_takeover_have_exactly_one_winner() {
        for _ in 0..20 {
            let (quit, exited) = coordinator(|_| {}, Duration::ZERO, SHORT);
            let barrier = Arc::new(Barrier::new(2));
            let contender = quit.clone();
            let ready = barrier.clone();
            let completion = std::thread::spawn(move || {
                ready.wait();
                contender.complete()
            });
            quit.arm();
            barrier.wait();
            if completion.join().unwrap() {
                assert!(exited.recv_timeout(SHORT).is_err());
                assert!(!quit.cancelled.load(Ordering::SeqCst));
            } else {
                assert_eq!(exited.recv_timeout(MARGIN).unwrap(), 0);
                assert!(quit.cancelled.load(Ordering::SeqCst));
            }
            assert!(!quit.complete());
            assert!(exited.try_recv().is_err());
        }
    }

    #[test]
    fn tray_quit_recovers_blocked_fixture_subprocess() {
        const CHILD: &str = "LEOPARDWM_QUIT_FIXTURE_HWND";
        if let Ok(id) = std::env::var(CHILD) {
            let id: u64 = id.parse().unwrap();
            let quit = QuitFallback::new(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicU64::new(0)),
                || {},
                move |deadline| {
                    leopardwm_platform_win32::emergency_restore_windows(&[id], deadline)
                },
                |code| std::process::exit(code),
                QuitTiming::default(),
            );
            let (sender, receiver) = mpsc::channel();
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
            event_tx.try_send(TrayEvent::Refresh).unwrap();
            std::thread::spawn(move || {
                while let Ok(event) = receiver.recv() {
                    if event_tx.blocking_send(event).is_err() {
                        break;
                    }
                }
            });
            dispatch_menu_event("exit", &sender, &quit).unwrap();
            loop {
                std::thread::park();
            }
        }
        run_fixture_parent(CHILD);
    }

    fn run_fixture_parent(child_env: &str) {
        use windows::core::w;
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowRect, PeekMessageW,
            TranslateMessage, MSG, PM_REMOVE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        };
        let (ready, fixture_id) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let fixture = std::thread::spawn(move || unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("LeopardWM Quit fixture"),
                WS_POPUP,
                leopardwm_platform_win32::MOVE_OFFSCREEN_SENTINEL_COORD,
                leopardwm_platform_win32::MOVE_OFFSCREEN_SENTINEL_COORD,
                120,
                80,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let id = hwnd.0 as usize as u64;
            leopardwm_platform_win32::move_window_offscreen(id).unwrap();
            let mut parked = RECT::default();
            GetWindowRect(hwnd, &mut parked).unwrap();
            ready.send((id, parked.left, parked.top)).unwrap();
            resumed.recv().unwrap();
            let primary = leopardwm_platform_win32::get_primary_monitor()
                .unwrap()
                .work_area;
            let until = Instant::now() + MARGIN;
            let mut restored = false;
            while Instant::now() < until {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect).unwrap();
                restored = rect.left >= primary.x
                    && rect.top >= primary.y
                    && rect.right <= primary.x + primary.width
                    && rect.bottom <= primary.y + primary.height;
                if restored {
                    break;
                }
                std::thread::yield_now();
            }
            let _ = DestroyWindow(hwnd);
            result_tx.send(restored).unwrap();
        });
        let (id, x, y) = fixture_id.recv_timeout(MARGIN).unwrap();
        assert!(leopardwm_platform_win32::is_move_offscreen_sentinel_position(x, y));
        let started = Instant::now();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "quit_fallback::tests::tray_quit_recovers_blocked_fixture_subprocess",
                "--nocapture",
            ])
            .env(child_env, id.to_string())
            .spawn()
            .unwrap();
        let bound = QUIT_TIMEOUT + RESTORE_BUDGET + MARGIN;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if started.elapsed() >= bound {
                child.kill().unwrap();
                child.wait().unwrap();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let was_blocked = !fixture.is_finished() && result_rx.try_recv().is_err();
        resume.send(()).unwrap();
        let restored = result_rx.recv_timeout(MARGIN + MARGIN).unwrap();
        fixture.join().unwrap();
        assert!(was_blocked, "fixture must not pump until child exits");
        assert_eq!(
            status
                .expect("tray Quit child exceeded its exit deadline")
                .code(),
            Some(0)
        );
        assert!(
            restored,
            "fixture did not return inside primary work area after pumping"
        );
    }
}
