//! Display brightness changes via WMI's `WmiMonitorBrightnessEvent`
//! (built-in laptop panels; external monitors don't report it).
//! The thread blocks inside `Next(INFINITE)` and wakes only when the
//! brightness actually changes.

use windows::Win32::System::Com::*;
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::*;
use windows::Win32::System::Wmi::*;
use windows::core::{BSTR, w};

/// Calls `cb(percent, is_initial_value)`.
pub fn start(cb: impl Fn(u8, bool) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("islo-brightness".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let _ = unsafe { run(&cb) };
        });
}

unsafe fn read_u8(obj: &IWbemClassObject, name: windows::core::PCWSTR) -> Option<u8> {
    let mut v = VARIANT::default();
    unsafe { obj.Get(name, 0, &mut v, None, None).ok()? };
    // VARIANT is a C union: check the type tag, then read the matching field.
    let inner = unsafe { &v.Anonymous.Anonymous };
    let n = unsafe {
        match inner.vt {
            VT_UI1 => inner.Anonymous.bVal as i64,
            VT_I2 => inner.Anonymous.iVal as i64,
            VT_I4 | VT_INT => inner.Anonymous.lVal as i64,
            VT_UI4 | VT_UINT => inner.Anonymous.lVal as u32 as i64,
            _ => return None,
        }
    };
    Some(n.clamp(0, 100) as u8)
}

unsafe fn run(cb: &dyn Fn(u8, bool)) -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let svc = locator.ConnectServer(&BSTR::from("ROOT\\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)?;
        CoSetProxyBlanket(&svc, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, None, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE)?;

        // Initial value (also tells us whether this machine supports it at all).
        let q = svc.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE"),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )?;
        let mut objs = [None];
        let mut n = 0u32;
        let _ = q.Next(WBEM_INFINITE, &mut objs, &mut n);
        match (n, objs[0].take()) {
            (1, Some(obj)) => {
                if let Some(b) = read_u8(&obj, w!("CurrentBrightness")) {
                    cb(b, true);
                }
            }
            _ => return Ok(()), // no WMI brightness here (desktop / external monitor)
        }

        let events = svc.ExecNotificationQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT * FROM WmiMonitorBrightnessEvent"),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )?;
        loop {
            let mut objs = [None];
            let mut n = 0u32;
            if events.Next(WBEM_INFINITE, &mut objs, &mut n).is_err() {
                return Ok(());
            }
            if let (1, Some(obj)) = (n, objs[0].take()) {
                if let Some(b) = read_u8(&obj, w!("Brightness")) {
                    cb(b, false);
                }
            }
        }
    }
}
