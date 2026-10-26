//! Master volume: push notifications from Windows (no polling) plus
//! set/mute commands. Follows the default output device when it changes
//! (e.g. plugging in headphones).

use std::sync::mpsc::{Sender, channel};
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::{PCWSTR, implement};

pub enum Cmd {
    /// Change volume by this many percentage points.
    Step(f32),
    Reacquire,
}

/// (volume 0..1, muted, is_initial_value)
pub type Callback = dyn Fn(f32, bool, bool) + Send + Sync;

// `#[implement]` generates the COM plumbing so Windows can call our struct.
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback(std::sync::Arc<Callback>);

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            (self.0)(d.fMasterVolume, d.bMuted.as_bool(), false);
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceWatcher(Sender<Cmd>);

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.0.send(Cmd::Reacquire);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &windows::Win32::Foundation::PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

pub fn start(cb: impl Fn(f32, bool, bool) + Send + Sync + 'static) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    let tx2 = tx.clone();
    let cb: std::sync::Arc<Callback> = std::sync::Arc::new(cb);
    std::thread::Builder::new()
        .name("islo-audio".into())
        .stack_size(256 * 1024)
        .spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return };
            let watcher: IMMNotificationClient = DeviceWatcher(tx2).into();
            let _ = enumerator.RegisterEndpointNotificationCallback(&watcher);
            let callback: IAudioEndpointVolumeCallback = VolumeCallback(cb.clone()).into();

            let acquire = || -> Option<IAudioEndpointVolume> {
                let dev = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).ok()?;
                vol.RegisterControlChangeNotify(&callback).ok()?;
                let v = vol.GetMasterVolumeLevelScalar().unwrap_or(0.0);
                let m = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                cb(v, m, true);
                Some(vol)
            };
            let mut endpoint = acquire();

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    Cmd::Step(delta) => {
                        if let Some(vol) = &endpoint {
                            let cur = vol.GetMasterVolumeLevelScalar().unwrap_or(0.0);
                            let next = ((cur * 100.0 + delta).round() / 100.0).clamp(0.0, 1.0);
                            let _ = vol.SetMasterVolumeLevelScalar(next, std::ptr::null());
                            if next > 0.0 && vol.GetMute().map(|b| b.as_bool()).unwrap_or(false) {
                                let _ = vol.SetMute(false, std::ptr::null());
                            }
                        }
                    }
                    Cmd::Reacquire => {
                        if let Some(old) = endpoint.take() {
                            let _ = old.UnregisterControlChangeNotify(&callback);
                        }
                        endpoint = acquire();
                    }
                }
            }
        })
        .expect("spawn audio thread");
    tx
}
