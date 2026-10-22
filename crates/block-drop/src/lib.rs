//! Owned native drop target for the Authlib quick-link drop zone.
//!
//! This crate is the only place in the workspace where `unsafe_code` is lowered:
//! the rest of the launcher forbids it. GPUI windows are never subclassed,
//! patched or swizzled. The launcher instead creates its own transparent native
//! drop surface over the rendered drop zone — an OLE-registered child HWND on
//! Windows, an owned `NSView` dragging destination on macOS and an XDND child
//! window on Linux X11 — and forwards accepted text payloads over an async
//! channel to the UI thread. Every `unsafe` block carries a SAFETY comment
//! explaining the invariant it relies on.

#![allow(unsafe_code)]

use std::fmt;

use anyhow::Result;

/// The Authlib quick-link scheme the drop zone accepts.
pub const QUICK_LINK_PREFIX: &str = "authlib-injector:yggdrasil-server:";

/// Extracts the percent-encoded Yggdrasil API root from an Authlib quick link.
///
/// The payload is returned verbatim (still percent-encoded) so the existing
/// `block_auth::ProviderAddress` parser stays the only decoder and validator.
#[must_use]
pub fn quick_link_payload(text: &str) -> Option<&str> {
    let text = text.trim();
    let payload = text.strip_prefix(QUICK_LINK_PREFIX)?;
    (!payload.is_empty()).then_some(payload)
}

/// Picks the text type the target will accept from an XDND offer.
///
/// `UTF8_STRING` is preferred, then `text/plain`, then legacy `STRING`. Any offer
/// without one of those (files, URLs, images, ...) yields `None` so the zone
/// never accepts a payload it cannot validate as a quick link.
#[cfg_attr(
    not(all(unix, not(target_os = "macos"))),
    expect(dead_code, reason = "consumed by the XDND backend on Linux")
)]
#[must_use]
pub(crate) fn text_offer(types: &[u32], utf8: u32, text_plain: u32, string: u32) -> Option<u32> {
    [utf8, text_plain, string]
        .into_iter()
        .find(|candidate| types.contains(candidate))
}

/// Logical bounds of the rendered drop zone inside a window client area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropZone {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

impl DropZone {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32, scale: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    /// Converts the logical zone into the physical pixel rectangle Win32 uses.
    #[must_use]
    pub fn pixel_rect(self) -> (i32, i32, i32, i32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let round = |value: f32| -> i32 {
            let scaled = (value * scale).round();
            if scaled.is_finite() {
                scaled.clamp(0.0, i32::MAX as f32) as i32
            } else {
                0
            }
        };
        (
            round(self.x),
            round(self.y),
            round(self.width).max(1),
            round(self.height).max(1),
        )
    }
}

#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(all(unix, not(target_os = "macos")))]
mod linux;

#[cfg(not(any(windows, unix)))]
mod stub;

#[cfg(windows)]
use windows::NativeTarget;

#[cfg(target_os = "macos")]
use macos::NativeTarget;

#[cfg(all(unix, not(target_os = "macos")))]
use linux::NativeTarget;

#[cfg(not(any(windows, unix)))]
use stub::NativeTarget;

/// Whether the operating system asks applications to reduce motion.
///
/// Windows reports `SPI_GETCLIENTAREAANIMATION` ("Show animations in Windows");
/// it means animations are enabled, so reduced motion is its negation. Other
/// platforms keep the caller's stored preference and report `false` here.
#[must_use]
pub fn system_reduce_motion() -> bool {
    #[cfg(windows)]
    {
        windows::system_reduce_motion()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// A live provider drop zone; dropping this value revokes the native drag
/// registration and removes the owned surface.
pub struct DropTarget(NativeTarget);

impl fmt::Debug for DropTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DropTarget").field(&self.0).finish()
    }
}

impl DropTarget {
    /// Creates the owned native drop surface over the product window.
    ///
    /// `parent_window` is the platform window handle obtained through
    /// `raw-window-handle`: the Win32 `HWND`, the X11 window id, or the
    /// `NSView` pointer. It must stay alive while this value exists, which the
    /// platform targets enforce by retaining it. On platforms without a
    /// backend the target is inert so the launcher keeps a single code path.
    ///
    /// # Errors
    /// Fails when the parent handle is missing or the platform refuses the
    /// owned drop window or its drag registration.
    pub fn attach(
        parent_window: isize,
        zone: DropZone,
        payloads: async_channel::Sender<String>,
    ) -> Result<Self> {
        Ok(Self(NativeTarget::attach(parent_window, zone, payloads)?))
    }

    /// Moves the owned child window to the latest rendered drop zone.
    pub fn set_zone(&self, zone: DropZone) {
        self.0.set_zone(zone);
    }
}

#[cfg(test)]
mod tests {
    use super::{DropZone, quick_link_payload, text_offer};

    #[test]
    fn quick_link_payload_accepts_the_authlib_scheme() {
        assert_eq!(
            quick_link_payload("authlib-injector:yggdrasil-server:https%3A%2F%2Fexample.com%2Fapi"),
            Some("https%3A%2F%2Fexample.com%2Fapi")
        );
    }

    #[test]
    fn quick_link_payload_trims_surrounding_whitespace() {
        assert_eq!(
            quick_link_payload("  authlib-injector:yggdrasil-server:https%3A%2F%2Fa.example\r\n"),
            Some("https%3A%2F%2Fa.example")
        );
    }

    #[test]
    fn quick_link_payload_rejects_other_text() {
        assert_eq!(quick_link_payload("https://example.com/api"), None);
        assert_eq!(quick_link_payload("authlib-injector:other:value"), None);
        assert_eq!(
            quick_link_payload("authlib-injector:yggdrasil-server:"),
            None
        );
        assert_eq!(quick_link_payload(""), None);
    }

    #[test]
    fn xdnd_offers_choose_a_supported_text_type() {
        let (utf8, text_plain, string) = (10, 11, 12);
        assert_eq!(text_offer(&[10, 13], utf8, text_plain, string), Some(10));
        assert_eq!(text_offer(&[11], utf8, text_plain, string), Some(11));
        assert_eq!(text_offer(&[12], utf8, text_plain, string), Some(12));
        assert_eq!(
            text_offer(&[10, 11, 12], utf8, text_plain, string),
            Some(10),
            "UTF8_STRING wins when several text formats are offered"
        );
        assert_eq!(text_offer(&[], utf8, text_plain, string), None);
        for other in [&[13u32][..], &[14, 15], &[100]] {
            assert_eq!(text_offer(other, utf8, text_plain, string), None);
        }
    }

    #[test]
    fn logical_zone_becomes_a_physical_rectangle() {
        let zone = DropZone::new(10.5, 20.25, 300.0, 80.0, 2.0);
        assert_eq!(zone.pixel_rect(), (21, 41, 600, 160));
        let degenerate = DropZone::new(0.0, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(degenerate.pixel_rect(), (0, 0, 1, 1));
    }
}
