//! Now-playing info and controls via Windows' GlobalSystemMediaTransportControls
//! (the same source as the Windows media flyout: Spotify, browsers, most players).
//!
//! Runs on its own thread. That thread sleeps on a channel (`rx.recv()`), which
//! costs no CPU; Windows wakes it through event handlers when something changes.
//! Results are sent to the UI thread with a callback.

use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Imaging::*;
use windows::Media::Control::*;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

pub enum Cmd {
    SessionChanged,
    Changed,
    PlayPause,
    Next,
    Prev,
}

#[derive(Clone)]
pub struct Art {
    pub rgba: Vec<u8>, // 96x96 straight-alpha RGBA
    pub accent: [u8; 3],
}

#[derive(Clone)]
pub struct Info {
    pub title: String,
    pub artist: String,
    /// Friendly name of the playing app ("Spotify", "Edge", ...).
    pub app: String,
    pub album: String,
    pub playing: bool,
    /// Bumps whenever the artwork changed, so the UI only re-uploads it then.
    pub art_id: u64,
    pub art: Option<Art>,
    /// (position s, duration s, seconds since that position was measured)
    pub timeline: Option<(f64, f64, f64)>,
}

pub const ART_SIZE: u32 = 96;

pub fn start(on_update: impl Fn(Option<Info>) + Send + 'static) -> Sender<Cmd> {
    let (tx, rx) = channel();
    let tx2 = tx.clone();
    std::thread::Builder::new()
        .name("islo-media".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            if let Err(e) = run(tx2, rx, &on_update) {
                eprintln!("media service stopped: {e}");
                on_update(None);
            }
        })
        .expect("spawn media thread");
    tx
}

struct Subscribed {
    session: GlobalSystemMediaTransportControlsSession,
    tokens: [i64; 3],
}

fn run(tx: Sender<Cmd>, rx: Receiver<Cmd>, on_update: &dyn Fn(Option<Info>)) -> windows::core::Result<()> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
    {
        let t = tx.clone();
        mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Cmd::SessionChanged);
            Ok(())
        }))?;
        let t = tx.clone();
        mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Cmd::SessionChanged);
            Ok(())
        }))?;
    }

    let mut sub: Option<Subscribed> = None;
    let mut art_key = String::new();
    let mut art_id = 0u64;
    let mut art: Option<Art> = None;
    let mut last_sent_none = false;

    let _ = tx.send(Cmd::SessionChanged);
    while let Ok(first) = rx.recv() {
        // Coalesce bursts (players often fire 3-4 events at once).
        let mut cmds = vec![first];
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        let mut refresh = false;
        for cmd in cmds {
            match cmd {
                Cmd::SessionChanged => {
                    if let Some(old) = sub.take() {
                        let _ = old.session.RemoveMediaPropertiesChanged(old.tokens[0]);
                        let _ = old.session.RemovePlaybackInfoChanged(old.tokens[1]);
                        let _ = old.session.RemoveTimelinePropertiesChanged(old.tokens[2]);
                    }
                    if let Ok(session) = mgr.GetCurrentSession() {
                        // Each event has its own argument type, so a small macro
                        // stamps out one handler per event.
                        macro_rules! notify {
                            () => {{
                                let t = tx.clone();
                                TypedEventHandler::new(move |_, _| {
                                    let _ = t.send(Cmd::Changed);
                                    Ok(())
                                })
                            }};
                        }
                        let a = session.MediaPropertiesChanged(&notify!()).unwrap_or(0);
                        let b = session.PlaybackInfoChanged(&notify!()).unwrap_or(0);
                        let c = session.TimelinePropertiesChanged(&notify!()).unwrap_or(0);
                        sub = Some(Subscribed { session, tokens: [a, b, c] });
                    }
                    refresh = true;
                }
                Cmd::Changed => refresh = true,
                Cmd::PlayPause | Cmd::Next | Cmd::Prev => {
                    if let Some(s) = &sub {
                        let r = match cmd {
                            Cmd::PlayPause => s.session.TryTogglePlayPauseAsync(),
                            Cmd::Next => s.session.TrySkipNextAsync(),
                            _ => s.session.TrySkipPreviousAsync(),
                        };
                        if let Ok(op) = r {
                            let _ = op.join();
                        }
                    }
                }
            }
        }
        if !refresh {
            continue;
        }

        let Some(s) = &sub else {
            if !last_sent_none {
                on_update(None);
                last_sent_none = true;
            }
            continue;
        };
        let info = read(&s.session, &mut art_key, &mut art_id, &mut art);
        last_sent_none = info.is_none();
        on_update(info);
    }
    Ok(())
}

fn read(session: &GlobalSystemMediaTransportControlsSession, art_key: &mut String, art_id: &mut u64, art: &mut Option<Art>) -> Option<Info> {
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let mut artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    if artist.is_empty() {
        artist = props.AlbumArtist().map(|s| s.to_string()).unwrap_or_default();
    }
    if title.is_empty() && artist.is_empty() {
        return None;
    }
    let (title, artist) = (crate::system::display_text(&title, 120), crate::system::display_text(&artist, 80));
    let playing = session
        .GetPlaybackInfo()
        .and_then(|p| p.PlaybackStatus())
        .map(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);

    // Only decode artwork when the track changed.
    let key = format!("{title}\u{1}{artist}");
    if *art_key != key {
        *art_key = key;
        *art = decode_art(&props);
        *art_id += 1;
    } else if art.is_none() {
        // Some players publish the thumbnail a moment after the title.
        if let Some(a) = decode_art(&props) {
            *art = Some(a);
            *art_id += 1;
        }
    }

    let timeline = session.GetTimelineProperties().ok().and_then(|t| {
        let pos = t.Position().ok()?.Duration as f64 / 1e7;
        let start = t.StartTime().ok()?.Duration as f64 / 1e7;
        let end = t.EndTime().ok()?.Duration as f64 / 1e7;
        let updated = t.LastUpdatedTime().ok()?.UniversalTime;
        let dur = end - start;
        if dur <= 1.0 {
            return None;
        }
        let age = (now_filetime() - updated) as f64 / 1e7;
        Some((pos - start, dur, age.clamp(0.0, 24.0 * 3600.0)))
    });

    let app = session.SourceAppUserModelId().map(|s| friendly_app(&s.to_string())).unwrap_or_default();
    let album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();
    Some(Info { title, artist, app, album, playing, art_id: *art_id, art: art.clone(), timeline })
}

fn now_filetime() -> i64 {
    // WinRT DateTime and FILETIME share the same epoch (1601) and 100 ns units.
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

fn decode_art(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Art> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;
    let (w, h) = (decoder.PixelWidth().ok()? as f64, decoder.PixelHeight().ok()? as f64);
    if w < 1.0 || h < 1.0 {
        return None;
    }
    // Scale so the short side is ART_SIZE, then center-crop a square
    // (video thumbnails are 16:9).
    let scale = ART_SIZE as f64 / w.min(h);
    let (sw, sh) = ((w * scale).round() as u32, (h * scale).round() as u32);
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(sw).ok()?;
    t.SetScaledHeight(sh).ok()?;
    t.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
    t.SetBounds(BitmapBounds {
        X: sw.saturating_sub(ART_SIZE) / 2,
        Y: sh.saturating_sub(ART_SIZE) / 2,
        Width: ART_SIZE.min(sw),
        Height: ART_SIZE.min(sh),
    })
    .ok()?;
    let data = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Straight,
            &t,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .join()
        .ok()?;
    let bytes = data.DetachPixelData().ok()?;
    let rgba: Vec<u8> = bytes.to_vec();
    if rgba.len() != (ART_SIZE * ART_SIZE * 4) as usize {
        return None;
    }
    Some(Art { accent: accent_of(&rgba), rgba })
}

/// A lively accent color from the artwork: average of the more saturated pixels,
/// then pushed to a readable brightness on black.
fn accent_of(rgba: &[u8]) -> [u8; 3] {
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for p in rgba.chunks_exact(4).step_by(3) {
        let (pr, pg, pb) = (p[0] as i32, p[1] as i32, p[2] as i32);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        if max - min > 40 && max > 60 {
            r += pr as u64;
            g += pg as u64;
            b += pb as u64;
            n += 1;
        }
    }
    if n == 0 {
        return [0x8a, 0xb4, 0xff];
    }
    let (r, g, b) = ((r / n) as f32, (g / n) as f32, (b / n) as f32);
    let max = r.max(g).max(b).max(1.0);
    let k = 230.0 / max;
    [(r * k).min(255.0) as u8, (g * k).min(255.0) as u8, (b * k).min(255.0) as u8]
}

/// "Spotify.exe" -> "Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify" -> "Spotify",
/// "MSEdge" -> "Edge", "Chrome" -> "Chrome".
fn friendly_app(aumid: &str) -> String {
    let mut s = aumid.rsplit('!').next().unwrap_or(aumid);
    s = s.rsplit('\\').next().unwrap_or(s);
    let s = s.trim_end_matches(".exe").trim_end_matches(".EXE");
    let lower = s.to_ascii_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("zunemusic", "Media Player"),
        ("microsoft.media.player", "Media Player"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
        ("youtube", "YouTube Music"),
    ];
    for (k, v) in known {
        if lower.contains(k) {
            return v.to_string();
        }
    }
    let last = s.rsplit('.').next().unwrap_or(s);
    let mut c = last.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
