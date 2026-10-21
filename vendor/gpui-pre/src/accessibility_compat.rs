//! Full-node compatibility properties attached to GPUI's official AccessKit tree.
//! No platform adapter, alternate tree, focus claim or raw native pointer lives here.
use crate::{App, Window};
use std::{cell::RefCell, rc::Rc};
#[derive(Clone)]
pub(crate) struct Accessibility {
    pub(crate) node: accesskit::Node,
    pub(crate) handler:
        Rc<RefCell<Box<dyn FnMut(&accesskit::ActionRequest, &mut Window, &mut App)>>>,
}

/// Values read from actual AppKit NSAccessibility objects, available only to
/// native regression builds. This is distinct from the GPUI semantic snapshot.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug)]
pub struct NativeAccessibilityNode {
    /// Native AX role returned by AppKit.
    pub role: String,
    /// Native accessible label.
    pub label: String,
    /// Native accessible value, when present.
    pub value: Option<String>,
    /// Depth in the native provider tree.
    pub depth: usize,
}

#[cfg(any(test, feature = "test-support"))]
impl Window {
    /// Inspect real platform provider objects. None means this platform or test
    /// platform has no native inspection implementation; it is not a pass.
    pub fn native_accessibility_snapshot(&self) -> Option<Vec<NativeAccessibilityNode>> {
        self.platform_window.native_accessibility_snapshot()
    }
    /// Invoke the actual native provider's press action by accessible label.
    pub fn native_accessibility_press(&self, label: &str) -> bool {
        self.platform_window.native_accessibility_press(label)
    }
}

#[cfg(test)]
mod block_compat_tests {
    use crate::{InteractiveElement as _, div};
    use accesskit::{Invalid, Live, Node, Rect, Role};

    #[test]
    fn full_node_properties_preserve_official_bounds() {
        let mut source = Node::new(Role::TextInput);
        source.set_required();
        source.set_read_only();
        source.set_invalid(Invalid::True);
        source.set_live(Live::Polite);
        source.set_value("中文 · Block");
        source.set_scroll_y_min(0.);
        source.set_scroll_y_max(280.);
        source.set_scroll_y(84.);
        source.set_description("Choose a real existing directory");
        let mut element = div()
            .id("real-input")
            .accessibility(source, true, |_, _, _| {});
        let expected_bounds = Rect {
            x0: 10.,
            y0: 20.,
            x1: 180.,
            y1: 52.,
        };
        let mut output = Node::new(Role::Group);
        output.set_bounds(expected_bounds);
        element.interactivity().write_a11y_info(&mut output);
        assert_eq!(output.role(), Role::TextInput);
        assert_eq!(output.bounds(), Some(expected_bounds));
        assert!(output.is_required());
        assert!(output.is_read_only());
        assert_eq!(output.invalid(), Some(Invalid::True));
        assert_eq!(output.live(), Some(Live::Polite));
        assert_eq!(output.value(), Some("中文 · Block"));
        assert_eq!(output.scroll_y_max(), Some(280.));
        assert_eq!(output.scroll_y(), Some(84.));
        assert_eq!(
            output.description(),
            Some("Choose a real existing directory")
        );
    }
}
