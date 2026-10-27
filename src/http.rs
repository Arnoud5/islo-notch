//! Tiny HTTPS GET over WinHTTP (built into Windows, no extra crates). Used by
//! lyrics (LRCLIB) and the updater (GitHub Releases). Blocking: call it from a
//! worker thread, never the UI thread.

use windows::Win32::Networking::WinHttp::*;
use windows::core::{PCWSTR, w};

/// GET `https://{host}{path}`. Redirects (also to other hosts, like GitHub's
/// download CDN) are followed by WinHTTP. Returns the body on status 200, or
/// None on any error or when the body grows past `max` bytes.
pub fn get(host: &str, path: &str, max: usize) -> Option<Vec<u8>> {
    request(w!("GET"), host, path, max).map(|(body, _)| body)
}

/// HEAD `https://{host}{path}` and return the URL it ends at after redirects
/// (status 200 only). Cheap: no body is downloaded.
pub fn final_url(host: &str, path: &str) -> Option<String> {
    request(w!("HEAD"), host, path, 0).map(|(_, url)| url)
}

fn request(method: PCWSTR, host: &str, path: &str, max: usize) -> Option<(Vec<u8>, String)> {
    let agent = crate::system::wide(&format!("Islo/{} (Dynamic Island for Windows)", env!("CARGO_PKG_VERSION")));
    let host = crate::system::wide(host);
    let path = crate::system::wide(path);
    unsafe {
        let session = WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() {
            return None;
        }
        let result = (|| {
            let connect = WinHttpConnect(session, PCWSTR(host.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0);
            if connect.is_null() {
                return None;
            }
            let req = WinHttpOpenRequest(connect, method, PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE);
            let out = (|| {
                if req.is_null() {
                    return None;
                }
                WinHttpSendRequest(req, None, None, 0, 0, 0).ok()?;
                WinHttpReceiveResponse(req, std::ptr::null_mut()).ok()?;
                let mut code = 0u32;
                let mut size = 4u32;
                WinHttpQueryHeaders(
                    req,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    PCWSTR::null(),
                    Some(&mut code as *mut _ as _),
                    &mut size,
                    std::ptr::null_mut(),
                )
                .ok()?;
                if code != 200 {
                    return None;
                }
                // where the redirects ended up (in bytes, UTF-16)
                let mut url = vec![0u16; 2048];
                let mut len = (url.len() * 2) as u32;
                let url = match WinHttpQueryOption(req, WINHTTP_OPTION_URL, Some(url.as_mut_ptr() as _), &mut len) {
                    Ok(()) => String::from_utf16_lossy(&url[..len as usize / 2]),
                    Err(_) => String::new(),
                };
                let mut body = Vec::new();
                loop {
                    let mut avail = 0u32;
                    WinHttpQueryDataAvailable(req, &mut avail).ok()?;
                    if avail == 0 {
                        break;
                    }
                    let mut buf = vec![0u8; avail as usize];
                    let mut read = 0u32;
                    WinHttpReadData(req, buf.as_mut_ptr() as _, avail, &mut read).ok()?;
                    body.extend_from_slice(&buf[..read as usize]);
                    if body.len() > max {
                        return None;
                    }
                }
                Some((body, url))
            })();
            if !req.is_null() {
                let _ = WinHttpCloseHandle(req);
            }
            let _ = WinHttpCloseHandle(connect);
            out
        })();
        let _ = WinHttpCloseHandle(session);
        result
    }
}

/// Like `get`, for text responses.
pub fn get_text(host: &str, path: &str, max: usize) -> Option<String> {
    String::from_utf8(get(host, path, max)?).ok()
}

/// Find the first `"key": "value"` in a JSON text and decode the string.
/// (A tiny extractor so we don't need a JSON crate for a few fields.)
pub fn json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(pos) = json[from..].find(&needle) {
        let after = from + pos + needle.len();
        let rest = json[after..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        if let Some(body) = rest.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = body.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return if out.trim().is_empty() { None } else { Some(out) },
                    '\\' => match chars.next()? {
                        'n' => out.push('\n'),
                        'r' => {}
                        't' => out.push('\t'),
                        'u' => {
                            let hex: String = chars.by_ref().take(4).collect();
                            let code = u32::from_str_radix(&hex, 16).ok()?;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        other => out.push(other),
                    },
                    c => out.push(c),
                }
            }
            return None;
        }
        // value was null / not a string: keep looking (search returns an array)
        from = after;
    }
    None
}
