//! File shelf: drop files onto the island, drag them back out to any app.
//!
//! * Drop in: the island registers an OLE `IDropTarget`.
//! * Drag out: the shell builds the data object for the file (same as Explorer),
//!   so every app that accepts files from Explorer accepts them from Islo.
//! * Icons: the system's 48 px file icon, converted to pixels once per file.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HRESULT, PCWSTR, Ref, implement};

pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    /// Size in bytes (folders: 0).
    pub bytes: u64,
    pub icon: Option<(u32, Vec<u8>)>, // (size, RGBA)
}

pub enum DropEvent {
    Enter,
    Leave,
    Drop(Vec<PathBuf>),
}

thread_local! {
    /// True while we are dragging *out*, so we don't accept our own drag.
    static DRAGGING_OUT: Cell<bool> = const { Cell::new(false) };
    /// The UI decides whether drops are accepted (module enabled?).
    static ACCEPTING: Cell<bool> = const { Cell::new(true) };
}

pub fn set_accepting(on: bool) {
    ACCEPTING.with(|a| a.set(on));
}

fn hdrop_format() -> FORMATETC {
    FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

#[implement(IDropTarget)]
struct DropTarget {
    on_event: Box<dyn Fn(DropEvent)>,
    accept: Cell<bool>,
}

impl DropTarget {
    fn effect(&self, out: *mut DROPEFFECT) {
        if let Some(e) = unsafe { out.as_mut() } {
            *e = if self.accept.get() { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let has_files = data.as_ref().map(|d| unsafe { d.QueryGetData(&hdrop_format()) } == S_OK).unwrap_or(false);
        let ok = has_files && ACCEPTING.with(|a| a.get()) && !DRAGGING_OUT.with(|d| d.get());
        self.accept.set(ok);
        self.effect(effect);
        if ok {
            (self.on_event)(DropEvent::Enter);
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        self.effect(effect);
        Ok(())
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.accept.replace(false) {
            (self.on_event)(DropEvent::Leave);
        }
        Ok(())
    }
    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        self.effect(effect);
        if !self.accept.replace(false) {
            return Ok(());
        }
        let mut files = Vec::new();
        if let Some(d) = data.as_ref() {
            unsafe {
                if let Ok(mut medium) = d.GetData(&hdrop_format()) {
                    let hdrop = HDROP(medium.u.hGlobal.0);
                    let count = DragQueryFileW(hdrop, u32::MAX, None);
                    for i in 0..count {
                        let len = DragQueryFileW(hdrop, i, None) as usize;
                        let mut buf = vec![0u16; len + 1];
                        DragQueryFileW(hdrop, i, Some(&mut buf));
                        files.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
                    }
                    ReleaseStgMedium(&mut medium);
                }
            }
        }
        (self.on_event)(DropEvent::Drop(files));
        Ok(())
    }
}

/// Must run on the UI thread after `OleInitialize`.
pub fn register_drop_target(hwnd: HWND, on_event: impl Fn(DropEvent) + 'static) {
    let target: IDropTarget = DropTarget { on_event: Box::new(on_event), accept: Cell::new(false) }.into();
    unsafe {
        let _ = RegisterDragDrop(hwnd, &target);
    }
}

/// Undo `register_drop_target` before the window goes away.
pub fn revoke_drop_target(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & windows::Win32::System::SystemServices::MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Start a drag of `path`. Blocks until dropped. Returns true if the file was moved.
pub fn drag_out(path: &Path) -> bool {
    let wide = crate::system::wide(&path.to_string_lossy());
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None) else { return false };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return false };
        let source: IDropSource = DropSource.into();
        let mut effect = DROPEFFECT(0);
        DRAGGING_OUT.with(|d| d.set(true));
        let r = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect);
        DRAGGING_OUT.with(|d| d.set(false));
        r == DRAGDROP_S_DROP && effect == DROPEFFECT_MOVE
    }
}

pub fn make_entry(path: PathBuf) -> Entry {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
    let icon = file_icon(&path);
    let bytes = std::fs::metadata(&path).map(|m| if m.is_file() { m.len() } else { 0 }).unwrap_or(0);
    Entry { path, name, bytes, icon }
}

fn file_icon(path: &Path) -> Option<(u32, Vec<u8>)> {
    let wide = crate::system::wide(&path.to_string_lossy());
    unsafe {
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(PCWSTR(wide.as_ptr()), Default::default(), Some(&mut sfi), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX);
        if r == 0 {
            return None;
        }
        let list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).ok()?;
        let hicon = list.GetIcon(sfi.iIcon, windows::Win32::UI::Controls::ILD_TRANSPARENT.0).ok()?;
        let out = icon_pixels(hicon, 48);
        let _ = DestroyIcon(hicon);
        out.map(|p| (48, p))
    }
}

unsafe fn icon_pixels(hicon: HICON, size: u32) -> Option<Vec<u8>> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(hicon, &mut ii).ok()?;
        let dc = CreateCompatibleDC(None);
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size as i32;
        bmi.bmiHeader.biHeight = -(size as i32);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;
        let mut px = vec![0u8; (size * size * 4) as usize];
        let lines = GetDIBits(dc, ii.hbmColor, 0, size, Some(px.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        if lines == 0 {
            return None;
        }
        let no_alpha = px.chunks_exact(4).all(|p| p[3] == 0);
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2); // BGRA -> RGBA
            if no_alpha {
                p[3] = 255;
            }
        }
        Some(px)
    }
}


/// "860 KB", "2.4 MB", "1.1 GB"
pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
