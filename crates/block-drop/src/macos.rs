//! AppKit implementation: an owned, transparent `NSView` subview that is the
//! dragging destination over the rendered quick-link drop zone.
//!
//! GPUI's classes, views and drag registrations are never patched or swizzled.
//! The launcher creates its own `NSView` subclass, registers it for the plain
//! text pasteboard type, and adds it as a subview of the GPUI view obtained
//! through `raw-window-handle`. The subview paints nothing and exists only as
//! a drag destination; the rendered zone itself has no pointer action, so
//! clicks inside it have nothing to forward.
//!
//! GPUI's AppKit view is not flipped, so the zone (top-left origin) is mirrored
//! into the view's bottom-left coordinate system before the subview is placed.

use std::fmt;

use anyhow::{Result, anyhow};
use async_channel::Sender as PayloadSender;
use objc2::{
    DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{
    NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSPasteboardType,
    NSPasteboardTypeString, NSView,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};

use crate::{DropZone, quick_link_payload};

/// Upper bound for a dragged quick link; real quick links are far smaller.
/// AppKit hands us the whole pasteboard string, so refuse implausibly large
/// payloads before they reach the UI.
const MAX_TEXT_BYTES: usize = 8192;

struct DropIvars {
    payloads: PayloadSender<String>,
}

define_class!(
    // SAFETY: `NSView` has no additional subclassing requirements here and the
    // class is main-thread-only, which matches every place this crate creates
    // or positions it. `DropView` does not implement `Drop`; `define_class!`
    // owns the ivar destruction.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "BlockProviderDropView"]
    #[ivars = DropIvars]
    struct DropView;

    unsafe impl NSObjectProtocol for DropView {}

    unsafe impl NSDraggingDestination for DropView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            drag_operation(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            drag_operation(sender)
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            accepted_text(sender).is_some()
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            match accepted_text(sender) {
                Some(text) => {
                    // A full channel means the UI already has a pending drop to
                    // process; dropping the newest payload keeps the AppKit
                    // callback non-blocking.
                    let _ = self.ivars().payloads.try_send(text);
                    true
                }
                None => false,
            }
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {}
    }
);

impl DropView {
    /// Creates the owned drop view with the supplied payload channel.
    fn new(
        frame: NSRect,
        payloads: PayloadSender<String>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DropIvars { payloads });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer. The
        // ivars were installed above, which is the order `define_class!`
        // requires, and `this` is a freshly allocated instance of this class.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

/// The drag operation for the offered pasteboard: copy for an Authlib quick
/// link, nothing for every other text payload (files, URLs, images, arbitrary
/// strings). The source always gets an explicit answer so the system drag UI
/// never shows a false acceptance.
fn drag_operation(sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
    if accepted_text(sender).is_some() {
        NSDragOperation::Copy
    } else {
        NSDragOperation::None
    }
}

/// Reads the plain-text payload and keeps it only when it is an Authlib quick
/// link. `text/plain` is the only type the target registers for, so file and
/// image drags never reach this point with an accepted payload.
fn accepted_text(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Option<String> {
    let pasteboard = sender.draggingPasteboard();
    // SAFETY: `NSPasteboardTypeString` is an immutable AppKit constant; reading
    // it cannot race or alias Rust memory.
    let pasteboard_type = unsafe { NSPasteboardTypeString };
    let text = pasteboard.stringForType(pasteboard_type)?;
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    let text = text.to_string();
    quick_link_payload(&text)?;
    Some(text)
}

/// Converts the logical top-left zone rectangle into a frame in the GPUI view's
/// bottom-left AppKit coordinate system. GPUI sizes are points on macOS, so no
/// display scale is applied; the subview frame uses exactly the same units.
#[must_use]
pub(crate) fn view_frame(zone: DropZone, parent_height: f64) -> NSRect {
    let width = f64::from(zone.width.max(1.0));
    let height = f64::from(zone.height.max(1.0));
    NSRect {
        origin: NSPoint {
            x: f64::from(zone.x),
            y: parent_height - f64::from(zone.y) - height,
        },
        size: NSSize { width, height },
    }
}

/// Owned AppKit drop view plus the parent view it is attached to. Dropping the
/// value removes the subview; the parent is retained so the geometry stays
/// valid even if the target outlives its render pass.
pub(crate) struct NativeTarget {
    view: Retained<DropView>,
    parent: Retained<NSView>,
}

impl fmt::Debug for NativeTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeTarget(appkit)")
    }
}

impl NativeTarget {
    pub(crate) fn attach(
        parent: isize,
        zone: DropZone,
        payloads: PayloadSender<String>,
    ) -> Result<Self> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err(anyhow!(
                "the AppKit drop target must be created on the main thread"
            ));
        };
        let parent = std::ptr::NonNull::new(parent as *mut NSView)
            .ok_or_else(|| anyhow!("the AppKit view handle is missing"))?;
        // SAFETY: the caller passes the live `NSView` from
        // `raw-window-handle` and promises that it stays alive for the lifetime
        // of this value; retaining it here keeps that promise explicit and
        // prevents the drop view from outliving its superview.
        let parent = unsafe { Retained::retain(parent.as_ptr()) }
            .ok_or_else(|| anyhow!("the AppKit view handle is invalid"))?;
        let view = DropView::new(view_frame(zone, parent.bounds().size.height), payloads, mtm);
        // SAFETY: `NSPasteboardTypeString` is an immutable AppKit constant.
        let types: Retained<NSArray<NSPasteboardType>> =
            NSArray::from_slice(&[unsafe { NSPasteboardTypeString }]);
        view.registerForDraggedTypes(&types);
        parent.addSubview(&view);
        Ok(Self { view, parent })
    }

    pub(crate) fn set_zone(&self, zone: DropZone) {
        // SAFETY: AppKit view geometry must be updated on the main thread, and
        // `set_zone` is called from the render pass that owns the window.
        if MainThreadMarker::new().is_none() {
            return;
        }
        self.view
            .setFrame(view_frame(zone, self.parent.bounds().size.height));
    }
}

impl Drop for NativeTarget {
    fn drop(&mut self) {
        // SAFETY: removing a subview is only valid on the main thread. If the
        // target is released during app teardown off the main thread, leaving
        // the (invisible, inert) view to the superview's destruction is the
        // only safe option.
        if MainThreadMarker::new().is_some() {
            self.view.removeFromSuperview();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::view_frame;
    use crate::DropZone;

    #[test]
    fn zone_is_mirrored_into_the_bottom_left_appkit_axis() {
        let frame = view_frame(DropZone::new(24.0, 200.0, 384.0, 96.0, 2.0), 600.0);
        assert_eq!(frame.origin.x, 24.0);
        assert_eq!(frame.origin.y, 304.0);
        assert_eq!(frame.size.width, 384.0);
        assert_eq!(frame.size.height, 96.0);
    }

    #[test]
    fn a_zero_sized_zone_stays_a_minimum_one_point_target() {
        let frame = view_frame(DropZone::new(0.0, 0.0, 0.0, 0.0, 1.0), 768.0);
        assert_eq!(frame.origin.x, 0.0);
        assert_eq!(frame.origin.y, 767.0);
        assert_eq!(frame.size.width, 1.0);
        assert_eq!(frame.size.height, 1.0);
    }
}
