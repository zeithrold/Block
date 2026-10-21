//! Opt-in native provider inspection. This reads the upstream adapter's AppKit
//! objects; it does not create a bridge, tree or event route.
use cocoa::{
    base::{BOOL, NO, YES, id, nil},
    foundation::{NSArray, NSString},
};
use objc::{msg_send, sel, sel_impl};
use std::{collections::HashSet, ffi::CStr};
unsafe fn text(value: id) -> Option<String> {
    if value == nil {
        return None;
    }
    let supported: BOOL = unsafe { msg_send![value, respondsToSelector: sel!(UTF8String)] };
    if supported == NO {
        return None;
    }
    let bytes = unsafe { value.UTF8String() };
    if bytes.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(bytes) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}
unsafe fn accessible_name(node: id) -> String {
    let label: id = unsafe { msg_send![node, accessibilityLabel] };
    if let Some(label) = unsafe { text(label) } {
        return label;
    }
    // The official AccessKit adapter exposes control names through the AppKit title
    // selector. Read that actual provider attribute rather than the schema.
    let title: id = unsafe { msg_send![node, accessibilityTitle] };
    unsafe { text(title) }.unwrap_or_default()
}
unsafe fn objects(
    array: id,
    depth: usize,
    visited: &mut HashSet<usize>,
    output: &mut Vec<(id, usize)>,
) {
    if array == nil || depth > 32 || output.len() > 4096 {
        return;
    }
    for index in 0..unsafe { array.count() } {
        let object = unsafe { array.objectAtIndex(index) };
        if !visited.insert(object as usize) {
            continue;
        }
        output.push((object, depth));
        let children: id = unsafe { msg_send![object, accessibilityChildren] };
        unsafe {
            objects(children, depth + 1, visited, output);
        }
    }
}

pub(super) unsafe fn snapshot(view: id) -> Vec<gpui::NativeAccessibilityNode> {
    let mut nodes = Vec::new();
    // SAFETY: The MacWindow owns this content view and calls on the main thread.
    let children: id = unsafe { msg_send![view, accessibilityChildren] };
    unsafe {
        objects(children, 0, &mut HashSet::new(), &mut nodes);
    }
    nodes
        .into_iter()
        .map(|(node, depth)| unsafe {
            let role: id = msg_send![node, accessibilityRole];
            let value: id = msg_send![node, accessibilityValue];
            gpui::NativeAccessibilityNode {
                role: text(role).unwrap_or_default(),
                label: accessible_name(node),
                value: text(value),
                depth,
            }
        })
        .collect()
}
pub(super) unsafe fn press(view: id, label: &str) -> bool {
    let mut nodes = Vec::new();
    let children: id = unsafe { msg_send![view, accessibilityChildren] };
    unsafe {
        objects(children, 0, &mut HashSet::new(), &mut nodes);
    }
    for (node, _) in nodes {
        if unsafe { accessible_name(node) } == label {
            let pressed: BOOL = unsafe { msg_send![node, accessibilityPerformPress] };
            return pressed == YES;
        }
    }
    false
}
