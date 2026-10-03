use super::*;
use crate::event_handler::{AdmissionKind, AdmitOutcome};
use std::collections::HashSet;
use std::time::{Duration, Instant};

fn admit_pending_maximized() -> AppState {
    let mut state = AppState::new_with_config(test_config(), test_monitors());
    state.reduce_motion = true;
    state
        .injected_window_info
        .insert(100, make_test_window_info(100));
    assert_eq!(
        state.try_admit_window_at_with_native_ops(
            100,
            AdmissionKind::Automatic,
            None,
            |_| true,
            |_| Ok(true)
        ),
        AdmitOutcome::Admitted
    );
    state.layout_transition = None;
    state
}

#[test]
fn test_maximized_admission_results_ignore_stale_or_departed_lifetimes() {
    for departed in [false, true] {
        for still_maximized in [false, true] {
            let mut state = admit_pending_maximized();
            let token = state.managed_lifetime_tokens[&100];
            if departed {
                state.workspaces.get_mut(&1).unwrap()[0]
                    .remove_window(100)
                    .unwrap();
            } else {
                state.managed_lifetime_tokens.insert(100, token + 1);
            }
            let grace = state.window_last_maximized_at[&100];
            let placements = state.last_placed_layout_rects.clone();
            state.handle_window_event(WindowEvent::MaximizedAdmissionRestored {
                window_id: 100,
                managed_lifetime_token: token,
                still_maximized,
            });
            assert_eq!(state.window_last_maximized_at[&100], grace);
            assert!(state.pending_maximized_admission_restores.contains(&100));
            assert_eq!(state.last_placed_layout_rects, placements);
        }
    }
}

#[test]
fn test_maximized_admission_queue_failure_uses_current_native_state() {
    for still_maximized in [false, true] {
        let mut state = AppState::new_with_config(test_config(), test_monitors());
        state
            .injected_window_info
            .insert(100, make_test_window_info(100));
        let queries = std::cell::Cell::new(0);
        assert_eq!(
            state.try_admit_window_at_with_native_ops(
                100,
                AdmissionKind::Automatic,
                None,
                |_| {
                    let query = queries.get();
                    queries.set(query + 1);
                    query == 0 || still_maximized
                },
                |_| Err(leopardwm_platform_win32::Win32Error::WindowNotFound(100)),
            ),
            AdmitOutcome::Admitted
        );
        assert_eq!(queries.get(), 2);
        assert_eq!(
            state.window_last_maximized_at.contains_key(&100),
            still_maximized
        );
        assert!(!state.pending_maximized_admission_restores.contains(&100));
    }
}

#[test]
fn test_maximized_admission_without_report_keeps_grace_when_worker_restores_first() {
    let mut state = AppState::new_with_config(test_config(), test_monitors());
    state
        .injected_window_info
        .insert(100, make_test_window_info(100));
    let queued = std::cell::Cell::new(false);
    assert_eq!(
        state.try_admit_window_at_with_native_ops(
            100,
            AdmissionKind::Automatic,
            None,
            |_| !queued.get(),
            |_| {
                queued.set(true);
                Ok(false)
            },
        ),
        AdmitOutcome::Admitted
    );
    assert!(queued.get());
    assert!(state.window_last_maximized_at.contains_key(&100));
    assert!(!state.pending_maximized_admission_restores.contains(&100));
}

#[test]
fn test_unfocused_maximized_admission_keeps_per_app_column_width() {
    let mut config = test_config();
    config.behavior.focus_new_windows = false;
    config.window_rules.push(config::WindowRule {
        match_class: Some("TestWindowClass".into()),
        column_width: Some(0.35),
        ..Default::default()
    });
    let mut state = AppState::new_with_config(config, test_monitors());
    state
        .focused_workspace_mut()
        .unwrap()
        .insert_window(200, None)
        .unwrap();
    state
        .injected_window_info
        .insert(100, make_test_window_info(100));
    let width = (0.35 * f64::from(state.viewport_width_for(1))).round() as i32;
    assert_eq!(
        state.try_admit_window_at_with_native_ops(
            100,
            AdmissionKind::Automatic,
            None,
            |_| true,
            |_| Ok(true)
        ),
        AdmitOutcome::Admitted
    );
    state.paused = false;
    state.layout_transition = None;
    state.injected_apply_placements_behavior =
        Some(TestApplyPlacementsBehavior::SleepAndSucceed(Duration::ZERO));
    state.handle_window_event(WindowEvent::MaximizedAdmissionRestored {
        window_id: 100,
        managed_lifetime_token: state.managed_lifetime_tokens[&100],
        still_maximized: false,
    });
    let workspace = state.focused_workspace().unwrap();
    assert_eq!(workspace.focused_window(), Some(200));
    assert_eq!(
        workspace
            .columns()
            .iter()
            .find(|column| column.contains(100))
            .unwrap()
            .width(),
        width
    );
    assert_eq!(state.last_placed_layout_rects[&100].width, width);
}

#[test]
fn test_pending_maximized_admission_outlasts_grace_without_user_maximize_cleanup() {
    let mut state = admit_pending_maximized();
    let old = Instant::now() - Duration::from_secs(10);
    state.window_managed_at.insert(100, old);
    state.window_last_maximized_at.insert(100, old);
    state.ghost_sources_pending_safe_landing.insert(100);
    let placements = state
        .focused_workspace()
        .unwrap()
        .compute_placements_animated(state.layout_viewport(1));
    state.handle_maximized_placement_skips(&[100]);
    assert!(state.ghost_sources_pending_safe_landing.contains(&100));
    assert!(state
        .filter_physical_placements_observed(placements.clone(), &HashSet::new())
        .is_empty());
    state.handle_window_event(WindowEvent::MovedOrResized(100));
    assert!(state.ghost_sources_pending_safe_landing.contains(&100));
    state.handle_window_event(WindowEvent::MaximizedAdmissionRestored {
        window_id: 100,
        managed_lifetime_token: state.managed_lifetime_tokens[&100],
        still_maximized: false,
    });
    assert_eq!(
        state
            .filter_physical_placements_observed(placements, &HashSet::new())
            .len(),
        1
    );
}
