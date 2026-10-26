//! Notifications in the island, read with Windows' UserNotificationListener.
//!
//! Windows only lets apps with a *package identity* read notifications, so
//! this works once Islo is registered as a sparse package
//! (`install-notifications.ps1`, see the README). Without identity the module
//! reports itself unavailable and Islo runs exactly as before.
//!
//! Changes are pushed by the `NotificationChanged` event where Windows allows
//! it for desktop apps; otherwise the thread checks every 2 seconds (cheap:
//! one call that returns the current list), and only while the module is on.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::Management::*;
use windows::UI::Notifications::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: u32,
    pub app: String,
    pub aumid: String,
    pub title: String,
    pub body: String,
    /// 100 ns ticks since 1601 (same clock as FILETIME)
    pub time: i64,
}

impl Note {
    /// Incoming calls arrive as notifications from Teams, WhatsApp, Phone Link, Zoom...
    pub fn is_call(&self) -> bool {
        let t = format!("{} {}", self.title, self.body).to_lowercase();
        t.contains("incoming call") || t.contains("incoming voice call") || t.contains("incoming video call") || t.contains("is calling")
            || t.contains("panggilan masuk") // Indonesian Windows
    }
}

pub enum Event {
    /// No package identity, or the listener isn't available on this Windows.
    Unavailable,
    /// The user hasn't allowed Islo to read notifications (yet).
    NeedsAccess,
    Denied,
    List(Vec<Note>),
}

pub enum Cmd {
    Stop,
}

/// True when Islo runs with a package identity (registered sparse package).
pub fn has_identity() -> bool {
    use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut len = 0u32;
    let r = unsafe { GetCurrentPackageFullName(&mut len, None) };
    r != APPMODEL_ERROR_NO_PACKAGE
}

/// Ask Windows to show the "allow Islo to access your notifications" prompt.
/// Completion is reported through `done(allowed)` on a thread-pool thread.
pub fn request_access(done: impl Fn(bool) + Send + 'static) {
    let Ok(l) = UserNotificationListener::Current() else { return done(false) };
    let Ok(op) = l.RequestAccessAsync() else { return done(false) };
    let _ = op.SetCompleted(&windows_future::AsyncOperationCompletedHandler::new(move |op: windows::core::Ref<'_, windows_future::IAsyncOperation<UserNotificationListenerAccessStatus>>, _| {
        let ok = op.as_ref().and_then(|o| o.GetResults().ok()) == Some(UserNotificationListenerAccessStatus::Allowed);
        done(ok);
        Ok(())
    }));
}

pub fn start(cb: impl Fn(Event) + Send + Sync + 'static) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    let tx2 = tx.clone();
    let _ = std::thread::Builder::new()
        .name("islo-notify".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }
            let cb: std::sync::Arc<dyn Fn(Event) + Send + Sync> = std::sync::Arc::new(cb);
            if !has_identity() {
                cb(Event::Unavailable);
                return;
            }
            let _ = run(rx, tx2, cb.clone()).map_err(|_| cb(Event::Unavailable));
        });
    tx
}

fn read(l: &UserNotificationListener) -> windows::core::Result<Vec<Note>> {
    let list = l.GetNotificationsAsync(NotificationKinds::Toast)?.join()?;
    let generic = KnownNotificationBindings::ToastGeneric()?;
    let mut out = Vec::new();
    for n in list {
        let id = n.Id().unwrap_or(0);
        let (app, aumid) = n
            .AppInfo()
            .map(|a| {
                let name = a.DisplayInfo().and_then(|d| d.DisplayName()).map(|s| s.to_string()).unwrap_or_default();
                let aumid = a.AppUserModelId().map(|s| s.to_string()).unwrap_or_default();
                (name, aumid)
            })
            .unwrap_or_default();
        let texts: Vec<String> = n
            .Notification()
            .and_then(|x| x.Visual())
            .and_then(|v| v.GetBinding(&generic))
            .and_then(|b| b.GetTextElements())
            .map(|els| els.into_iter().filter_map(|t| t.Text().ok().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        // one tidy line each: no line breaks, no emoji boxes
        let title = crate::system::display_text(&texts.first().cloned().unwrap_or_default(), 120);
        let body = crate::system::display_text(&texts.iter().skip(1).cloned().collect::<Vec<_>>().join(" "), 200);
        let app = crate::system::display_text(&app, 40);
        let time = n.CreationTime().map(|t| t.UniversalTime).unwrap_or(0);
        out.push(Note { id, app, aumid, title, body, time });
    }
    // newest first
    out.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(out)
}

fn run(rx: Receiver<Cmd>, tx: Sender<Cmd>, cb: std::sync::Arc<dyn Fn(Event) + Send + Sync>) -> windows::core::Result<()> {
    let l = UserNotificationListener::Current()?;
    match l.GetAccessStatus()? {
        UserNotificationListenerAccessStatus::Allowed => {}
        UserNotificationListenerAccessStatus::Denied => {
            cb(Event::Denied);
            return Ok(());
        }
        _ => {
            cb(Event::NeedsAccess);
            return Ok(());
        }
    }
    let mut last = read(&l).unwrap_or_default();
    cb(Event::List(last.clone()));

    // Prefer the push event. Desktop apps sometimes aren't allowed to subscribe;
    // then fall back to a light 2 s check.
    let (wake_tx, wake_rx) = channel::<()>();
    let pushed = l
        .NotificationChanged(&TypedEventHandler::new(move |_, _| {
            let _ = wake_tx.send(());
            Ok(())
        }))
        .is_ok();
    let _ = tx; // keep the command channel alive
    loop {
        if pushed {
            // sleep until Windows says something changed (or a Stop arrives)
            match wake_rx.recv_timeout(Duration::from_secs(60)) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            if matches!(rx.try_recv(), Ok(Cmd::Stop)) {
                return Ok(());
            }
        } else {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        if let Ok(now) = read(&l) {
            if now != last {
                cb(Event::List(now.clone()));
                last = now;
            }
        }
    }
}
