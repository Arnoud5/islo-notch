//! Synced lyrics from LRCLIB (https://lrclib.net), a free open lyrics database.
//!
//! A worker thread waits on a channel for "look up this song" requests,
//! fetches over HTTPS (see http.rs), parses the LRC
//! text and sends the timed lines back. Results are cached per song.

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use crate::http::json_string;

#[derive(Clone)]
pub struct Request {
    pub key: String,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration: f64,
}

/// Timed lines: (seconds from start, text). Empty = no synced lyrics found.
pub type Lines = Vec<(f64, String)>;

pub fn start(cb: impl Fn(String, Lines) + Send + 'static) -> Sender<Request> {
    let (tx, rx) = channel::<Request>();
    let _ = std::thread::Builder::new()
        .name("islo-lyrics".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let mut cache: HashMap<String, Lines> = HashMap::new();
            while let Ok(mut req) = rx.recv() {
                // only the newest request matters when tracks are skipped quickly
                while let Ok(newer) = rx.try_recv() {
                    req = newer;
                }
                let lines = match cache.get(&req.key) {
                    Some(l) => l.clone(),
                    None => {
                        let l = fetch(&req).map(|t| parse_lrc(&t)).unwrap_or_default();
                        if cache.len() > 50 {
                            cache.clear();
                        }
                        cache.insert(req.key.clone(), l.clone());
                        l
                    }
                };
                cb(req.key, lines);
            }
        });
    tx
}

fn fetch(r: &Request) -> Option<String> {
    let mut path = format!("/api/get?artist_name={}&track_name={}", enc(&r.artist), enc(&r.title));
    if !r.album.is_empty() {
        path += &format!("&album_name={}", enc(&r.album));
    }
    if r.duration > 1.0 {
        path += &format!("&duration={}", r.duration.round() as i64);
    }
    if let Some(body) = http_get(&path) {
        if let Some(t) = json_string(&body, "syncedLyrics") {
            return Some(t);
        }
    }
    // fall back to search (looser matching on title/artist)
    let body = http_get(&format!("/api/search?track_name={}&artist_name={}", enc(&r.title), enc(&r.artist)))?;
    json_string(&body, "syncedLyrics")
}

fn http_get(path: &str) -> Option<String> {
    crate::http::get_text("lrclib.net", path, 2_000_000)
}

/// Percent-encode a query value (UTF-8).
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out += &format!("%{b:02X}"),
        }
    }
    out
}

/// Parse LRC: "[01:23.45] text" (a line may carry several timestamps).
pub fn parse_lrc(text: &str) -> Lines {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            if let Some((m, s)) = tag.split_once(':') {
                if let (Ok(m), Ok(s)) = (m.trim().parse::<f64>(), s.trim().parse::<f64>()) {
                    stamps.push(m * 60.0 + s);
                }
            }
            rest = r[end + 1..].trim_start();
        }
        let text = crate::system::display_text(rest, 200);
        for t in stamps {
            lines.push((t, text.clone()));
        }
    }
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // instrumental gaps: plain dots (in every font, so no fallback font is needed)
    for l in &mut lines {
        if l.1.trim().is_empty() {
            l.1 = "· · ·".into();
        }
    }
    lines
}
