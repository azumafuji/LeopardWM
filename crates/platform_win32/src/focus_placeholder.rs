//! Transparent foreground target used when an empty workspace needs to release focus.

use crate::Win32Error;
use leopardwm_core_layout::{Rect, WindowId};
use std::ffi::c_void;
use std::sync::mpsc;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, PostMessageW, RegisterClassW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, UnregisterClassW, HWND_TOP,
    LWA_ALPHA, MSG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, WM_USER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};

const WINDOW_CLASS: &str = "LeopardWMFocusPlaceholder";
const OWNER_CLASS: &str = "LeopardWMFocusPlaceholderOwner";
const WM_STOP: u32 = WM_USER + 120;

fn work_area_center(work_area: Rect) -> (i32, i32) {
    (
        work_area.x.saturating_add(work_area.width.max(1) / 2),
        work_area.y.saturating_add(work_area.height.max(1) / 2),
    )
}

/// Identify the internal focus target without treating it as a managed window.
pub fn is_focus_placeholder(hwnd: WindowId) -> bool {
    let mut class_name = [0u16; 64];
    let length = unsafe { GetClassNameW(HWND(hwnd as *mut c_void), &mut class_name) };
    length > 0 && String::from_utf16_lossy(&class_name[..length as usize]) == WINDOW_CLASS
}

/// Owns a transparent, click-through window that can safely receive foreground.
pub struct FocusPlaceholder {
    hwnd: isize,
    thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FocusPlaceholder {
    /// Create the placeholder and its hidden owner on a dedicated message-loop thread.
    pub fn new() -> Result<Self, Win32Error> {
        #[cfg(test)]
        panic!("FocusPlaceholder::new creates a native window; do not construct it in unit tests");
        #[allow(unreachable_code)]
        let (init_tx, init_rx) = mpsc::channel::<Result<(isize, u32), Win32Error>>();
        let thread = std::thread::Builder::new()
            .name("focus-placeholder".into())
            .spawn(move || unsafe {
                let class_name: Vec<u16> = format!("{WINDOW_CLASS}\0").encode_utf16().collect();
                let owner_class_name: Vec<u16> =
                    format!("{OWNER_CLASS}\0").encode_utf16().collect();
                let class_name_ptr = windows::core::PCWSTR(class_name.as_ptr());
                let owner_class_name_ptr = windows::core::PCWSTR(owner_class_name.as_ptr());
                let class = WNDCLASSW {
                    lpfnWndProc: Some(placeholder_window_proc),
                    lpszClassName: class_name_ptr,
                    ..Default::default()
                };
                let owner_class = WNDCLASSW {
                    lpfnWndProc: Some(placeholder_window_proc),
                    lpszClassName: owner_class_name_ptr,
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 || RegisterClassW(&owner_class) == 0 {
                    let _ = init_tx.send(Err(Win32Error::HookInstallFailed(
                        "Failed to register focus-placeholder window classes".into(),
                    )));
                    let _ = UnregisterClassW(class_name_ptr, None);
                    let _ = UnregisterClassW(owner_class_name_ptr, None);
                    return;
                }

                let owner = match CreateWindowExW(
                    Default::default(),
                    owner_class_name_ptr,
                    None,
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    None,
                    None,
                ) {
                    Ok(hwnd) => hwnd,
                    Err(error) => {
                        let _ = init_tx.send(Err(Win32Error::HookInstallFailed(format!(
                            "Failed to create focus-placeholder owner: {error}"
                        ))));
                        let _ = UnregisterClassW(class_name_ptr, None);
                        let _ = UnregisterClassW(owner_class_name_ptr, None);
                        return;
                    }
                };

                let placeholder = match CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT,
                    class_name_ptr,
                    None,
                    WS_POPUP | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    Some(owner),
                    None,
                    None,
                    None,
                ) {
                    Ok(hwnd) => hwnd,
                    Err(error) => {
                        let _ = init_tx.send(Err(Win32Error::HookInstallFailed(format!(
                            "Failed to create focus placeholder: {error}"
                        ))));
                        let _ = DestroyWindow(owner);
                        let _ = UnregisterClassW(class_name_ptr, None);
                        let _ = UnregisterClassW(owner_class_name_ptr, None);
                        return;
                    }
                };
                if let Err(error) =
                    SetLayeredWindowAttributes(placeholder, COLORREF(0), 0, LWA_ALPHA)
                {
                    let _ = init_tx.send(Err(Win32Error::HookInstallFailed(format!(
                        "Failed to make focus placeholder transparent: {error}"
                    ))));
                    let _ = DestroyWindow(placeholder);
                    let _ = DestroyWindow(owner);
                    let _ = UnregisterClassW(class_name_ptr, None);
                    let _ = UnregisterClassW(owner_class_name_ptr, None);
                    return;
                }

                let thread_id = GetCurrentThreadId();
                if init_tx
                    .send(Ok((placeholder.0 as isize, thread_id)))
                    .is_err()
                {
                    let _ = DestroyWindow(placeholder);
                    let _ = DestroyWindow(owner);
                    let _ = UnregisterClassW(class_name_ptr, None);
                    let _ = UnregisterClassW(owner_class_name_ptr, None);
                    return;
                }

                let mut msg = MSG::default();
                loop {
                    let result = GetMessageW(&mut msg, None, 0, 0).0;
                    if result <= 0 || msg.message == WM_STOP {
                        break;
                    }
                    let _ = DispatchMessageW(&msg);
                }
                let _ = DestroyWindow(placeholder);
                let _ = DestroyWindow(owner);
                let _ = UnregisterClassW(class_name_ptr, None);
                let _ = UnregisterClassW(owner_class_name_ptr, None);
            })
            .map_err(|error| {
                Win32Error::HookInstallFailed(format!(
                    "Failed to spawn focus-placeholder thread: {error}"
                ))
            })?;

        match init_rx.recv() {
            Ok(Ok((hwnd, thread_id))) => Ok(Self {
                hwnd,
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(Win32Error::HookInstallFailed(
                    "Focus-placeholder thread exited during initialization".into(),
                ))
            }
        }
    }

    /// Move the placeholder onto the selected monitor and transfer foreground only
    /// if the parked window remains foreground throughout the handoff.
    pub fn release_foreground(
        &self,
        expected: WindowId,
        work_area: Rect,
    ) -> Result<bool, Win32Error> {
        let expected_hwnd = crate::window_id_to_hwnd(expected)?;
        if unsafe { GetForegroundWindow() } != expected_hwnd {
            return Ok(false);
        }

        let hwnd = HWND(self.hwnd as *mut c_void);
        let (x, y) = work_area_center(work_area);
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                x,
                y,
                1,
                1,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_SHOWWINDOW,
            )
            .map_err(|error| {
                Win32Error::SetPositionFailed(format!(
                    "Could not position focus placeholder: {error}"
                ))
            })?;
        }

        let expected_thread = unsafe { GetWindowThreadProcessId(expected_hwnd, None) };
        if expected_thread == 0 {
            return Err(Win32Error::SetPositionFailed(format!(
                "GetWindowThreadProcessId returned 0 for window {expected}"
            )));
        }
        let current_thread = unsafe { GetCurrentThreadId() };
        let mut attached = Vec::new();
        for thread_id in [expected_thread, self.thread_id] {
            if thread_id == current_thread || attached.contains(&thread_id) {
                continue;
            }
            if !unsafe { AttachThreadInput(current_thread, thread_id, true) }.as_bool() {
                detach_input_threads(current_thread, &attached);
                return Err(Win32Error::SetPositionFailed(format!(
                    "AttachThreadInput attach failed (current_thread={current_thread}, other_thread={thread_id})"
                )));
            }
            attached.push(thread_id);
        }

        let foreground = unsafe { GetForegroundWindow() };
        let foreground_set = if foreground == expected_hwnd {
            unsafe { SetForegroundWindow(hwnd).as_bool() }
        } else {
            false
        };
        detach_input_threads(current_thread, &attached);
        Ok(foreground_set)
    }
}

impl Drop for FocusPlaceholder {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut c_void)),
                WM_STOP,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn detach_input_threads(current_thread: u32, attached: &[u32]) {
    for thread_id in attached.iter().rev() {
        if !unsafe { AttachThreadInput(current_thread, *thread_id, false) }.as_bool() {
            tracing::warn!(
                "AttachThreadInput detach failed (current_thread={}, other_thread={})",
                current_thread,
                thread_id
            );
        }
    }
}

unsafe extern "system" fn placeholder_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::work_area_center;
    use leopardwm_core_layout::Rect;

    #[test]
    fn placeholder_position_uses_monitor_work_area_center() {
        assert_eq!(
            work_area_center(Rect::new(-1920, 100, 1920, 800)),
            (-960, 500)
        );
    }
}
