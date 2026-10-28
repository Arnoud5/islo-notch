//! Turning on notifications from Settings ("Set up").
//!
//! Windows only lets apps with a *package identity* read notifications. Islo
//! gets one by registering package\AppxManifest.xml as a sparse package for the
//! Islo.exe in its folder. Those files are baked into Islo.exe (see build.rs),
//! so the exe is all users need. Setting up:
//!   1. Developer Mode on (needed for an unsigned package). Windows asks for
//!      permission once, because it's a machine-wide setting.
//!   2. package\ (and the logos in Assets\) written next to Islo.exe.
//!   3. The package registered with Windows (Add-AppxPackage -Register).
//!   4. Islo restarts: only a new process gets the identity.
//! Remove undoes step 3 (see `remove_and_relaunch`).

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{PCWSTR, w};

include!(concat!(env!("OUT_DIR"), "/package_files.rs"));

/// Passed to the restarted Islo, so it asks Windows for notification access right away.
pub const READY_ARG: &str = "--notifications-ready";
/// Passed to the restarted Islo when Remove couldn't unregister it.
pub const REMOVE_FAILED_ARG: &str = "--notifications-remove-failed";

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A Windows tool by its full path in System32, so a same-named file next to
/// Islo.exe (or anywhere on PATH) is never started instead.
fn system_tool(rel: &str) -> PathBuf {
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    let dir = if n > 0 && n < buf.len() { String::from_utf16_lossy(&buf[..n]) } else { r"C:\Windows\System32".into() };
    PathBuf::from(dir).join(rel)
}

fn powershell() -> std::process::Command {
    let mut c = std::process::Command::new(system_tool(r"WindowsPowerShell\v1.0\powershell.exe"));
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"]).creation_flags(CREATE_NO_WINDOW);
    c
}

pub fn developer_mode() -> bool {
    crate::system::reg_dword(
        HKEY_LOCAL_MACHINE,
        w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModelUnlock"),
        w!("AllowDevelopmentWithoutDevLicense"),
    ) == Some(1)
}

/// Set up in the background. `cb` runs on the worker thread with Ok when Islo
/// is registered (restart it next), or a message for the user.
pub fn set_up(cb: impl FnOnce(Result<(), String>) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("islo-package".into())
        .stack_size(256 * 1024)
        .spawn(move || cb(run()));
}

fn run() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|_| "Couldn't find Islo.exe.".to_string())?;
    // The manifest names Islo.exe, so a renamed copy wouldn't get the identity.
    if !exe.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Islo.exe")) {
        return Err("Rename this file to Islo.exe, start it again, then click Set up.".into());
    }
    let dir = exe.parent().ok_or_else(|| "Couldn't find Islo's folder.".to_string())?.to_path_buf();
    if !developer_mode() {
        enable_developer_mode()?;
    }
    write_files(&dir)?;
    register(&dir.join("package").join("AppxManifest.xml"), &dir)
}

/// reg.exe as administrator: Windows shows its permission prompt.
fn enable_developer_mode() -> Result<(), String> {
    let declined = "Islo needs Windows Developer Mode to read notifications. Click Set up again and choose Yes when Windows asks.";
    let params = crate::system::wide(
        "add HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModelUnlock /v AllowDevelopmentWithoutDevLicense /t REG_DWORD /d 1 /f",
    );
    let reg = crate::system::wide(&system_tool("reg.exe").to_string_lossy());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: HWND::default(),
        lpVerb: w!("runas"),
        lpFile: PCWSTR(reg.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| declined.to_string())?;
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            if code != 0 {
                return Err("Windows didn't turn on Developer Mode. Try again, or switch it on in Settings > System > For developers.".into());
            }
        }
    }
    if developer_mode() { Ok(()) } else { Err(declined.into()) }
}

/// package\... next to Islo.exe, plus a copy of the logos in Assets\ (some
/// shells look for them beside the exe, the package's external location).
fn write_files(dir: &Path) -> Result<(), String> {
    let read_only = "Islo's folder is read-only. Move Islo to a folder you can write to (like Documents), then try again.";
    for (rel, bytes) in FILES {
        let mut targets: Vec<PathBuf> = vec![dir.join("package").join(rel)];
        if rel.starts_with("Assets\\") {
            targets.push(dir.join(rel));
        }
        for path in targets {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|_| read_only.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|_| read_only.to_string())?;
        }
    }
    Ok(())
}

/// A path as a single-quoted PowerShell string. PowerShell also ends such a
/// string at the curly quotes ‘ ’ ‚ ‛ (C:\Users\O’Brien), so each quote
/// character inside is doubled, which PowerShell reads as one literal quote.
fn quote(p: &Path) -> String {
    let mut out = String::from("'");
    for c in p.to_string_lossy().chars() {
        if matches!(c, '\'' | '\u{2018}'..='\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

fn register(manifest: &Path, dir: &Path) -> Result<(), String> {
    let script = format!(
        "$ErrorActionPreference = 'Stop'\n\
         Get-AppxPackage -Name Islo | Remove-AppxPackage -ErrorAction SilentlyContinue\n\
         Add-AppxPackage -Register {} -ExternalLocation {}",
        quote(manifest),
        quote(dir)
    );
    let out = powershell().arg(&script).output().map_err(|_| "Couldn't start PowerShell to register Islo.".to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let first = err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown error");
    Err(format!("Windows didn't register Islo: {}", crate::system::display_text(first, 160)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_the_package_next_to_the_exe() {
        let dir = std::env::temp_dir().join(format!("islo-package-test-{}", std::process::id()));
        super::write_files(&dir).unwrap();
        let manifest = std::fs::read_to_string(dir.join("package\\AppxManifest.xml")).unwrap();
        assert!(manifest.contains("userNotificationListener") && manifest.contains("Executable=\"Islo.exe\""));
        for logo in ["StoreLogo.png", "Square150x150Logo.png", "Square44x44Logo.png"] {
            assert!(dir.join("package\\Assets").join(logo).exists(), "package\\Assets\\{logo}");
            assert!(dir.join("Assets").join(logo).exists(), "Assets\\{logo}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quotes_any_quote_in_a_path() {
        use std::path::Path;
        assert_eq!(super::quote(Path::new(r"C:\Users\O'Brien")), r"'C:\Users\O''Brien'");
        assert_eq!(super::quote(Path::new("C:\\Users\\O\u{2019}Brien")), "'C:\\Users\\O\u{2019}\u{2019}Brien'");
        assert_eq!(super::quote(Path::new("\u{2018}\u{201A}\u{201B}")), "'\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}'");
    }

    #[test]
    fn helpers_start_from_system32() {
        let ps = super::system_tool(r"WindowsPowerShell\v1.0\powershell.exe");
        assert!(ps.is_absolute() && ps.exists(), "{}", ps.display());
        assert!(super::system_tool("reg.exe").exists());
    }

    #[test]
    #[ignore] // starts PowerShell: cargo test package -- --ignored
    fn powershell_reads_a_curly_quoted_path_back() {
        let p = std::path::Path::new("C:\\Users\\O\u{2019}Brien 'x' \u{2018}y\u{201B}");
        let out = super::powershell().arg(format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; Write-Output {}", super::quote(p))).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), p.to_str().unwrap());
    }
}

/// Start Islo again with the identity, asking for notification access when it's up.
pub fn relaunch() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    std::process::Command::new(exe).args([crate::update::AFTER_UPDATE_ARG, READY_ARG]).spawn().is_ok()
}

/// Settings > Notifications > Remove: unregister Islo, then start it again
/// without the identity. Windows may close a package's running app while
/// removing it, so a hidden PowerShell waits for this Islo to quit first, then
/// removes the package and starts Islo. The caller quits right after `true`.
/// Developer Mode and the files next to Islo.exe stay as they are.
pub fn remove_and_relaunch() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let script = format!(
        "Wait-Process -Id {pid} -Timeout 20 -ErrorAction SilentlyContinue\n\
         $a = @('{after}')\n\
         try {{ Get-AppxPackage -Name Islo | Remove-AppxPackage -ErrorAction Stop }} catch {{ $a += '{failed}' }}\n\
         Start-Process -FilePath {exe} -ArgumentList $a",
        pid = std::process::id(),
        after = crate::update::AFTER_UPDATE_ARG,
        failed = REMOVE_FAILED_ARG,
        exe = quote(&exe),
    );
    powershell().arg(&script).spawn().is_ok()
}
