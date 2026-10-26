//! Mochi, the island pet, and nearby visits over Bluetooth LE.
//!
//! Visits need no server and no pairing: every Islo advertises a tiny
//! Bluetooth LE beacon and listens for others.
//!
//! Beacon (manufacturer data, company id 0xFFFF = "for testing / no company"):
//!   [0x15 magic][id: 3 bytes][kind][target id: 3 bytes][pet name len][pet name][owner name]
//!   kind 0 = here, 1 = "my pet visits <target>", 2 = "I'm hosting <target>'s pet",
//!   3 = "sending <target>'s pet home". Names are cut to fit the 24-byte budget.
//!
//! The radio thread only reports *changes* of the peer table to the UI, so a
//! room full of beacons doesn't wake the UI thread a few times per second.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use windows::Devices::Bluetooth::Advertisement::*;
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter};

const COMPANY: u16 = 0xFFFF;
const MAGIC: u8 = 0x15;

#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub id: [u8; 3],
    pub pet: String,
    pub owner: String,
    pub kind: u8,
    pub target: [u8; 3],
}

pub enum Cmd {
    /// What we advertise: pet + owner name, kind and target.
    Beacon { pet: String, owner: String, kind: u8, target: [u8; 3] },
    Stop,
}

pub enum Event {
    Peers(Vec<Peer>),
    /// Bluetooth missing, off, or the adapter can't advertise.
    Unavailable,
}

pub fn start(id: [u8; 3], cb: impl Fn(Event) + Send + Sync + 'static) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    let _ = std::thread::Builder::new()
        .name("islo-ble".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }
            if run(id, rx, std::sync::Arc::new(cb)).is_err() {
                // reported by run() through the callback where possible
            }
        });
    tx
}

fn buffer(bytes: &[u8]) -> windows::core::Result<windows::Storage::Streams::IBuffer> {
    let w = DataWriter::new()?;
    w.WriteBytes(bytes)?;
    w.DetachBuffer()
}

fn encode(id: [u8; 3], pet: &str, owner: &str, kind: u8, target: [u8; 3]) -> Vec<u8> {
    let cut = |s: &str, max: usize| {
        let mut out = Vec::new();
        for ch in s.chars() {
            let mut b = [0u8; 4];
            let e = ch.encode_utf8(&mut b).as_bytes();
            if out.len() + e.len() > max {
                break;
            }
            out.extend_from_slice(e);
        }
        out
    };
    let pet = cut(pet, 8);
    let owner = cut(owner, 15 - pet.len());
    let mut v = vec![MAGIC, id[0], id[1], id[2], kind, target[0], target[1], target[2], pet.len() as u8];
    v.extend_from_slice(&pet);
    v.extend_from_slice(&owner);
    v
}

fn decode(d: &[u8]) -> Option<Peer> {
    if d.len() < 9 || d[0] != MAGIC {
        return None;
    }
    let n = d[8] as usize;
    if 9 + n > d.len() {
        return None;
    }
    Some(Peer {
        id: [d[1], d[2], d[3]],
        kind: d[4],
        target: [d[5], d[6], d[7]],
        pet: String::from_utf8_lossy(&d[9..9 + n]).into_owned(),
        owner: String::from_utf8_lossy(&d[9 + n..]).into_owned(),
    })
}

fn run(id: [u8; 3], rx: Receiver<Cmd>, cb: std::sync::Arc<dyn Fn(Event) + Send + Sync>) -> windows::core::Result<()> {
    // ---- listen ----
    let seen: std::sync::Arc<std::sync::Mutex<HashMap<[u8; 3], (Peer, Instant)>>> = Default::default();
    let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = BluetoothLEAdvertisementWatcher::new()?;
    watcher.SetScanningMode(BluetoothLEScanningMode::Passive)?;
    // only wake us for Islo beacons (company id + magic byte prefix)
    let filter = BluetoothLEManufacturerData::Create(COMPANY, &buffer(&[MAGIC])?)?;
    watcher.AdvertisementFilter()?.Advertisement()?.ManufacturerData()?.Append(&filter)?;
    {
        let seen = seen.clone();
        let changed = changed.clone();
        let cb = cb.clone();
        watcher.Received(&TypedEventHandler::new(move |_, args: windows::core::Ref<'_, BluetoothLEAdvertisementReceivedEventArgs>| {
            let Some(args) = args.as_ref() else { return Ok(()) };
            let list = args.Advertisement()?.GetManufacturerDataByCompanyId(COMPANY)?;
            for md in list {
                let buf = md.Data()?;
                let r = DataReader::FromBuffer(&buf)?;
                let mut bytes = vec![0u8; r.UnconsumedBufferLength()? as usize];
                r.ReadBytes(&mut bytes)?;
                let Some(p) = decode(&bytes) else { continue };
                if p.id == id {
                    continue; // ourselves
                }
                let mut s = seen.lock().unwrap();
                let fresh = s.get(&p.id).map(|(old, _)| old != &p).unwrap_or(true);
                s.insert(p.id, (p, Instant::now()));
                if fresh {
                    let peers = s.values().map(|(p, _)| p.clone()).collect();
                    drop(s);
                    cb(Event::Peers(peers));
                    changed.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Ok(())
        }))?;
    }
    if watcher.Start().is_err() {
        cb(Event::Unavailable);
        return Ok(());
    }

    // ---- advertise ----
    let mut publisher: Option<BluetoothLEAdvertisementPublisher> = None;
    loop {
        // Commands, or a 10 s tick to forget peers that walked away.
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Cmd::Beacon { pet, owner, kind, target }) => {
                if let Some(p) = publisher.take() {
                    let _ = p.Stop();
                }
                let p = BluetoothLEAdvertisementPublisher::new()?;
                let md = BluetoothLEManufacturerData::Create(COMPANY, &buffer(&encode(id, &pet, &owner, kind, target))?)?;
                p.Advertisement()?.ManufacturerData()?.Append(&md)?;
                if p.Start().is_err() {
                    cb(Event::Unavailable);
                }
                publisher = Some(p);
            }
            Ok(Cmd::Stop) => {
                if let Some(p) = publisher.take() {
                    let _ = p.Stop();
                }
                let _ = watcher.Stop();
                return Ok(());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let mut s = seen.lock().unwrap();
                let before = s.len();
                s.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(30));
                if s.len() != before {
                    let peers = s.values().map(|(p, _)| p.clone()).collect();
                    drop(s);
                    cb(Event::Peers(peers));
                }
            }
            Err(_) => return Ok(()),
        }
    }
}

/// First name of the signed-in Windows user ("Frumentius Daneswara" -> "Frumentius").
pub fn owner_name() -> String {
    use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    let name = unsafe {
        if GetUserNameExW(NameDisplay, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len) {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            std::env::var("USERNAME").unwrap_or_default()
        }
    };
    name.split_whitespace().next().unwrap_or("Friend").to_string()
}

/// "fed 2 hours ago"
pub fn fed_ago(fed_unix: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if fed_unix == 0 {
        return "not fed yet".into();
    }
    let m = now.saturating_sub(fed_unix) / 60;
    match m {
        0 => "fed just now".into(),
        1..=59 => format!("fed {m} min ago"),
        60..=119 => "fed an hour ago".into(),
        _ if m < 60 * 48 => format!("fed {} hours ago", m / 60),
        _ => format!("fed {} days ago", m / 60 / 24),
    }
}

/// Mood from how long ago the pet was fed.
pub fn mood(fed_unix: u64) -> &'static str {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let h = now.saturating_sub(fed_unix) / 3600;
    if fed_unix == 0 || h >= 12 {
        "Hungry"
    } else if h >= 5 {
        "Peckish"
    } else {
        "Happy"
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
