//! Win32 implementation: an owned, nearly invisible child HWND carrying its own
//! OLE drop target over the rendered quick-link drop zone.
//!
//! GPUI's window and message hook are never patched. The child uses
//! `WS_EX_LAYERED` with 1/255 alpha so the status quo rendering shows through,
//! while the window remains a real hit-test target for OLE drag-and-drop.

use std::{
    fmt, ptr,
    sync::{
        Once,
        atomic::{AtomicU16, Ordering},
    },
};

use anyhow::{Context as _, Result};
use async_channel::Sender;
use windows::{
    Win32::{
        Foundation::{COLORREF, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, POINTL, WPARAM},
        System::{
            Com::{FORMATETC, IDataObject},
            LibraryLoader::GetModuleHandleW,
            Memory::{GlobalLock, GlobalUnlock},
            Ole::{
                DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
                OleInitialize, OleUninitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, LWA_ALPHA, MoveWindow, RegisterClassW,
            SetLayeredWindowAttributes, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_VISIBLE,
        },
    },
    core::{HSTRING, Ref, implement},
};

use crate::{DropZone, quick_link_payload};

const DROP_WINDOW_CLASS: windows::core::PCWSTR = windows::core::w!("BlockQuickLinkDrop");
/// `CF_UNICODETEXT` without pulling in the clipboard feature set.
const CF_UNICODETEXT_FORMAT: u16 = 13;
/// `TYMED_HGLOBAL`
const TYMED_HGLOBAL: u32 = 1;
/// `DVASPECT_CONTENT`
const DVASPECT_CONTENT: u32 = 1;

static REGISTER_CLASS: Once = Once::new();
static CLASS_ATOM: AtomicU16 = AtomicU16::new(0);

/// COM object that accepts Authlib quick links and forwards them to `block-ui`.
#[implement(IDropTarget)]
struct ZoneDropTarget {
    payloads: Sender<String>,
}

impl fmt::Debug for ZoneDropTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ZoneDropTarget")
    }
}

impl ZoneDropTarget {
    /// Reads the dragged `CF_UNICODETEXT`, if any.
    fn read_text(data: &IDataObject) -> Option<String> {
        let format = FORMATETC {
            cfFormat: CF_UNICODETEXT_FORMAT,
            ptd: ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        };
        // SAFETY: `data` is the live data object supplied by OLE for this drag;
        // `format` is a fully initialised plain struct.
        let mut medium = unsafe { data.GetData(&format) }.ok()?;
        let text = if medium.tymed == TYMED_HGLOBAL {
            // SAFETY: the union member is `hGlobal` precisely when
            // `tymed == TYMED_HGLOBAL`, which the guard just established.
            let handle: HGLOBAL = unsafe { medium.u.hGlobal };
            // SAFETY: an HGLOBAL-backed text medium holds a lockable block owned
            // by the medium; the pointer stays valid until GlobalUnlock below.
            let wide = unsafe { GlobalLock(handle) } as *const u16;
            let mut value = None;
            if !wide.is_null() {
                let mut length = 0usize;
                // SAFETY: CF_UNICODETEXT is a null-terminated UTF-16 buffer; the
                // scan stops at the terminator (never past it).
                unsafe {
                    while *wide.add(length) != 0 {
                        length += 1;
                    }
                    value = Some(String::from_utf16_lossy(std::slice::from_raw_parts(
                        wide, length,
                    )));
                }
                // SAFETY: balances the successful GlobalLock above.
                unsafe {
                    GlobalUnlock(handle).ok();
                }
            }
            value
        } else {
            None
        };
        // SAFETY: `medium` was produced by a successful GetData and is not
        // retained; ReleaseStgMedium is the matching cleanup call.
        unsafe { ReleaseStgMedium(&mut medium) };
        text
    }

    fn accept_effect(effect: *mut DROPEFFECT, accepted: bool) {
        // SAFETY: OLE passes a valid out-pointer for the duration of the call.
        if !effect.is_null() {
            unsafe {
                *effect = if accepted {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
        }
    }
}

impl IDropTarget_Impl for ZoneDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let accepted = data
            .as_ref()
            .and_then(ZoneDropTarget::read_text)
            .is_some_and(|text| quick_link_payload(&text).is_some());
        ZoneDropTarget::accept_effect(effect, accepted);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        ZoneDropTarget::accept_effect(effect, true);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let payload = data
            .as_ref()
            .and_then(ZoneDropTarget::read_text)
            .filter(|text| quick_link_payload(text).is_some());
        if let Some(payload) = payload {
            ZoneDropTarget::accept_effect(effect, true);
            // A full channel means the UI already has a pending drop to process;
            // dropping the newest payload keeps the OLE callback non-blocking.
            let _ = self.payloads.try_send(payload);
        } else {
            ZoneDropTarget::accept_effect(effect, false);
        }
        Ok(())
    }
}

/// Owned child window plus its registered OLE drop target.
pub(crate) struct NativeTarget {
    hwnd: isize,
    _drop_target: IDropTarget,
}

impl fmt::Debug for NativeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeTarget")
            .field("hwnd", &self.hwnd)
            .finish_non_exhaustive()
    }
}

fn hwnd_of(raw: isize) -> Option<HWND> {
    (raw != 0).then(|| HWND(raw as *mut core::ffi::c_void))
}

/// Minimal window procedure: the layered child paints nothing of its own.
unsafe extern "system" fn drop_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: every message this child does not interpret is delegated to the
    // system default procedure; the child carries no application state.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn ensure_window_class() -> Result<()> {
    REGISTER_CLASS.call_once(|| {
        // SAFETY: a null module name resolves to this process module; the handle
        // is only stored in the registered window class.
        let instance = unsafe { GetModuleHandleW(None) }
            .map_or_else(|_| HINSTANCE::default(), |module| HINSTANCE(module.0));
        let class = WNDCLASSW {
            lpfnWndProc: Some(drop_window_proc),
            hInstance: instance,
            lpszClassName: DROP_WINDOW_CLASS,
            ..WNDCLASSW::default()
        };
        // SAFETY: `class` is fully populated for the duration of the call and the
        // class name points at a static wide string literal.
        let atom = unsafe { RegisterClassW(&class) };
        CLASS_ATOM.store(atom, Ordering::Relaxed);
    });
    if CLASS_ATOM.load(Ordering::Relaxed) == 0 {
        anyhow::bail!("could not register the quick-link drop window class");
    }
    Ok(())
}

impl NativeTarget {
    pub(crate) fn attach(parent: isize, zone: DropZone, payloads: Sender<String>) -> Result<Self> {
        let parent = hwnd_of(parent).context("drop zone parent window is missing")?;
        ensure_window_class()?;
        // SAFETY: initialises OLE on the calling UI thread; the matching
        // OleUninitialize happens in Drop.
        unsafe { OleInitialize(None) }.context("could not initialize OLE for the drop target")?;
        let (x, y, width, height) = zone.pixel_rect();
        // SAFETY: the class atom is registered, the parent belongs to this
        // thread, and the child style flags are a plain constant combination.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE,
                DROP_WINDOW_CLASS,
                &HSTRING::from("Block quick-link drop target"),
                WS_CHILD | WS_CLIPSIBLINGS | WS_VISIBLE,
                x,
                y,
                width,
                height,
                Some(parent),
                None,
                None,
                None,
            )
        }
        .context("could not create the quick-link drop child window")?;
        // 1/255 alpha keeps the GPUI drop-zone visuals visible while the window
        // stays hit-testable for OLE drag-and-drop.
        // SAFETY: `hwnd` is the child window created above.
        unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA) }
            .context("could not make the quick-link drop window transparent")?;
        let drop_target: IDropTarget = ZoneDropTarget { payloads }.into();
        // SAFETY: `hwnd` is live and owned by this target; the interface stays
        // alive in `drop_target` until RevokeDragDrop in Drop.
        unsafe { RegisterDragDrop(hwnd, &drop_target) }
            .context("could not register the OLE drop target")?;
        Ok(Self {
            hwnd: hwnd.0 as isize,
            _drop_target: drop_target,
        })
    }

    pub(crate) fn set_zone(&self, zone: DropZone) {
        let (x, y, width, height) = zone.pixel_rect();
        let Some(hwnd) = hwnd_of(self.hwnd) else {
            return;
        };
        // SAFETY: resizes only this target's own child window inside the product
        // window client area; no GPUI-managed handle is touched.
        let _ = unsafe { MoveWindow(hwnd, x, y, width, height, false) };
    }
}

impl Drop for NativeTarget {
    fn drop(&mut self) {
        let Some(hwnd) = hwnd_of(self.hwnd) else {
            return;
        };
        // SAFETY: revokes the registration created in attach and then destroys
        // the owned child window, both exactly once and on the owning thread.
        unsafe {
            let _ = RevokeDragDrop(hwnd);
            let _ = DestroyWindow(hwnd);
        }
        // SAFETY: balances the OleInitialize performed in attach.
        unsafe { OleUninitialize() };
    }
}
