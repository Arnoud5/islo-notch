//! Tic-tac-toe: play Mochi at three levels, or a friend through a tiny relay.
//!
//! Board cells: 0 empty, 1 you (x), 2 the other side (o: Mochi, or the friend).
//!
//! Online play goes through Islo's relay (relay/ in the repo: a Cloudflare
//! Worker on the free plan). One player hosts and gets a 4-letter room code,
//! the other pastes it. The relay only passes moves between the two islands;
//! nothing is kept after the room closes. While a game is open a worker thread
//! long-polls the room (one request at a time, held up to 20 s by the relay),
//! so nothing runs when you aren't playing online.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// The relay's host name, set at build time (ISLO_RELAY=islo-relay.<you>.workers.dev).
/// Empty: this build has no online play.
pub const RELAY: &str = match option_env!("ISLO_RELAY") {
    Some(h) => h,
    None => "",
};

pub type Board = [u8; 9];

const LINES: [[usize; 3]; 8] = [[0, 1, 2], [3, 4, 5], [6, 7, 8], [0, 3, 6], [1, 4, 7], [2, 5, 8], [0, 4, 8], [2, 4, 6]];

/// Winner (1 or 2) with the winning line, a draw (0, no line), or None while playing.
pub fn outcome(b: &Board) -> Option<(u8, Option<[usize; 3]>)> {
    for l in LINES {
        let v = b[l[0]];
        if v != 0 && v == b[l[1]] && v == b[l[2]] {
            return Some((v, Some(l)));
        }
    }
    b.iter().all(|&c| c != 0).then_some((0, None))
}

/// Best score for `me` to move: +1 win, 0 draw, -1 loss (perfect play both sides).
fn negamax(b: &mut Board, me: u8) -> (i8, Option<usize>) {
    if let Some((w, _)) = outcome(b) {
        return (if w == 0 { 0 } else if w == me { 1 } else { -1 }, None);
    }
    let other = 3 - me;
    let mut best = (-2, None);
    for i in 0..9 {
        if b[i] == 0 {
            b[i] = me;
            let s = -negamax(b, other).0;
            b[i] = 0;
            if s > best.0 {
                best = (s, Some(i));
            }
        }
    }
    best
}

/// A cell that finishes a line for `who`, if any.
fn finishing(b: &Board, who: u8) -> Option<usize> {
    (0..9).find(|&i| {
        if b[i] != 0 {
            return false;
        }
        let mut t = *b;
        t[i] = who;
        matches!(outcome(&t), Some((w, _)) if w == who)
    })
}

/// Mochi's move (it plays 2). `level`: 0 easy, 1 medium, 2 hard. `roll` gives randomness.
pub fn pick(b: &Board, level: i32, mut roll: impl FnMut(u32) -> u32) -> Option<usize> {
    let free: Vec<usize> = (0..9).filter(|&i| b[i] == 0).collect();
    if free.is_empty() {
        return None;
    }
    let any = |r: &mut dyn FnMut(u32) -> u32| free[r(free.len() as u32) as usize];
    match level {
        // Hard: perfect play. The best you can get is a draw.
        2 => negamax(&mut b.clone(), 2).1,
        // Medium: wins when it can, blocks you, likes the centre, else guesses.
        1 => finishing(b, 2)
            .or_else(|| finishing(b, 1))
            .or_else(|| (b[4] == 0 && roll(10) < 6).then_some(4))
            .or_else(|| Some(any(&mut roll))),
        // Easy: mostly random; now and then it spots its own win.
        _ => {
            if roll(10) < 3 {
                if let Some(i) = finishing(b, 2) {
                    return Some(i);
                }
            }
            Some(any(&mut roll))
        }
    }
}

// ---------------- online ----------------

pub enum Cmd {
    Host,
    Join(String),
    Move(usize),
    Again,
    Leave,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub code: String,
    pub seat: u8,      // 1 hosts (and plays x), 2 joined (plays o)
    pub board: Board,  // cells by seat: 1 host, 2 guest
    pub turn: u8,      // whose move: 1 or 2
    pub players: u8,   // 1 waiting for a friend, 2 playing
    pub gone: bool,    // the other player left
    pub version: i64,
}

pub enum Event {
    Hosting(String),
    State(Room),
    /// The relay's free daily limit ran out.
    Limit,
    /// Couldn't play online: a message for the island.
    Problem(String),
    Closed,
}

/// Room codes: letters and digits that can't be confused (no 0/O, 1/I/L).
pub fn clean_code(s: &str) -> Option<String> {
    const OK: &str = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let c: String = s.trim().to_uppercase().chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    (c.chars().count() == 4 && c.chars().all(|ch| OK.contains(ch))).then_some(c)
}

fn parse_room(json: &str, code: &str, seat: u8) -> Option<Room> {
    let cells = crate::http::json_string(json, "board")?;
    let mut board = [0u8; 9];
    for (i, ch) in cells.chars().take(9).enumerate() {
        board[i] = match ch {
            '1' | 'X' => 1,
            '2' | 'O' => 2,
            _ => 0,
        };
    }
    Some(Room {
        code: code.to_string(),
        seat,
        board,
        turn: crate::http::json_int(json, "turn").unwrap_or(1) as u8,
        players: crate::http::json_int(json, "players").unwrap_or(1) as u8,
        gone: crate::http::json_bool(json, "gone").unwrap_or(false),
        version: crate::http::json_int(json, "v").unwrap_or(0),
    })
}

/// What went wrong, from the relay's answer (None: it worked).
fn problem(status: u32, body: &str) -> Option<Event> {
    match status {
        200 => None,
        // Cloudflare refuses requests once the free plan's daily limit is used up
        // (429, or error 1027), and the relay says "limit" when its storage is.
        429 => Some(Event::Limit),
        _ if body.contains("1027") || body.contains("\"limit\"") => Some(Event::Limit),
        404 => Some(Event::Problem("No game with that code. Check it with your friend, it works for 10 minutes.".into())),
        409 => Some(Event::Problem("That game already has two players.".into())),
        _ => Some(Event::Problem("The game server didn't answer. Try again in a moment.".into())),
    }
}

/// Start the online worker. It sleeps on its channel until you host or join.
pub fn start(on_event: impl Fn(Event) + Send + Sync + 'static) -> Sender<Cmd> {
    let (tx, rx) = channel();
    let on: std::sync::Arc<dyn Fn(Event) + Send + Sync> = std::sync::Arc::new(on_event);
    let _ = std::thread::Builder::new().name("islo-game".into()).stack_size(256 * 1024).spawn(move || run(rx, on));
    tx
}

fn call(method: &str, path: &str, body: Option<&str>, wait: i32) -> Option<(u32, String)> {
    crate::http::send(method, RELAY, path, body, wait, 16 * 1024)
}

/// The room we're in. `epoch` changes whenever we leave, so a poll that was
/// still waiting for the relay drops its answer instead of reviving the room.
#[derive(Clone)]
struct Seat {
    code: String,
    token: String,
    seat: u8,
    epoch: u64,
}

type Shared = std::sync::Arc<std::sync::Mutex<Option<Seat>>>;

fn current(shared: &Shared, epoch: u64) -> bool {
    shared.lock().map(|g| g.as_ref().is_some_and(|s| s.epoch == epoch)).unwrap_or(false)
}

/// Long poll: the relay answers when something changes, or after ~20 s.
/// One request at a time, only while a room is open.
fn poll(shared: Shared, me: Seat, on: std::sync::Arc<dyn Fn(Event) + Send + Sync>) {
    let mut since = -1i64;
    while current(&shared, me.epoch) {
        let path = format!("/rooms/{}/state?token={}&since={since}", me.code, me.token);
        let answer = call("GET", &path, None, 30_000);
        if !current(&shared, me.epoch) {
            return;
        }
        match answer {
            Some((200, body)) => {
                if let Some(r) = parse_room(&body, &me.code, me.seat) {
                    since = r.version;
                    let gone = r.gone;
                    on(Event::State(r));
                    if gone {
                        if let Ok(mut g) = shared.lock() {
                            *g = None;
                        }
                        return;
                    }
                }
            }
            Some((404, _)) => {
                if let Ok(mut g) = shared.lock() {
                    *g = None;
                }
                on(Event::Problem("The game ended: the room closed.".into()));
                return;
            }
            Some((s, body)) => {
                if let Some(e) = problem(s, &body) {
                    if matches!(e, Event::Limit) {
                        if let Ok(mut g) = shared.lock() {
                            *g = None;
                        }
                        on(e);
                        return;
                    }
                    on(e);
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            None => std::thread::sleep(Duration::from_secs(3)), // offline for a moment: try again
        }
    }
}

fn run(rx: Receiver<Cmd>, on: std::sync::Arc<dyn Fn(Event) + Send + Sync>) {
    let shared: Shared = Default::default();
    let mut epoch = 0u64;
    let seat_now = || shared.lock().ok().and_then(|g| g.clone());
    let enter = |code: String, token: String, seat: u8, epoch: u64| {
        let me = Seat { code, token, seat, epoch };
        if let Ok(mut g) = shared.lock() {
            *g = Some(me.clone());
        }
        let (shared, on) = (shared.clone(), on.clone());
        let _ = std::thread::Builder::new().name("islo-game-poll".into()).stack_size(128 * 1024).spawn(move || poll(shared, me, on));
    };
    while let Ok(cmd) = rx.recv() {
        if RELAY.is_empty() {
            on(Event::Problem("Online play isn't set up in this build of Islo.".into()));
            continue;
        }
        match cmd {
            Cmd::Host | Cmd::Join(_) => {
                // leaving any room we were in first
                if let Some(old) = seat_now() {
                    let _ = call("POST", &format!("/rooms/{}/leave", old.code), Some(&format!("{{\"token\":\"{}\"}}", old.token)), 8_000);
                }
                epoch += 1;
                let (path, seat, join_code) = match &cmd {
                    Cmd::Join(code) => (format!("/rooms/{code}/join"), 2u8, Some(code.clone())),
                    _ => ("/rooms".to_string(), 1u8, None),
                };
                match call("POST", &path, Some("{}"), 15_000) {
                    Some((200, body)) => {
                        let token = crate::http::json_string(&body, "token").unwrap_or_default();
                        let code = join_code.or_else(|| crate::http::json_string(&body, "code")).unwrap_or_default();
                        if code.is_empty() || token.is_empty() {
                            on(Event::Problem("The game server answered oddly. Try again later.".into()));
                            continue;
                        }
                        if seat == 1 {
                            on(Event::Hosting(code.clone()));
                        } else if let Some(r) = parse_room(&body, &code, 2) {
                            on(Event::State(r));
                        }
                        enter(code, token, seat, epoch);
                    }
                    Some((s, body)) => on(problem(s, &body).unwrap_or(Event::Closed)),
                    None => on(Event::Problem("Couldn't reach the game server. Are you online?".into())),
                }
            }
            Cmd::Move(cell) => {
                let Some(me) = seat_now() else { continue };
                let body = format!("{{\"token\":\"{}\",\"cell\":{cell}}}", me.token);
                match call("POST", &format!("/rooms/{}/move", me.code), Some(&body), 15_000) {
                    Some((200, b)) => {
                        if let Some(r) = parse_room(&b, &me.code, me.seat) {
                            on(Event::State(r));
                        }
                    }
                    Some((409, _)) => {} // not our turn (any more): the poll catches up
                    Some((s, b)) => {
                        if let Some(e) = problem(s, &b) {
                            on(e);
                        }
                    }
                    None => on(Event::Problem("Couldn't reach the game server. Are you online?".into())),
                }
            }
            Cmd::Again => {
                if let Some(me) = seat_now() {
                    let _ = call("POST", &format!("/rooms/{}/again", me.code), Some(&format!("{{\"token\":\"{}\"}}", me.token)), 15_000);
                }
            }
            Cmd::Leave => {
                let me = shared.lock().ok().and_then(|mut g| g.take());
                if let Some(me) = me {
                    let _ = call("POST", &format!("/rooms/{}/leave", me.code), Some(&format!("{{\"token\":\"{}\"}}", me.token)), 8_000);
                }
                on(Event::Closed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_never_loses() {
        // play every possible line for x against perfect o: o never loses
        fn walk(b: &mut Board) {
            if outcome(b).is_some() {
                assert_ne!(outcome(b).unwrap().0, 1, "x beat hard: {b:?}");
                return;
            }
            for i in 0..9 {
                if b[i] == 0 {
                    b[i] = 1;
                    if outcome(b).is_none() {
                        let m = pick(b, 2, |_| 0).unwrap();
                        b[m] = 2;
                        walk(b);
                        b[m] = 0;
                    } else {
                        walk(b);
                    }
                    b[i] = 0;
                }
            }
        }
        walk(&mut [0; 9]);
    }

    #[test]
    fn medium_blocks_and_wins() {
        let b = [1, 1, 0, 0, 2, 0, 0, 0, 0];
        assert_eq!(pick(&b, 1, |_| 9), Some(2)); // block
        let b = [1, 1, 0, 2, 2, 0, 1, 0, 0];
        assert_eq!(pick(&b, 1, |_| 9), Some(5)); // win first
    }

    #[test]
    fn codes() {
        assert_eq!(clean_code(" k7qf "), Some("K7QF".into()));
        assert_eq!(clean_code("K0QF"), None);
        assert_eq!(clean_code("K7Q"), None);
    }
}
