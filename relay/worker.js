// Islo game relay: rooms of two players, moves passed between their islands.
//
// Runs on Cloudflare Workers' free plan with one Durable Object per room.
// Nothing about the players is stored: a room holds a code, two random
// tokens and the board, and deletes itself 10 minutes after it goes quiet.
//
//   POST /rooms                   create a room   -> { code, token, seat: 1, ...state }
//   POST /rooms/CODE/join         join it          -> { token, seat: 2, ...state }
//   POST /rooms/CODE/move         { token, cell }  -> state
//   POST /rooms/CODE/again        { token }        -> state (rematch, the other side starts)
//   POST /rooms/CODE/leave        { token }
//   GET  /rooms/CODE/state?token=T&since=V        -> state, held up to 20 s until v > V
//
// state = { v, board: "000120000", turn, players, gone }

const ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"; // no 0/O, 1/I/L
const QUIET_MS = 10 * 60 * 1000;
const HOLD_MS = 20 * 1000;
const LINES = [[0, 1, 2], [3, 4, 5], [6, 7, 8], [0, 3, 6], [1, 4, 7], [2, 5, 8], [0, 4, 8], [2, 4, 6]];

const json = (body, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", "cache-control": "no-store" } });

function newCode() {
  const b = new Uint8Array(4);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => ALPHABET[x % ALPHABET.length]).join("");
}

function outcome(board) {
  for (const [a, b, c] of LINES) if (board[a] && board[a] === board[b] && board[a] === board[c]) return board[a];
  return board.every((x) => x) ? 3 : 0; // 3 = draw
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const parts = url.pathname.split("/").filter(Boolean); // ["rooms", CODE?, action?]
    if (parts[0] !== "rooms") return json({ error: "not-found" }, 404);
    try {
      if (parts.length === 1 && req.method === "POST") {
        // a fresh code; retry the rare clash with a live room
        for (let i = 0; i < 6; i++) {
          const code = newCode();
          const res = await env.ROOMS.get(env.ROOMS.idFromName(code)).fetch(`https://room/${code}/create`, { method: "POST" });
          if (res.status !== 409) return res;
        }
        return json({ error: "busy" }, 503);
      }
      const code = (parts[1] || "").toUpperCase();
      if (!/^[A-Z2-9]{4}$/.test(code) || parts.length !== 3) return json({ error: "not-found" }, 404);
      const target = `https://room/${code}/${parts[2]}${url.search}`;
      return await env.ROOMS.get(env.ROOMS.idFromName(code)).fetch(target, { method: req.method, body: req.method === "POST" ? await req.text() : undefined });
    } catch (e) {
      // the free plan's daily Durable Object allowance used up shows up as an error here
      const limit = /limit|exceeded|quota/i.test(String(e && e.message));
      return json({ error: limit ? "limit" : "relay" }, limit ? 503 : 502);
    }
  },
};

export class Room {
  constructor(state) {
    this.state = state;
    this.room = null;
    this.waiters = [];
  }

  async load() {
    if (this.room === null) this.room = (await this.state.storage.get("room")) || false;
    return this.room;
  }

  async save() {
    await this.state.storage.put("room", this.room);
    await this.state.storage.setAlarm(Date.now() + QUIET_MS);
    const waiting = this.waiters;
    this.waiters = [];
    for (const wake of waiting) wake();
  }

  async alarm() {
    // quiet for 10 minutes: the room is gone
    this.room = false;
    await this.state.storage.deleteAll();
    const waiting = this.waiters;
    this.waiters = [];
    for (const wake of waiting) wake();
  }

  view() {
    const r = this.room;
    return { v: r.v, board: r.board.join(""), turn: r.turn, players: r.tokens[1] ? 2 : 1, gone: r.gone };
  }

  seatOf(token) {
    const r = this.room;
    if (!token) return 0;
    if (token === r.tokens[0]) return 1;
    if (token === r.tokens[1]) return 2;
    return 0;
  }

  async fetch(req) {
    const url = new URL(req.url);
    const [, action] = url.pathname.split("/").filter(Boolean);
    const body = req.method === "POST" ? await req.json().catch(() => ({})) : {};
    const r = await this.load();

    if (action === "create") {
      if (r) return json({ error: "taken" }, 409);
      const token = crypto.randomUUID();
      this.room = { v: 1, board: [0, 0, 0, 0, 0, 0, 0, 0, 0], turn: 1, starter: 1, tokens: [token, null], gone: false };
      await this.save();
      const code = url.pathname.split("/").filter(Boolean)[0];
      return json({ code, token, seat: 1, ...this.view() });
    }
    if (!r) return json({ error: "no-room" }, 404);

    if (action === "join") {
      if (r.tokens[1] || r.gone) return json({ error: "full" }, 409);
      const token = crypto.randomUUID();
      r.tokens[1] = token;
      r.v++;
      await this.save();
      return json({ token, seat: 2, ...this.view() });
    }

    const token = body.token || url.searchParams.get("token");
    const seat = this.seatOf(token);
    if (!seat) return json({ error: "not-yours" }, 403);

    if (action === "state") {
      const since = Number(url.searchParams.get("since") || -1);
      if (r.v <= since && !r.gone) {
        // hold the request until something changes (or 20 s pass)
        await new Promise((resolve) => {
          const t = setTimeout(resolve, HOLD_MS);
          this.waiters.push(() => { clearTimeout(t); resolve(); });
        });
      }
      if (!this.room) return json({ error: "no-room" }, 404);
      return json(this.view());
    }
    if (action === "move") {
      const cell = Number(body.cell);
      if (r.gone || !r.tokens[1] || outcome(r.board) || r.turn !== seat || !(cell >= 0 && cell < 9) || r.board[cell]) {
        return json({ error: "not-now", ...this.view() }, 409);
      }
      r.board[cell] = seat;
      r.turn = 3 - seat;
      r.v++;
      await this.save();
      return json(this.view());
    }
    if (action === "again") {
      if (outcome(r.board)) {
        r.board = [0, 0, 0, 0, 0, 0, 0, 0, 0];
        r.starter = 3 - r.starter;
        r.turn = r.starter;
        r.v++;
        await this.save();
      }
      return json(this.view());
    }
    if (action === "leave") {
      r.gone = true;
      r.v++;
      await this.save();
      // tidy up soon after both have seen it
      await this.state.storage.setAlarm(Date.now() + 60 * 1000);
      return json(this.view());
    }
    return json({ error: "not-found" }, 404);
  }
}
