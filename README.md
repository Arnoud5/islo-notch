# Islo

A macOS-style notch / Dynamic Island for Windows, written in Rust + Slint and
built from the Islo design system. A black island at the screen edge shows
what's happening (music, lyrics, timers, calls, volume) and springs open when
you reach for it.

## Run it

Double-click `Islo.exe` (if you have `Islo-new.exe`, quit Islo from the tray,
delete the old `Islo.exe` and rename the new one). A tray icon appears:
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
- **Synced lyrics** from LRCLIB (free, no account). At rest it shows 1–10 lines
  (Settings → Modules). Hover to see the whole song scrolling with the music.
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
  Feed it, and send it to visit a friend's PC nearby over Bluetooth (no pairing,
  no account).
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

Windows only lets apps with a *package identity* read notifications. Double-click
**`install-notifications.cmd`**:

1. If Developer Mode is off, Windows Settings opens. Switch Developer Mode on
   (this is the only step that needs administrator rights), then run the script again.
2. The script registers `package\AppxManifest.xml` as a *sparse package* for the
   `Islo.exe` in this folder. Nothing is copied or installed; Windows just gives
   Islo.exe an identity. It also renames `Islo-new.exe` to `Islo.exe` and restarts Islo.
3. In Islo Settings → Modules, turn on **Notifications** and allow access when Windows asks.

To undo: `install-notifications.cmd -Uninstall`. If you move the Islo folder, run
the script again. To share Islo with other people without the Developer Mode
step, the package would need a real code-signing certificate or the Microsoft Store.

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
src/pet.rs            Mochi + Bluetooth LE beacons for visits
src/system.rs         battery, full screen, tray, autostart, theme, monitors, hotkeys, flyout
src/shelf.rs          drag & drop in/out, file icons
src/config.rs         islo.ini load/save
src/snapshot.rs       (Linux/macOS only) renders UI states to PNG for review
package/              sparse package manifest + logos (notifications)
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
   The GNU toolchain ships its own linker, so you don't need the multi-GB Visual
   Studio Build Tools, which install mostly to C:.
3. Build and run:
   ```powershell
   cd D:\Werks\projects\Islo
   cargo run --release
   ```
   The result is `target\release\islo.exe`. Tip: `cargo check` is the fastest way
   to see compiler errors while editing.

## Publishing an update

1. Bump `version` in `Cargo.toml` (for example `0.3.8` → `0.3.9`).
2. Build and package:
   ```powershell
   powershell -ExecutionPolicy Bypass -File release.ps1
   ```
   This makes `dist\Islo.exe` (what the updater downloads) and
   `dist\Islo-<version>.zip` (for new users).
3. On GitHub (repository set in `src/update.rs`, `REPO`, and it must be public):
   **Releases → Draft a new release**, tag `v<version>` (for example `v0.3.9`),
   attach **both** files, write what changed (the first lines show in Settings),
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
