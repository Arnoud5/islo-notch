# Islo

A macOS-style notch / Dynamic Island for Windows, written in Rust + Slint and
built from the Islo design system. A black island at the screen edge shows
what's happening (music, lyrics, timers, calls, volume) and springs open when
you reach for it.

## Run it

Download `Islo.exe` from the latest release, put it in a folder of its own that
you can write to (Documents, Desktop or `%LOCALAPPDATA%`, not `C:\Program Files`)
and double-click it. That one file is the whole app. A tray icon appears:
left-click opens Settings, right-click gives Settings / Hide / Quit.
Settings are saved in `islo.ini` next to the exe, so nothing is written to C:.

| Do this | What happens |
|---|---|
| Rest the pointer on the island (300 ms, adjustable) | It springs open. Hover the side tab's glyph on left/right edges to open that feature |
| Move away | It closes after a short pause, faster than it opened |
| Drag the island | It lifts off and follows you; let go and it snaps to the nearest edge of that display |
| Scroll on the island | Volume up/down |
| Win + Alt + I | Open / close from any app (Esc closes) |
| Right-click the island | Settings |
| Drag files onto the island | Shelf: keep files, drag them back out to any app |

### What the island shows

- **Media:** album art, title and a waveform. When opened you get "Artist · App",
  a progress bar and prev / play / next controls.
- **Synced lyrics** from LRCLIB (free, no account), for any player: titles like
  "Artist - Song (Official Video)" from browsers and video sites are cleaned up
  before the lookup. At rest it shows 1–10 lines (Settings → Modules). Hover to
  see the whole song scrolling with the music. A lyrics mark in the pill and a
  chip in the player show whether the song has lyrics (synced, plain, none,
  instrumental, offline).
- **HUDs:** volume and brightness pills (brightness only on built-in laptop screens).
- **Battery:** the pill widens once and shows "Charging" when you plug in, and a
  red "Battery low" warning at 20 / 10 / 5 %. The charging panel shows the time
  left until full.
- **Timers:** start one from the ring icon in the opened island. An orange ring
  and the digits show in the pill.
- **Mic / camera in use:** a steady orange or green dot with the app's name.
- **Focus:** an indigo moon while Windows Focus is on.
- **File shelf:** a dashed drop zone, file tiles with sizes, "Kept · 1 file".
- **Mochi the pet:** naps in the island and is awake only while you look at it.
  Now and then it peeks out or walks all the way around the island (also while
  music plays). Pick its look, size, what it does and how often in
  Settings → Modules → Pet. Feed it, and send it to visit a friend's PC nearby
  over Bluetooth (no pairing, no account).
- **Notifications and calls** (after the one-time setup below). New
  notifications show briefly, then wait as a blue count. Calls arriving as
  notifications (Teams, WhatsApp, Phone Link, Zoom) show in green.

### Updates

Islo updates itself from GitHub Releases. Settings → About → **Updates** shows
whether a new version is out, with **Check now** and **Install and restart**.
With "Check automatically" on (the default) Islo looks once shortly after it
starts and then once a day. When a new version is found, Windows shows a
notification and the tray menu gets an "Update to …" item. Nothing installs until
you click Install.

Installing downloads the new `Islo.exe`, swaps it in place of the running one and
restarts. `islo.ini`, start with Windows and the notifications setup all stay as
they are, because the path doesn't change. Islo's folder has to be writable
(Documents, Desktop or `%LOCALAPPDATA%` are fine; `C:\Program Files` isn't).

Versions before 0.3.8 have no updater: install 0.3.8 by hand once, and later
versions arrive by themselves.

### Turning on notifications (one time)

Windows only lets apps with a *package identity* read notifications. In Islo
Settings → Modules → Notifications, click **Set up**:

1. If Developer Mode is off, Windows asks for permission to turn it on (it's a
   machine-wide setting, so it needs an administrator once). Choose **Yes**.
2. Islo writes its package files (`package\` and `Assets\`, baked into Islo.exe)
   next to Islo.exe and registers them as a *sparse package*. Nothing is
   installed; Windows just gives Islo.exe an identity.
3. Islo restarts with Notifications on. Allow access when Windows asks.

The exe has to be named `Islo.exe`. If you move the Islo folder, click **Set up**
again. To undo, click **Remove** next to the Notifications toggle: Islo
unregisters itself and restarts without notifications. Developer Mode stays on
(turn it off in Windows Settings → System → For developers if you like), and the
`package` and `Assets` folders next to Islo.exe can be deleted. Without Islo,
run this in PowerShell instead:
```powershell
Get-AppxPackage -Name Islo | Remove-AppxPackage
```
To skip the Developer Mode step, the package would need a real code-signing
certificate or the Microsoft Store (options in `TODO.md`).

## Why it's light

| | What Islo does | Effect |
|---|---|---|
| Rendering | A tiny custom Win32 backend for Slint (`src/platform.rs`) draws with the CPU into a bitmap, shown by a layered window with per-pixel alpha | No GPU driver, browser engine or WebView in the process |
| Partial updates | Only the changed rectangle is sent to Windows | The waveform and lyric changes stay cheap |
| Idle | The event loop sleeps in `MsgWaitForMultipleObjectsEx` | 0 % CPU while nothing changes |
| Animating | Frames are paced with `DwmFlush` | In sync with the display, and only while something moves |
| System events | Media, volume, brightness, battery, full screen, mic/camera (registry change events), Focus, Bluetooth and lyrics timing are all push or event-driven | One timer per lyric line, not a polling loop. The only fallback is a 2 s notification check if Windows refuses the change event for desktop apps |
| Ambient loops | The waveform and the pet's bob run only while visible | They stop when hidden, in full screen, with reduced motion, or on battery saver |
| Settings window | Built when opened, freed when closed | No memory used while closed |

The Settings footer (About → Memory) shows live memory use. Check it there or in
Task Manager's "Memory" column.

## Project layout

```
ui/island.slint       the island: compact states, HUD, side tab, panels (design tokens in global T)
ui/settings.slint     settings app: sidebar pages, toggles, sliders, "Saved" flashes
ui/icons.slint        vector icons as SVG path data
src/main.rs           startup: single instance, DPI awareness, OLE, platform
src/platform.rs       Win32 backend for Slint (windows, input, keyboard, clipboard, event loop)
src/app.rs            controller: views per display, state machine, springs, drag & dock, wiring
src/spring.rs         damped spring physics (interruptible, keeps velocity)
src/media.rs          Now Playing via GlobalSystemMediaTransportControls
src/lyrics.rs         LRCLIB lookup + LRC parser
src/http.rs           HTTPS GET over WinHTTP, tiny JSON string reader
src/update.rs         update check + download + self-swap (GitHub Releases)
src/audio.rs          volume notifications + set volume
src/brightness.rs     WMI brightness events
src/live.rs           mic / camera in use, Windows Focus
src/notify.rs         notifications (UserNotificationListener) and calls
src/package.rs        Settings > Notifications > Set up: writes out package/ and registers it
src/pet.rs            Mochi + Bluetooth LE beacons for visits
src/system.rs         battery, full screen, tray, autostart, theme, monitors, hotkeys, flyout
src/shelf.rs          drag & drop in/out, file icons
src/config.rs         islo.ini load/save
src/snapshot.rs       (Linux/macOS only) renders UI states to PNG for review
package/              sparse package manifest + logos (notifications), baked into Islo.exe by build.rs
res/                  app icon + exe manifest, pre-compiled (no resource compiler needed)
```

## Build it yourself (everything installed on D:)

1. Tell Rust to live on D:. In PowerShell:
   ```powershell
   setx RUSTUP_HOME D:\dev\rustup
   setx CARGO_HOME  D:\dev\cargo
   ```
   Close and reopen PowerShell.
2. Download `rustup-init.exe` from https://rustup.rs and run:
   ```powershell
   .\rustup-init.exe --default-host x86_64-pc-windows-gnu --default-toolchain stable -y
   ```
   The GNU toolchain avoids the multi-GB Visual Studio Build Tools, which install
   mostly to C:.
3. Add a full MinGW-w64. Rust's GNU toolchain ships only part of it, and the
   `windows` crate needs `dlltool` plus its assembler. Download the WinLibs zip
   from https://winlibs.com (GCC, **UCRT** runtime, **Win64**, *without LLVM*,
   Zip archive), extract it so you have `D:\dev\mingw64\bin\gcc.exe`, and put
   that folder on your PATH:
   ```powershell
   [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path','User') + ';D:\dev\mingw64\bin', 'User')
   ```
   Close and reopen PowerShell. `dlltool --version` should print a version.
4. Build and run:
   ```powershell
   cd D:\Werks\projects\Islo
   cargo run --release
   ```
   The result is `target\release\islo.exe`. Tip: `cargo check` is the fastest way
   to see compiler errors while editing.

## Developing (rebuild on save)

- **Rust and UI changes:** run `.\dev.ps1`. It rebuilds when you save a file in
  `src\` or `ui\` and restarts Islo (about 15 s per save). A failed build keeps
  the running Islo up. Ctrl+C stops the loop and quits Islo. The dev Islo runs
  from `target\dev\` with its own `islo.ini`, and it replaces any Islo that is
  already running.
- **UI only, instantly:** open a `.slint` file in VS Code with the Slint
  extension and click "Show Preview" above a component. It redraws as you type.
  Set properties such as `compact-kind` or `lyric-state` in its side panel to see
  each state. Rust-driven behaviour (springs, media, Mochi's walks) only runs in
  the real app.
- Dev builds are unoptimized and use more CPU. Check performance with
  `cargo run --release`.
- `islo.exe --quit` asks a running Islo to quit cleanly.

## Publishing an update

**Automatic (GitHub Actions):** bump `version` in `Cargo.toml` (for example
`0.3.8` → `0.3.9`), commit and push, then push a matching tag:
```powershell
git tag v0.3.9
git push origin v0.3.9
```
`.github/workflows/release.yml` builds Islo on GitHub, creates the release with
`Islo.exe`, and uses the commit messages since the last tag as release notes
(edit them on GitHub afterwards if you like; the first lines show in Settings).
A tag that doesn't match the `Cargo.toml` version fails without publishing. If
you already drafted the release on GitHub for that tag, its notes stay and
`Islo.exe` is attached to it (publish the draft afterwards). To try the build
without releasing: **Actions → Release → Run workflow**.

To redo a tag (for example after a failed run), move it to the newest commit:
```powershell
git tag -f v0.3.9
git push -f origin v0.3.9
```

**By hand:**

1. Bump `version` in `Cargo.toml` (for example `0.3.8` → `0.3.9`).
2. Build:
   ```powershell
   powershell -ExecutionPolicy Bypass -File release.ps1
   ```
   This makes `dist\Islo.exe`, the whole app: new users download it and the
   updater fetches it.
3. On GitHub (repository set in `src/update.rs`, `REPO`, and it must be public):
   **Releases → Draft a new release**, tag `v<version>` (for example `v0.3.9`),
   attach `Islo.exe`, write what changed (the first lines show in Settings),
   and publish.

Every Islo 0.3.8 or newer sees it within a day. Keep the asset name exactly
`Islo.exe`, and always tag with a higher version than the last release.

## Known limits

- **Native flyout:** hiding the Windows volume/brightness flyout works on
  Windows 10 and early Windows 11. Newer Windows 11 builds draw it where apps
  can't hide it, so it appears alongside Islo's HUD.
- **Screen recording:** there's no red indicator, because Windows has no public
  signal for "screen is being recorded".
- **Focus:** shows "On" rather than "Until 17:00", because Windows doesn't expose
  the end time.
- **Calls:** detected from notifications. Islo can't answer or decline;
  clicking the call opens the app.
- **Pet visits:** need Bluetooth that can broadcast on both PCs. Visits over the
  internet would need a relay server and aren't included.
- **Cursor:** stays an arrow, because Slint doesn't expose cursor shapes to
  custom backends yet.
