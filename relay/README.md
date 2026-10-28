# Islo game relay

A tiny Cloudflare Worker that passes tic-tac-toe moves between two islands.
It runs on Cloudflare's **free plan** (100,000 requests a day; each move or
wait for a move is one request, so a game costs a few dozen). When the day's
allowance runs out, Cloudflare refuses requests until it resets (00:00 UTC,
07:00 in Jakarta) and Islo shows a friendly note instead of the online tab.

Nothing about players is stored: a room is a 4-letter code, two random tokens
and the board, and it deletes itself 10 minutes after it goes quiet.

## Deploy it (once, about 5 minutes)

You need a free Cloudflare account and Node.js (install Node on D:, it doesn't
have to go on C:).

```powershell
cd relay
npx wrangler login        # opens the browser to sign in to Cloudflare
npx wrangler deploy       # prints the address, e.g. https://islo-relay.<you>.workers.dev
```

Then tell the release builds where it is: on GitHub, open the repo's
**Settings → Secrets and variables → Actions → Variables**, add a variable
named `ISLO_RELAY` with the host name only, for example:

```
islo-relay.yourname.workers.dev
```

The next release is built with online play switched on. Builds without the
variable (or local builds) simply show "Online play isn't set up in this build".

For a local build with online play: `$env:ISLO_RELAY="islo-relay.yourname.workers.dev"; cargo build --release`.

## Watch it

`npx wrangler tail` shows requests live. The Cloudflare dashboard shows daily
usage under Workers & Pages → islo-relay → Metrics.
