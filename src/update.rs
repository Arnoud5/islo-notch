//! Updates from GitHub Releases.
//!
//! Checking: one HTTPS request to the GitHub API for the latest release, on a
//! worker thread. Its tag (like "v0.3.8") is compared with this build's version.
//!
//! Installing: the release's `Islo.exe` asset is downloaded next to the running
//! exe, then swapped in. Windows won't let a running exe be overwritten or
//! deleted, but it can be *renamed*, so:
//!   1. Islo.exe      -> Islo.exe.old   (the running one, moved aside)
//!   2. Islo.exe.new  -> Islo.exe       (the download takes its place)
//!   3. start the new Islo.exe with --after-update, and quit
//! The new process waits for this one to exit (single instance) and deletes
//! Islo.exe.old. The path stays the same, so islo.ini, start with Windows and
//! the notifications package keep working.

use std::path::PathBuf;

/// The GitHub repository releases come from ("owner/name"). It must be public.
pub const REPO: &str = "frumentius44/Islo";
/// The release asset the updater downloads. Attach a file with exactly this name.
const ASSET: &str = "Islo.exe";
/// Islo.exe is ~10 MB; anything far bigger isn't ours.
const MAX_EXE: usize = 64 * 1024 * 1024;

pub const AFTER_UPDATE_ARG: &str = "--after-update";

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,     // "v0.3.8"
    pub version: String, // "0.3.8"
    pub notes: String,   // first lines of the release description
}

/// "v1.2.3" / "1.2.3" -> (1, 2, 3). Missing parts count as 0.
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    // ignore a pre-release / build suffix: "1.2.3-beta" -> "1.2.3"
    let s = s.split(['-', '+']).next()?;
    let mut parts = s.split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

fn current() -> (u32, u32, u32) {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

/// Check in the background. `cb` runs on the worker thread with:
/// Ok(Some(release)) when a newer version exists, Ok(None) when up to date,
/// Err(message) when GitHub couldn't be reached.
pub fn check(cb: impl FnOnce(Result<Option<Release>, String>) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("islo-update-check".into())
        .stack_size(256 * 1024)
        .spawn(move || cb(latest()));
}

fn latest() -> Result<Option<Release>, String> {
    let body = crate::http::get_text("api.github.com", &format!("/repos/{REPO}/releases/latest"), 1_000_000)
        .ok_or_else(|| "Couldn't reach GitHub. Check your connection and try again.".to_string())?;
    let tag = crate::http::json_string(&body, "tag_name").ok_or_else(|| "No release found.".to_string())?;
    let Some(theirs) = parse_version(&tag) else {
        return Err(format!("Release \"{tag}\" isn't a version number."));
    };
    if theirs <= current() {
        return Ok(None);
    }
    let notes = crate::http::json_string(&body, "body")
        // plain text from the Markdown: drop heading / bullet marks
        .map(|b| b.lines().map(|l| l.trim().trim_start_matches(['#', '-', '*', ' '])).filter(|l| !l.is_empty()).take(3).collect::<Vec<_>>().join(" · "))
        .unwrap_or_default();
    let (a, b, c) = theirs;
    Ok(Some(Release { tag, version: format!("{a}.{b}.{c}"), notes: crate::system::display_text(&notes, 160) }))
}

/// Download `release` and swap it in. `cb` runs on the worker thread with the
/// path of the new exe (ready to start), or a message for the user.
pub fn install(release: Release, cb: impl FnOnce(Result<PathBuf, String>) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("islo-update-install".into())
        .stack_size(256 * 1024)
        .spawn(move || cb(download_and_swap(&release)));
}

fn download_and_swap(release: &Release) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "Couldn't find Islo.exe.".to_string())?;
    let name = exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| ASSET.into());
    let new = exe.with_file_name(format!("{name}.new"));
    let old = exe.with_file_name(format!("{name}.old"));

    let bytes = crate::http::get("github.com", &format!("/{REPO}/releases/download/{}/{ASSET}", release.tag), MAX_EXE)
        .ok_or_else(|| "The download failed. Try again in a moment.".to_string())?;
    // Every Windows exe starts with "MZ"; anything else is an error page, not Islo.
    if bytes.len() < 1024 * 1024 || !bytes.starts_with(b"MZ") {
        return Err("The download wasn't a valid Islo.exe.".into());
    }
    std::fs::write(&new, &bytes).map_err(|_| {
        "Islo's folder is read-only. Move Islo to a folder you can write to (like Documents), or download the update yourself.".to_string()
    })?;

    // Swap. A leftover .old from a past update would block step 1, so clear it first.
    let _ = std::fs::remove_file(&old);
    if std::fs::rename(&exe, &old).is_err() {
        let _ = std::fs::remove_file(&new);
        return Err("Windows didn't let Islo replace itself. Try again after restarting Islo.".into());
    }
    if std::fs::rename(&new, &exe).is_err() {
        // put the running version back so Islo still starts next time
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&new);
        return Err("Couldn't put the new version in place. Nothing was changed.".into());
    }
    Ok(exe)
}

/// Start the freshly installed exe. The caller quits right after.
pub fn relaunch(exe: &std::path::Path) -> bool {
    std::process::Command::new(exe).arg(AFTER_UPDATE_ARG).spawn().is_ok()
}

/// After an update: remove the previous version, which was moved aside.
/// It may still be exiting for a moment, so retry briefly in the background.
pub fn clean_up_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().to_string()) else { return };
    let old = exe.with_file_name(format!("{name}.old"));
    let new = exe.with_file_name(format!("{name}.new"));
    let _ = std::fs::remove_file(&new); // an interrupted download
    if !old.exists() {
        return;
    }
    let _ = std::thread::Builder::new().name("islo-update-cleanup".into()).stack_size(64 * 1024).spawn(move || {
        for _ in 0..20 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}
