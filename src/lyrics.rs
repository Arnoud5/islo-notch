//! Synced lyrics from LRCLIB (https://lrclib.net), a free open lyrics database.
//!
//! A worker thread waits on a channel for "look up this song" requests,
//! fetches over HTTPS (see http.rs), parses the LRC
//! text and sends the timed lines back. Results are cached per song.
//!
//! Works with any player: browsers and video sites report titles like
//! "Artist - Song (Official Video)" under a channel name, so the lookup tries
//! a few cleaned-up guesses (and LRCLIB's free-text search) before giving up.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
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

/// What the lookup found for one song.
#[derive(Clone, Default)]
pub struct Found {
    /// Synced: timed lines. Plain: every line at 0 s.
    pub lines: Lines,
    pub synced: bool,
    /// LRCLIB marks the track as instrumental (no words to show).
    pub instrumental: bool,
    /// LRCLIB couldn't be reached at all (offline): not cached, tried again next time.
    pub offline: bool,
}

pub fn start(cb: impl Fn(String, Found) + Send + 'static) -> Sender<Request> {
    let (tx, rx) = channel::<Request>();
    let _ = std::thread::Builder::new()
        .name("islo-lyrics".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let mut cache: HashMap<String, Found> = HashMap::new();
            let mut pending: Option<Request> = None;
            loop {
                let mut req = match pending.take() {
                    Some(r) => r,
                    None => match rx.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    },
                };
                // only the newest request matters when tracks are skipped quickly
                while let Ok(newer) = rx.try_recv() {
                    req = newer;
                }
                let found = match cache.get(&req.key) {
                    Some(f) => f.clone(),
                    None => {
                        let Some(f) = lookup(&req, &rx, &mut pending) else {
                            continue; // a newer song came in mid-lookup
                        };
                        if !f.offline {
                            if cache.len() > 50 {
                                cache.clear();
                            }
                            cache.insert(req.key.clone(), f.clone());
                        }
                        f
                    }
                };
                cb(req.key, found);
            }
        });
    tx
}

/// Try each guess in turn; None when a newer request arrived (it's left in `pending`).
fn lookup(r: &Request, rx: &Receiver<Request>, pending: &mut Option<Request>) -> Option<Found> {
    let mut stale = || {
        if let Ok(n) = rx.try_recv() {
            *pending = Some(n);
        }
        pending.is_some()
    };
    let mut reached = false;
    let mut plain: Option<String> = None;
    let mut instrumental = false;
    let done = |synced: String| Some(Found { lines: parse_lrc(&synced), synced: true, ..Default::default() });

    for (artist, title) in guesses(&r.artist, &r.title) {
        if stale() {
            return None;
        }
        // exact match first (album only when it came from the player for this artist)
        let mut path = format!("/api/get?artist_name={}&track_name={}", enc(&artist), enc(&title));
        if !r.album.is_empty() && artist == clean_artist(&r.artist) {
            path += &format!("&album_name={}", enc(&r.album));
        }
        if r.duration > 1.0 {
            path += &format!("&duration={}", r.duration.round() as i64);
        }
        match http_get(&path) {
            Some(body) => {
                reached = true;
                if let Some(t) = json_string(&body, "syncedLyrics").filter(|t| real_synced(t)) {
                    return done(t);
                }
                plain = plain.or_else(|| json_string(&body, "plainLyrics").filter(|t| real_plain(t)));
                instrumental |= body.contains("\"instrumental\":true");
            }
            None => {}
        }
        if stale() {
            return None;
        }
        // looser matching on title/artist
        let path = format!("/api/search?track_name={}&artist_name={}", enc(&title), enc(&artist));
        if let Some(body) = http_get(&path) {
            reached = true;
            match best(&body, r.duration) {
                Pick::Synced(t) => return done(t),
                Pick::Plain(t) => plain = plain.or(Some(t)),
                Pick::Instrumental => instrumental = true,
                Pick::Nothing => {}
            }
        }
    }
    // last resort: free-text search over the whole title
    if !stale() {
        let q = format!("{} {}", clean_artist(&r.artist), clean_title(&r.title));
        if let Some(body) = http_get(&format!("/api/search?q={}", enc(q.trim()))) {
            reached = true;
            // without a duration to check against, a free-text hit could be another song
            if r.duration > 1.0 {
                match best(&body, r.duration) {
                    Pick::Synced(t) => return done(t),
                    Pick::Plain(t) => plain = plain.or(Some(t)),
                    Pick::Instrumental => instrumental = true,
                    Pick::Nothing => {}
                }
            }
        }
    } else {
        return None;
    }
    if let Some(p) = plain {
        let lines = plain_lines(&p);
        if !lines.is_empty() {
            return Some(Found { lines, synced: false, ..Default::default() });
        }
    }
    Some(Found { instrumental, offline: !reached, ..Default::default() })
}

/// Plain lyrics: every line at 0 s, without blank lines around them.
fn plain_lines(text: &str) -> Lines {
    let mut lines: Lines = text
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .take(MAX_LINES)
        .map(|l| (0.0, crate::system::display_text(l, 200)))
        .collect();
    // display_text may blank a line that only held control characters
    let lead = lines.iter().take_while(|l| l.1.is_empty()).count();
    lines.drain(..lead);
    while lines.last().is_some_and(|l| l.1.is_empty()) {
        lines.pop();
    }
    lines
}

enum Pick {
    Synced(String),
    Plain(String),
    Instrumental,
    Nothing,
}

/// Choose from a search result array: synced lyrics whose length matches the
/// song (within 5 s when we know it), else plain lyrics, else "instrumental".
fn best(body: &str, duration: f64) -> Pick {
    let fits = |o: &str| duration <= 1.0 || json_number(o, "duration").is_none_or(|d| (d - duration).abs() <= 5.0);
    let items = json_objects(body);
    if let Some(t) = items.iter().filter(|o| fits(o)).find_map(|o| json_string(o, "syncedLyrics").filter(|t| real_synced(t))) {
        return Pick::Synced(t);
    }
    if let Some(t) = items.iter().filter(|o| fits(o)).find_map(|o| json_string(o, "plainLyrics").filter(|t| real_plain(t))) {
        return Pick::Plain(t);
    }
    if items.iter().any(|o| fits(o) && o.contains("\"instrumental\":true")) {
        return Pick::Instrumental;
    }
    Pick::Nothing
}

/// LRCLIB is open to edits and has junk entries (one line like "probe"):
/// lyrics need a few real lines to count.
fn real_synced(t: &str) -> bool {
    parse_lrc(t).iter().filter(|l| l.1 != "· · ·").count() >= 4
}

fn real_plain(t: &str) -> bool {
    t.lines().filter(|l| !l.trim().is_empty()).count() >= 4
}

/// The top-level `{...}` objects of a JSON array (strings may hold braces).
fn json_objects(json: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut in_str, mut esc) = (0usize, 0usize, false, false);
    for (i, c) in json.char_indices() {
        if in_str {
            match c {
                _ if esc => esc = false,
                '\\' => esc = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    out.push(&json[start..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// `"key": 123.4` in a JSON object.
fn json_number(json: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\"");
    let pos = json.find(&needle)? + needle.len();
    let rest = json[pos..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// (artist, title) pairs worth asking LRCLIB about, best guess first.
fn guesses(artist: &str, title: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |a: String, t: String| {
        if !a.is_empty() && !t.is_empty() && !out.iter().any(|(oa, ot)| oa.eq_ignore_ascii_case(&a) && ot.eq_ignore_ascii_case(&t)) {
            out.push((a, t));
        }
    };
    let (a, t) = (clean_artist(artist), clean_title(title));
    // "Artist - Song" in the title (video sites, browsers, many players)
    // ("Song - Remastered 2011" is a version tag, not an artist)
    let split = [" - ", " – ", " — ", " ~ "].iter().find_map(|sep| title.split_once(sep)).filter(|(_, r)| !version_tag(r));
    let channel = a.is_empty() || artist.ends_with(" - Topic") || artist.contains("VEVO") || {
        let l = artist.to_ascii_lowercase();
        l.contains("records") || l.contains("music") || l.contains("official") || l.contains("channel")
    };
    if let Some((left, right)) = split {
        let (sa, st) = (clean_artist(left), clean_title(right));
        if channel {
            add(sa.clone(), st.clone());
        }
        add(a.clone(), t.clone());
        add(a.clone(), st.clone());
        add(sa, st);
    } else {
        add(a.clone(), t.clone());
    }
    // "A, B & C" -> "A"
    let first = first_artist(&a);
    if first != a {
        add(first, t);
    }
    out.truncate(4);
    out
}

/// Strip what players and video sites add to a song title.
pub fn clean_title(s: &str) -> String {
    let mut s = s.to_string();
    for sep in [" | ", " ｜ ", " // "] {
        if let Some(i) = s.find(sep) {
            s.truncate(i);
        }
    }
    // "Song - Remastered 2011", "Song - Radio Edit"
    if let Some((l, r)) = s.split_once(" - ") {
        if version_tag(r) {
            s = l.to_string();
        }
    }
    let noise = [
        "official", "video", "audio", "lyric", "visuali", "m/v", "mv", "hd", "hq", "4k", "remaster", "explicit", "clean",
        "color coded", "feat", "ft.", "ft ", "prod", "with ", "full song", "music video",
    ];
    s = strip_brackets(&s, |inner| {
        let l = inner.to_ascii_lowercase();
        noise.iter().any(|n| l.contains(n))
    });
    // trailing " feat. X" outside brackets
    let lower = s.to_ascii_lowercase();
    for tag in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(i) = lower.find(tag) {
            s.truncate(i);
            break;
        }
    }
    tidy(&s)
}

/// The part after " - " names a version of the song, not its artist or title.
fn version_tag(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ["remaster", "version", "edit", "live", "mono", "stereo", "single", "from ", "demo", "bonus", "deluxe", "anniversary"]
        .iter()
        .any(|t| l.contains(t))
}

fn clean_artist(s: &str) -> String {
    let mut s = s.trim().to_string();
    for suffix in [" - Topic", "VEVO", " Official", "Official"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest.to_string();
        }
    }
    s = strip_brackets(&s, |_| true);
    tidy(&s)
}

fn first_artist(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let cut = [",", " & ", " x ", " feat", " ft.", ";", " / ", " and "]
        .iter()
        .filter_map(|sep| lower.find(sep))
        .min()
        .unwrap_or(s.len());
    tidy(&s[..cut])
}

/// Remove (...) [...] 【...】 groups whose inside matches `drop`.
fn strip_brackets(s: &str, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        let open = rest.char_indices().find(|(_, c)| matches!(c, '(' | '[' | '【' | '「'));
        let Some((i, o)) = open else {
            out += rest;
            break;
        };
        let close = match o {
            '(' => ')',
            '[' => ']',
            '【' => '】',
            _ => '」',
        };
        let inner_start = i + o.len_utf8();
        let Some(j) = rest[inner_start..].find(close) else {
            out += rest;
            break;
        };
        let inner = &rest[inner_start..inner_start + j];
        out += &rest[..i];
        if !drop(inner) {
            out += &rest[i..inner_start + j + close.len_utf8()];
        }
        rest = &rest[inner_start + j + close.len_utf8()..];
    }
    out
}

fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c| c == '"' || c == '\'' || c == '-' || c == ' ').to_string()
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

/// Most lines kept from one song's lyrics. Real songs have well under 200;
/// LRCLIB is open to edits, so a huge record mustn't eat memory or the UI.
const MAX_LINES: usize = 2000;
/// Most timestamps one LRC line may carry (a chorus repeated a few times).
const MAX_STAMPS: usize = 16;

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
                    // f64 parsing takes "nan" and "inf" too: only real times count
                    let t = m * 60.0 + s;
                    if t.is_finite() && stamps.len() < MAX_STAMPS {
                        stamps.push(t);
                    }
                }
            }
            rest = r[end + 1..].trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        let text = crate::system::display_text(rest, 200);
        for t in stamps.into_iter().take(MAX_LINES - lines.len()) {
            lines.push((t, text.clone()));
        }
        if lines.len() == MAX_LINES {
            break;
        }
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    // instrumental gaps: plain dots (in every font, so no fallback font is needed)
    for l in &mut lines {
        if l.1.trim().is_empty() {
            l.1 = "· · ·".into();
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_video_titles() {
        assert_eq!(clean_title("Never Gonna Give You Up (Official Music Video)"), "Never Gonna Give You Up");
        assert_eq!(clean_title("Song [Lyrics] | Some Channel"), "Song");
        assert_eq!(clean_title("Song (Remix)"), "Song (Remix)");
        assert_eq!(clean_title("Song feat. Someone"), "Song");
        assert_eq!(clean_title("Song - Remastered 2011"), "Song");
        assert_eq!(clean_artist("ArtistVEVO"), "Artist");
        assert_eq!(clean_artist("Artist - Topic"), "Artist");
        assert_eq!(first_artist("A, B & C"), "A");
    }

    #[test]
    fn splits_artist_from_title() {
        let g = guesses("Some Channel Music", "Rick Astley - Never Gonna Give You Up (Official Video)");
        assert_eq!(g[0], ("Rick Astley".to_string(), "Never Gonna Give You Up".to_string()));
        let g = guesses("Adele", "Hello");
        assert_eq!(g[0], ("Adele".to_string(), "Hello".to_string()));
    }

    #[test]
    #[ignore] // needs the internet: cargo test lyrics -- --ignored
    fn finds_video_site_title_online() {
        let (_tx, rx) = channel::<Request>();
        let req = Request {
            key: "k".into(),
            artist: "RickAstleyVEVO".into(),
            title: "Rick Astley - Never Gonna Give You Up (Official Music Video)".into(),
            album: String::new(),
            duration: 213.0,
        };
        let f = lookup(&req, &rx, &mut None).expect("not stale");
        assert!(f.synced && f.lines.len() > 10, "found {} lines, synced {}", f.lines.len(), f.synced);
    }

    #[test]
    fn skips_stamps_that_are_not_times() {
        // NaN in a sort comparator could panic, which aborts a release build
        for n in 21..=64 {
            let lrc: String = (0..n)
                .map(|i| if i % 3 == 0 { format!("[nan:00] x{i}\n") } else { format!("[inf:{i}]  [00:{:02}.00] l{i}\n", n - i) })
                .collect();
            let lines = parse_lrc(&lrc);
            assert!(lines.iter().all(|l| l.0.is_finite()));
            assert!(lines.windows(2).all(|w| w[0].0 <= w[1].0));
            assert_eq!(lines.len(), n - (n + 2) / 3);
        }
    }

    #[test]
    fn caps_huge_lyrics() {
        let lrc = "[0:0]".repeat(400_000) + " line";
        assert_eq!(parse_lrc(&lrc).len(), MAX_STAMPS);
        let lrc = "[0:0] line\n".repeat(400_000);
        assert_eq!(parse_lrc(&lrc).len(), MAX_LINES);

        let start = std::time::Instant::now();
        let plain = "\n".repeat(1_000_000) + "a\nb\n\n\nc\nd\n\n";
        assert_eq!(plain_lines(&plain).iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["a", "b", "", "", "c", "d"]);
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "took {:?}", start.elapsed());
        assert_eq!(plain_lines(&"x\n".repeat(10_000)).len(), MAX_LINES);
    }

    #[test]
    fn picks_matching_duration() {
        let lrc = r"[00:01.00] a\n[00:02.00] b\n[00:03.00] c\n[00:04.00] d";
        let body = format!(
            r#"[{{"duration":212,"syncedLyrics":"[00:00.00]probe"}},{{"duration":100,"syncedLyrics":"[00:00.50] wrong\n{lrc}"}},{{"duration":212.5,"syncedLyrics":"[00:00.50] right {{x}}\n{lrc}"}}]"#
        );
        match best(&body, 212.0) {
            Pick::Synced(t) => assert!(t.contains("right")),
            _ => panic!("no pick"),
        }
    }
}
