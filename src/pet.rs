//! Mochi, the island pet, and nearby visits over Bluetooth LE.
//!
//! Visits need no server and no pairing: every Islo advertises a tiny
//! Bluetooth LE beacon and listens for others.
//!
//! Beacon (manufacturer data, company id 0xFFFF = "for testing / no company"):
//!   [0x15 magic][id: 3 bytes][kind][target id: 3 bytes][pet name len][pet name][owner name]
//!   kind 0 = here, 1 = "my pet visits <target>", 2 = "I'm hosting <target>'s pet",
//!   3 = "sending <target>'s pet home". Names are cut to fit the 24-byte budget.
//!   Any device in range can read a beacon, so the owner name is empty unless
//!   the user turns on "Show my name", and the id is new every time Islo starts.
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

/// Mochi's preset looks: (name, body, eyes) as 0xRRGGBB. Any body and eye
/// color can be picked in Settings; these are the starting points.
/// Must match `skins` in settings.slint (same order).
pub const SKINS: [(&str, u32, u32); 6] = [
    ("Classic", 0x000000, 0xd3b63b),
    ("Snow", 0xf5f5f5, 0x000000),
    ("Rose", 0xfc90bc, 0x000000),
    ("Mint", 0x6fd087, 0x000000),
    ("Sky", 0x71bfff, 0x000000),
    ("Ember", 0xf99f55, 0x000000),
];

/// The preset with exactly these colors, or -1 for a custom look.
pub fn preset_of(body: u32, eyes: u32) -> i32 {
    SKINS.iter().position(|s| s.1 == body && s.2 == eyes).map_or(-1, |i| i as i32)
}

/// WCAG contrast ratio of two 0xRRGGBB colors (1 .. 21).
pub fn contrast(a: u32, b: u32) -> f32 {
    let lum = |c: u32| {
        let ch = |v: u32| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch((c >> 16) & 255) + 0.7152 * ch((c >> 8) & 255) + 0.0722 * ch(c & 255)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The colors Mochi is drawn with: (body, eyes, body on the black island, eye
/// outline). The eye color you pick is always kept; when it would vanish on the
/// body (under 3:1), the eyes get a thin black or white outline instead. A body
/// too dark for the island is drawn a step lighter there.
pub fn colors(body: u32, eyes: u32) -> (u32, u32, u32, Option<u32>) {
    let island = if contrast(body, 0x000000) < 1.25 { 0x161616 } else { body };
    let ring = if contrast(body, eyes) >= 3.0 && contrast(island, eyes) >= 3.0 {
        None
    } else if contrast(eyes, 0x000000) >= contrast(eyes, 0xffffff) {
        Some(0x000000)
    } else {
        Some(0xffffff)
    };
    (body, eyes, island, ring)
}

/// Mochi's moods: the `face` numbers of the Mochi component (ui/icons.slint).
pub mod face {
    pub const NEUTRAL: i32 = 0;
    pub const HAPPY: i32 = 1;
    pub const SAD: i32 = 2;
    pub const ANGRY: i32 = 3;
    pub const SURPRISED: i32 = 4;
    pub const SCARED: i32 = 5;
    pub const DISGUSTED: i32 = 6;
    pub const LOVE: i32 = 7;
    pub const EXCITED: i32 = 8;
    pub const CONFUSED: i32 = 10;
    pub const SLEEPY: i32 = 11; // also chewing
    pub const BORED: i32 = 12;
    pub const SQUEEZED: i32 = 13; // >< : chomping, shaking off a yank
}

/// Size of the peeking / wandering Mochi (logical px) for small, medium, large.
pub const SIZES: [f32; 3] = [26.0, 34.0, 44.0];

/// Wait between appearances (ms) for rarely, sometimes, often.
pub const OFTEN: [(u64, u64); 3] = [(45_000, 90_000), (15_000, 35_000), (5_000, 12_000)];

/// A random beacon id for this run of Islo (never saved, so a PC can't be
/// recognized from one day to the next). 0 is left out: it means "no target".
pub fn session_id() -> [u8; 3] {
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from the OS's random number generator
    let r = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let id = ((r >> 40) as u32 & 0xff_ffff).max(1);
    [(id >> 16) as u8, (id >> 8) as u8, id as u8]
}

/// How a nearby owner is shown: their first name if they share it.
pub fn shown_owner(owner: &str) -> String {
    if owner.is_empty() { "A friend".into() } else { owner.into() }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eyes_stay_visible_on_any_body() {
        // the brand look is kept as is, and drawn a step lighter on the black island
        assert_eq!(colors(0x000000, 0xd3b63b), (0x000000, 0xd3b63b, 0x161616, None));
        // the picked eye color always stays; an outline makes it readable
        assert_eq!(colors(0xf5f5f5, 0xffffff), (0xf5f5f5, 0xffffff, 0xf5f5f5, Some(0x000000)));
        assert_eq!(colors(0x101010, 0x000000).1, 0x000000);
        assert_eq!(colors(0x101010, 0x000000).3, Some(0xffffff));
        // a colored body shows as itself in the island
        assert_eq!(colors(0x71bfff, 0x000000).2, 0x71bfff);
        assert!((contrast(0x000000, 0xffffff) - 21.0).abs() < 0.01);
    }

    #[test]
    fn beacon_carries_the_owner_only_when_given() {
        for kind in 0..4 {
            let v = encode([0x12, 0x34, 0x56], "Mochi", "", kind, [0xaa, 0xbb, 0xcc]);
            assert_eq!(v.len(), 9 + 5, "no owner bytes after the pet name");
            let p = decode(&v).unwrap();
            assert_eq!((p.id, p.kind, p.pet.as_str(), p.owner.as_str()), ([0x12, 0x34, 0x56], kind, "Mochi", ""));
        }
        let v = encode([1, 2, 3], "Mochi", "Testfirst", 0, [0; 3]);
        assert_eq!(decode(&v).unwrap().owner, "Testfirst");
        assert_eq!(shown_owner(""), "A friend");
    }

    #[test]
    fn beacon_id_is_new_each_session() {
        let ids: Vec<[u8; 3]> = (0..8).map(|_| session_id()).collect();
        assert!(ids.iter().all(|id| *id != [0; 3]));
        assert!(ids.windows(2).any(|w| w[0] != w[1]), "ids repeat: {ids:?}");
    }

    #[test]
    fn presets_are_found_by_their_colors() {
        assert_eq!(preset_of(0x000000, 0xd3b63b), 0);
        assert_eq!(preset_of(0xf99f55, 0x000000), 5);
        assert_eq!(preset_of(0x123456, 0x000000), -1);
    }
}
