//! Win32 implementation: an owned, nearly invisible child HWND carrying its own
//! OLE drop target over the rendered quick-link drop zone.
//!
//! GPUI's window and message hook are never patched. The child uses
//! `WS_EX_LAYERED` with 1/255 alpha so the status quo rendering shows through,
//! while the window remains a real hit-test target for OLE drag-and-drop.

use std::{
    fmt,
    mem::size_of,
    ptr,
    sync::{
        Once,
        atomic::{AtomicBool, AtomicU16, Ordering},
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
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{
                DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
                OleInitialize, OleUninitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, LWA_ALPHA, MoveWindow, RegisterClassW,
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
            SetLayeredWindowAttributes, SystemParametersInfoW, WNDCLASSW, WS_CHILD,
            WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_VISIBLE,
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
    /// Whether the payload of the drag currently over the zone was accepted.
    accepted: AtomicBool,
}

impl fmt::Debug for ZoneDropTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ZoneDropTarget")
    }
}

/// Length of the UTF-16 payload inside an allocation that may not be terminated.
///
/// The caller passes exactly the units the HGLOBAL can hold, so a malformed drag
/// source without a NUL terminator can never make the scan read past the block.
fn utf16_units_len(units: &[u16]) -> usize {
    units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len())
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
        let mut text = None;
        if medium.tymed == TYMED_HGLOBAL {
            // SAFETY: the union member is `hGlobal` precisely when
            // `tymed == TYMED_HGLOBAL`, which the guard just established.
            let handle: HGLOBAL = unsafe { medium.u.hGlobal };
            // SAFETY: `handle` belongs to the medium; GlobalSize returns the
            // allocation size in bytes, so the scan below cannot leave the block.
            let max_units = unsafe { GlobalSize(handle) } / size_of::<u16>();
            // SAFETY: an HGLOBAL-backed text medium holds a lockable block owned
            // by the medium; the pointer stays valid until GlobalUnlock below.
            let wide = unsafe { GlobalLock(handle) } as *const u16;
            if !wide.is_null() {
                if max_units > 0 {
                    // SAFETY: GlobalLock succeeded and the block is at least
                    // `max_units` UTF-16 units according to GlobalSize.
                    let units = unsafe { std::slice::from_raw_parts(wide, max_units) };
                    let length = utf16_units_len(units);
                    text = Some(String::from_utf16_lossy(&units[..length]));
                }
                // SAFETY: balances the successful GlobalLock above.
                unsafe {
                    GlobalUnlock(handle).ok();
                }
            }
        }
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
        self.accepted.store(accepted, Ordering::Relaxed);
        ZoneDropTarget::accept_effect(effect, accepted);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        ZoneDropTarget::accept_effect(effect, self.accepted.load(Ordering::Relaxed));
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.accepted.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let accepted = self.accepted.swap(false, Ordering::Relaxed);
        let payload = data
            .as_ref()
            .and_then(ZoneDropTarget::read_text)
            .filter(|text| quick_link_payload(text).is_some());
        if accepted && let Some(payload) = payload {
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
    drop_target: Option<IDropTarget>,
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
        // OleUninitialize happens in Drop, or immediately below when attaching
        // fails before the target exists.
        unsafe { OleInitialize(None) }.context("could not initialize OLE for the drop target")?;
        let attached = Self::create_registered(parent, zone, payloads);
        if attached.is_err() {
            // SAFETY: balances the OleInitialize above because no target (and
            // therefore no Drop) owns this initialisation on the error path.
            unsafe { OleUninitialize() };
        }
        attached
    }

    /// Creates and registers the child window; callers already own the OLE
    /// initialisation and must release it when this returns an error.
    fn create_registered(parent: HWND, zone: DropZone, payloads: Sender<String>) -> Result<Self> {
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
        let transparent = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA) }
            .context("could not make the quick-link drop window transparent");
        if let Err(error) = transparent {
            // SAFETY: destroys the child created above before returning.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
        let drop_target: IDropTarget = ZoneDropTarget {
            payloads,
            accepted: AtomicBool::new(false),
        }
        .into();
        // SAFETY: `hwnd` is live and owned by this target; the interface stays
        // alive in `drop_target` until Drop releases it.
        let registered = unsafe { RegisterDragDrop(hwnd, &drop_target) }
            .context("could not register the OLE drop target");
        if let Err(error) = registered {
            // SAFETY: destroys the child created above before returning; the
            // local interface drops naturally.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
        Ok(Self {
            hwnd: hwnd.0 as isize,
            drop_target: Some(drop_target),
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
        // SAFETY: revokes the registration created in create_registered; the
        // registration is revoked exactly once and on the owning thread.
        unsafe {
            let _ = RevokeDragDrop(hwnd);
        }
        // Release our interface reference while the COM apartment and the child
        // window are still alive: deterministic OLE teardown ordering.
        drop(self.drop_target.take());
        // SAFETY: destroys the owned child window and then balances the
        // OleInitialize performed in attach, in that order.
        unsafe {
            let _ = DestroyWindow(hwnd);
            OleUninitialize();
        }
    }
}

/// Whether Windows asks applications to reduce motion.
///
/// Reads `SPI_GETCLIENTAREAANIMATION`, the "Show animations in Windows"
/// accessibility setting; the query returns whether animations are enabled, so
/// reduced motion is its negation. A failed query keeps motion enabled.
pub(crate) fn system_reduce_motion() -> bool {
    let mut animations_enabled: i32 = 0;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes exactly one BOOL into the
    // supplied pointer, which is valid for the duration of the call.
    let queried = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(ptr::from_mut(&mut animations_enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    queried.is_ok() && animations_enabled == 0
}

#[cfg(test)]
mod tests {
    use super::utf16_units_len;

    #[test]
    fn utf16_units_stop_at_the_terminator() {
        assert_eq!(utf16_units_len(&[65, 66, 0, 67]), 2);
    }

    #[test]
    fn utf16_units_use_the_whole_bounded_buffer_without_a_terminator() {
        assert_eq!(utf16_units_len(&[65, 66]), 2);
    }

    #[test]
    fn utf16_units_handle_an_empty_buffer() {
        assert_eq!(utf16_units_len(&[]), 0);
    }
}
