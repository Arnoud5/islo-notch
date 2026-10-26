# Islo — project context

## What it is
Islo is a Windows desktop app that recreates the macOS notch / Dynamic Island: a small pill at the top of the screen that stays out of the way and expands into media controls, system HUDs, and quick tools when needed.

## Priorities (in order)
1. Performance: near-zero CPU when idle, low memory, no polling loops, animations synced to display refresh rate, never slows games or full-screen apps
2. Polished UI: clean, minimal, fits Windows 11 (rounded corners, dark/light themes), crisp on any DPI and multi-monitor
3. Playful, interactive motion: spring-based expand/collapse, responsive hover/click/drag feedback, smooth morphing between states; respects "reduce animations"

## Core features
- Media controls (Windows GlobalSystemMediaTransportControls: Spotify, browsers, players)
- System HUDs: volume, brightness, battery
- Drag-and-drop file shelf
- Notifications inside the island (later; requires MSIX packaging for package identity)
- Auto-hide in full-screen apps; never steals focus or blocks clicks underneath
- Customizable size, position, and modules

## Non-goals (for now)
Plugin marketplace, cloud sync, accounts

## Tech stack — decision pending
Option A: Tauri 2 + Vue 3 + TypeScript + Vite + Motion for Vue, with small Rust commands (windows crate) for system APIs. Faster to ship; ~50–100 MB memory.
Option B: Full Rust with Slint UI + windows crate. Lowest resource use (~10–30 MB), more work for animations, steeper learning curve.
Plan: a weekend spike building the pill with a hover-to-expand spring animation in Slint, then decide.

## Performance rules
- Animate only transform/opacity (or the equivalent GPU-friendly properties)
- Pause animation loops when the island is idle or collapsed
- Push events from the system layer to the UI; don't poll
- Keep the window as small as the pill; expand only when open

## About me
I'm a JavaScript/TypeScript developer (Vue 3, Tailwind) and new to Rust. When writing Rust, explain it step by step, point out ownership/borrowing issues, and help me read compiler errors. Prefer small, working increments.