//! Additional keyboard navigation for the product field.
use super::*;
use unicode_segmentation::UnicodeSegmentation as _;

impl<V: Clone + PartialEq + 'static> Mounted<V> {
    /// Kit exposes the committed selection setter, but not a cursor setter.
    /// Move its native list cursor, never the checked value, for Block's extra keys.
    pub(super) fn key(&mut self, event: &gpui::KeystrokeEvent, window: &mut Window, cx: &mut App) {
        let list = self.picker.focus_handle(cx);
        if list == self.trigger || !list.is_focused(window) || self.rows.is_empty() {
            return;
        }
        let key = &event.keystroke;
        let modifiers = key.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        if key.key == "tab" {
            // Let Kit cancel from its real content focus, then traverse exactly once
            // from its restored trigger (including a containing Sheet's focus trap).
            list.dispatch_action(&gpui_base::actions::Cancel, window, cx);
            let trigger = self.trigger.clone();
            window.defer(cx, move |window, cx| {
                trigger.focus(window, cx);
                crate::base_overlay::advance_focus(modifiers.shift, window, cx);
            });
            cx.stop_propagation();
            return;
        }
        let count = self.rows.len();
        let current = self.picker.read(cx).selected_index(cx).map(|ix| ix.row);
        let target = match key.key.as_str() {
            "home" => 0,
            "end" => count - 1,
            "up" | "down" => {
                self.prefix.clear();
                return;
            }
            _ => {
                let text = key.key_char.as_deref().unwrap_or(&key.key);
                if text.graphemes(true).count() != 1
                    || text.trim().is_empty()
                    || text.chars().any(char::is_control)
                {
                    return;
                }
                let now = cx.background_executor().now();
                if self.typed_at.is_none_or(|last| {
                    now.saturating_duration_since(last) >= std::time::Duration::from_millis(700)
                }) {
                    self.prefix.clear();
                }
                self.typed_at = Some(now);
                let text = text.to_lowercase();
                if self.prefix.is_empty() || !self.prefix.graphemes(true).all(|part| part == text) {
                    self.prefix.push_str(&text);
                }
                let start = current.unwrap_or(0) + usize::from(self.prefix == text);
                let found = (0..count)
                    .map(|offset| (start + offset) % count)
                    .find(|&i| self.rows[i].label.to_lowercase().starts_with(&self.prefix));
                cx.stop_propagation();
                let Some(found) = found else {
                    return;
                };
                found
            }
        };
        let next = current.map_or(target + 1, |from| (target + count - from) % count);
        let prev = current.map_or(count - target, |from| (from + count - target) % count);
        if next <= prev {
            for _ in 0..next {
                list.dispatch_action(&gpui_base::actions::SelectDown, window, cx);
            }
        } else {
            for _ in 0..prev {
                list.dispatch_action(&gpui_base::actions::SelectUp, window, cx);
            }
        }
        cx.stop_propagation();
    }
}
