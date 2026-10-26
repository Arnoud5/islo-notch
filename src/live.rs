//! Live indicators: microphone / camera in use, and Windows Focus.
//!
//! * Mic & camera: Windows records every app that uses them under
//!   `HKCU\...\CapabilityAccessManager\ConsentStore\{microphone,webcam}`.
//!   An app is using it right now when `LastUsedTimeStart` is set and
//!   `LastUsedTimeStop` is 0. We ask Windows to signal an event when that key
//!   tree changes (`RegNotifyChangeKeyValue`), so the thread sleeps until then.
//! * Focus: `FocusSessionManager` (Windows 11) raises an event when Focus
//!   turns on or off.

use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::{PCWSTR, w};

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Privacy {
    pub mic: Option<String>,
    pub camera: Option<String>,
}

const STORE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore");

pub fn start_privacy(cb: impl Fn(Privacy) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("islo-privacy".into())
        .stack_size(256 * 1024)
        .spawn(move || unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, STORE, None, KEY_READ | KEY_NOTIFY, &mut key) != ERROR_SUCCESS {
                return;
            }
            let Ok(event) = CreateEventW(None, false, false, None) else { return };
            let mut last = Privacy::default();
            loop {
                // arm the notification first, then read, so no change slips between
                let armed = RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME, Some(event), true);
                let now = Privacy { mic: in_use(key, w!("microphone")), camera: in_use(key, w!("webcam")) };
                if now != last {
                    cb(now.clone());
                    last = now;
                }
                if armed != ERROR_SUCCESS {
                    return;
                }
                WaitForSingleObject(event, INFINITE);
            }
        });
}

/// Name of an app currently using `capability`, if any.
unsafe fn in_use(store: HKEY, capability: PCWSTR) -> Option<String> {
    unsafe {
        let mut cap = HKEY::default();
        if RegOpenKeyExW(store, capability, None, KEY_READ, &mut cap) != ERROR_SUCCESS {
            return None;
        }
        let mut found = scan(cap, false);
        if found.is_none() {
            let mut np = HKEY::default();
            if RegOpenKeyExW(cap, w!("NonPackaged"), None, KEY_READ, &mut np) == ERROR_SUCCESS {
                found = scan(np, true);
                let _ = RegCloseKey(np);
            }
        }
        let _ = RegCloseKey(cap);
        found
    }
}

unsafe fn scan(parent: HKEY, non_packaged: bool) -> Option<String> {
    unsafe {
        let mut i = 0u32;
        loop {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(parent, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) != ERROR_SUCCESS {
                return None;
            }
            i += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            if sub == "NonPackaged" {
                continue;
            }
            let start = read_qword(parent, &name, w!("LastUsedTimeStart"));
            let stop = read_qword(parent, &name, w!("LastUsedTimeStop"));
            if start.unwrap_or(0) != 0 && stop == Some(0) {
                return Some(app_name(&sub, non_packaged));
            }
        }
    }
}

unsafe fn read_qword(parent: HKEY, sub: &[u16], value: PCWSTR) -> Option<u64> {
    let mut data = 0u64;
    let mut size = 8u32;
    let r = unsafe { RegGetValueW(parent, PCWSTR(sub.as_ptr()), value, RRF_RT_REG_QWORD, None, Some(&mut data as *mut _ as _), Some(&mut size)) };
    (r == ERROR_SUCCESS).then_some(data)
}

/// "C:#Program Files#Discord#app-1.0#Discord.exe" -> "Discord",
/// "Microsoft.WindowsCamera_8wekyb3d8bbwe" -> "Camera".
fn app_name(key: &str, non_packaged: bool) -> String {
    let base = if non_packaged {
        key.rsplit('#').next().unwrap_or(key).trim_end_matches(".exe").trim_end_matches(".EXE").to_string()
    } else {
        let family = key.split('_').next().unwrap_or(key);
        family.rsplit('.').next().unwrap_or(family).to_string()
    };
    let known = [("windowscamera", "Camera"), ("msteams", "Teams"), ("ms-teams", "Teams"), ("zoom", "Zoom"), ("discord", "Discord"), ("chrome", "Chrome"), ("msedge", "Edge"), ("firefox", "Firefox"), ("obs64", "OBS"), ("slack", "Slack"), ("whatsapp", "WhatsApp")];
    let lower = base.to_ascii_lowercase();
    for (k, v) in known {
        if lower.contains(k) {
            return v.to_string();
        }
    }
    base
}

/// Calls `cb(focus_on)` once at start and whenever Windows Focus changes.
pub fn start_focus(cb: impl Fn(bool) + Send + Sync + 'static) {
    let _ = std::thread::Builder::new()
        .name("islo-focus".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            use windows::UI::Shell::FocusSessionManager;
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }
            if !FocusSessionManager::IsSupported().unwrap_or(false) {
                return;
            }
            let Ok(mgr) = FocusSessionManager::GetDefault() else { return };
            let cb = std::sync::Arc::new(cb);
            cb(mgr.IsFocusActive().unwrap_or(false));
            let cb2 = cb.clone();
            let handler = windows::Foundation::TypedEventHandler::new(move |m: windows::core::Ref<'_, FocusSessionManager>, _| {
                if let Some(m) = m.as_ref() {
                    cb2(m.IsFocusActive().unwrap_or(false));
                }
                Ok(())
            });
            if mgr.IsFocusActiveChanged(&handler).is_err() {
                return;
            }
            // Keep the manager (and its subscription) alive; events arrive on the thread pool.
            loop {
                std::thread::park();
            }
        });
}
