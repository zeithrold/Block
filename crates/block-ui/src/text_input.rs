//! Single-line native text editing, adapted from GPUI 0.2.2's input example.
//! Adapted from the published GPUI 0.2.2 `examples/input.rs` (Apache-2.0).
//! Modified for Block: styling, safe paint errors, IME offsets, graphemes and horizontal scrolling.
//! The upstream license is preserved in `../GPUI-LICENSE-APACHE`.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InteractiveElement as _,
    IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, ParentElement as _, Pixels, Point, Render, ShapedLine, SharedString, Style,
    Styled as _, TextRun, UTF16Selection, UnderlineStyle, Window, div, fill, point, px, relative,
    rgb, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation as _;

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Backspace;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Delete;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Left;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Right;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct SelectLeft;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct SelectRight;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct SelectAll;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Home;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct End;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct ShowCharacterPalette;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Paste;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Cut;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Copy;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Undo;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = text_input)]
struct Redo;

#[derive(Clone, Debug)]
struct EditSnapshot {
    content: SharedString,
    selection: Range<usize>,
    reversed: bool,
}

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Selection, composition and native accessibility flags are independent input states."
)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "This private module exposes items only to its parent view."
)]
pub(crate) struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    scroll_offset: Pixels,
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    composition_before: Option<EditSnapshot>,
    read_only: bool,
    accessible_label: String,
    accessible_description: String,
    invalid: bool,
    required: bool,
    search: bool,
}

impl TextInput {
    pub(super) fn set_placeholder(&mut self, placeholder: &str, cx: &mut Context<'_, Self>) {
        if self.placeholder.as_ref() != placeholder {
            self.placeholder = placeholder.to_owned().into();
            cx.notify();
        }
    }

    pub(super) fn new(value: &str, placeholder: &str, cx: &Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            content: value.to_owned().into(),
            placeholder: placeholder.to_owned().into(),
            selected_range: value.len()..value.len(),
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            scroll_offset: px(0.),
            undo: Vec::new(),
            redo: Vec::new(),
            composition_before: None,
            read_only: false,
            accessible_label: placeholder.to_owned(),
            accessible_description: String::new(),
            invalid: false,
            required: false,
            search: false,
        }
    }

    pub(super) const fn is_composing(&self) -> bool {
        self.marked_range.is_some()
    }

    /// An independent visual copy for an inert outgoing transition. The live
    /// editor keeps its IME session, undo history and focus. The inert transition
    /// scope isolates this copy while preserving the editor's original appearance.
    pub(super) fn frozen(&self, cx: &Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: self.content.clone(),
            placeholder: self.placeholder.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
            marked_range: self.marked_range.clone(),
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            scroll_offset: self.scroll_offset,
            undo: Vec::new(),
            redo: Vec::new(),
            composition_before: None,
            read_only: self.read_only,
            accessible_label: self.accessible_label.clone(),
            accessible_description: self.accessible_description.clone(),
            invalid: self.invalid,
            required: self.required,
            search: self.search,
        }
    }

    pub(super) fn set_read_only(&mut self, read_only: bool, cx: &mut Context<'_, Self>) {
        if self.read_only != read_only {
            self.read_only = read_only;
            cx.notify();
        }
    }

    pub(super) fn set_search(&mut self, search: bool, cx: &mut Context<'_, Self>) {
        if self.search != search {
            self.search = search;
            cx.notify();
        }
    }

    pub(super) fn set_accessible_label(&mut self, label: &str, cx: &mut Context<'_, Self>) {
        if self.accessible_label != label {
            label.clone_into(&mut self.accessible_label);
            cx.notify();
        }
    }

    pub(super) fn set_accessible_details(
        &mut self,
        description: &str,
        invalid: bool,
        required: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if self.accessible_description != description
            || self.invalid != invalid
            || self.required != required
        {
            description.clone_into(&mut self.accessible_description);
            self.invalid = invalid;
            self.required = required;
            cx.notify();
        }
    }

    pub(super) fn value(&self) -> String {
        self.content.to_string()
    }

    pub(super) fn set_value(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        self.content = value.to_owned().into();
        self.selected_range = value.len()..value.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.composition_before = None;
        self.undo.clear();
        self.redo.clear();
        self.scroll_offset = px(0.);
        cx.notify();
    }

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            content: self.content.clone(),
            selection: self.selected_range.clone(),
            reversed: self.selection_reversed,
        }
    }

    fn remember(&mut self, previous: EditSnapshot) {
        if previous.content != self.content {
            if self.undo.len() == 100 {
                self.undo.remove(0);
            }
            self.undo.push(previous);
            self.redo.clear();
        }
    }

    fn restore(&mut self, snapshot: EditSnapshot, cx: &mut Context<'_, Self>) {
        self.content = snapshot.content;
        self.selected_range = snapshot.selection;
        self.selection_reversed = snapshot.reversed;
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.read_only
            && self.marked_range.is_none()
            && let Some(previous) = self.undo.pop()
        {
            self.redo.push(self.snapshot());
            self.restore(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.read_only
            && self.marked_range.is_none()
            && let Some(next) = self.redo.pop()
        {
            self.undo.push(self.snapshot());
            self.restore(next, cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<'_, Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<'_, Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    const fn on_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        _: &mut Context<'_, Self>,
    ) {
        self.is_selecting = false;
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        _this: &mut Self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<'_, Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.read_only {
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<'_, Self>) {
        self.selected_range = offset..offset;
        cx.notify();
    }

    const fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_offset)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<'_, Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) {
        self.marked_range = None;
        if let Some(previous) = self.composition_before.take() {
            self.remember(previous);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only {
            return;
        }
        let previous = self
            .composition_before
            .take()
            .unwrap_or_else(|| self.snapshot());
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let new_text = new_text.replace(['\n', '\r'], " ");
        self.content =
            (self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.selection_reversed = false;
        self.remember(previous);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.composition_before.is_none() {
            self.composition_before = Some(self.snapshot());
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let new_text = new_text.replace(['\n', '\r'], " ");
        self.content =
            (self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..])
                .into();
        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            self.marked_range = Some(range.start..range.start + new_text.len());
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| {
                utf16_to_utf8(&new_text, range_utf16.start)
                    ..utf16_to_utf8(&new_text, range_utf16.end)
            })
            .map_or_else(
                || range.start + new_text.len()..range.start + new_text.len(),
                |new_range| new_range.start + range.start..new_range.end + range.start,
            );

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(range.start) - self.scroll_offset,
                bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(range.end) - self.scroll_offset,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;

        if last_layout.text != self.content {
            return None;
        }
        let utf8_index = last_layout.index_for_x(line_point.x + self.scroll_offset)?;
        Some(self.offset_to_utf16(utf8_index))
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.is_composing() {
            return;
        }
        self.selected_range = self.range_from_utf16(&range_utf16);
        self.selection_reversed = false;
        cx.notify();
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.content.len()))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> bool {
        !self.read_only
    }
}

#[derive(Debug)]
struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    origin: Point<Pixels>,
    scroll_offset: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), rgb(0x007f_91a8).into())
        } else {
            (content, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let cursor_pos = line.x_for_index(cursor);
        let mut scroll_offset = input.scroll_offset;
        if cursor_pos - scroll_offset > bounds.size.width - px(2.) {
            scroll_offset = (cursor_pos - bounds.size.width + px(2.)).max(px(0.));
        } else if cursor_pos < scroll_offset {
            scroll_offset = cursor_pos;
        }
        let origin = point(bounds.left() - scroll_offset, bounds.top());
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(origin.x + cursor_pos, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    rgb(0x0083_d8b4),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            origin.x + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            origin.x + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x62bd_a855),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            origin,
            scroll_offset,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        let Some(line) = prepaint.line.take() else {
            return;
        };
        let _ = line.paint(
            prepaint.origin,
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.scroll_offset = prepaint.scroll_offset;
        });
    }
}

impl Render for TextInput {
    #[expect(
        clippy::too_many_lines,
        reason = "The native editor keeps its semantic, keyboard, IME and pointer contracts on one element."
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut semantic = gpui::accesskit::Node::new(gpui::accesskit::Role::TextInput);
        semantic.set_label(self.accessible_label.clone());
        semantic.set_description(self.accessible_description.clone());
        semantic.set_value(self.content.to_string());
        semantic.add_action(gpui::accesskit::Action::Focus);
        if self.read_only {
            semantic.set_read_only();
        } else {
            semantic.add_action(gpui::accesskit::Action::SetValue);
            semantic.add_action(gpui::accesskit::Action::ReplaceSelectedText);
        }
        if self.invalid {
            semantic.set_invalid(gpui::accesskit::Invalid::True);
        }
        if self.required {
            semantic.set_required();
        }
        let palette = cx
            .try_global::<super::Palette>()
            .copied()
            .unwrap_or(super::Palette::LIGHT);
        let border = if self.invalid {
            palette.destructive
        } else if self.search {
            palette.secondary
        } else {
            palette.input
        };
        let ring = if self.invalid {
            palette.destructive
        } else {
            palette.ring
        };
        div()
            .id("text-input")
            .accessibility(
                semantic,
                self.focus_handle.is_focused(window),
                cx.listener(
                    |input, request: &gpui::accesskit::ActionRequest, window, cx| {
                        if input.read_only || input.is_composing() {
                            return;
                        }
                        if let Some(gpui::accesskit::ActionData::Value(value)) = &request.data {
                            match request.action {
                                gpui::accesskit::Action::SetValue => {
                                    let length = input.offset_to_utf16(input.content.len());
                                    input.replace_text_in_range(Some(0..length), value, window, cx);
                                }
                                gpui::accesskit::Action::ReplaceSelectedText => {
                                    input.replace_text_in_range(None, value, window, cx);
                                }
                                _ => {}
                            }
                        }
                    },
                ),
            )
            .flex()
            .key_context("TextInput")
            .items_center()
            .gap(px(8.))
            .track_focus(&self.focus_handle(cx))
            .tab_index(0)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(|input, _: &gpui_base::actions::Cancel, _, cx| {
                if !input.is_composing() {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|input, _: &gpui_base::actions::Confirm, _, cx| {
                    if !input.is_composing() {
                        cx.propagate();
                    }
                }),
            )
            .on_key_down(cx.listener(|input, event: &gpui::KeyDownEvent, _, cx| {
                // App-level Enter/Escape must not submit or dismiss a surface
                // while the input method owns an unfinished composition.
                if input.marked_range.is_some()
                    && matches!(
                        event.keystroke.key.as_str(),
                        "enter" | "escape" | "up" | "down"
                    )
                {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .h(px(super::design::CONTROL_HEIGHT))
            .px(px(10.))
            .py(px(5.))
            .line_height(px(super::design::BODY_LINE_HEIGHT))
            .text_size(px(super::design::BODY_FONT_SIZE))
            .rounded(px(super::design::CONTROL_RADIUS))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(if self.search {
                palette.secondary
            } else {
                palette.input_background
            }))
            .opacity(if self.read_only { 0.65 } else { 1. })
            .focus(move |style| {
                style
                    .border_color(rgb(ring))
                    .shadow(super::design::focus_ring(ring))
            })
            .children(
                self.search
                    .then(|| super::design::Icon::Search.render(14., palette.muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(super::design::BODY_LINE_HEIGHT))
                    .child(TextElement { input: cx.entity() }),
            )
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "This private module exposes items only to its parent view."
)]
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("TextInput")),
        KeyBinding::new("delete", Delete, Some("TextInput")),
        KeyBinding::new("left", Left, Some("TextInput")),
        KeyBinding::new("right", Right, Some("TextInput")),
        KeyBinding::new("shift-left", SelectLeft, Some("TextInput")),
        KeyBinding::new("shift-right", SelectRight, Some("TextInput")),
        KeyBinding::new("home", Home, Some("TextInput")),
        KeyBinding::new("end", End, Some("TextInput")),
        KeyBinding::new("cmd-a", SelectAll, Some("TextInput")),
        KeyBinding::new("ctrl-a", SelectAll, Some("TextInput")),
        KeyBinding::new("cmd-v", Paste, Some("TextInput")),
        KeyBinding::new("ctrl-v", Paste, Some("TextInput")),
        KeyBinding::new("cmd-c", Copy, Some("TextInput")),
        KeyBinding::new("ctrl-c", Copy, Some("TextInput")),
        KeyBinding::new("cmd-x", Cut, Some("TextInput")),
        KeyBinding::new("ctrl-x", Cut, Some("TextInput")),
        KeyBinding::new("cmd-z", Undo, Some("TextInput")),
        KeyBinding::new("ctrl-z", Undo, Some("TextInput")),
        KeyBinding::new("cmd-shift-z", Redo, Some("TextInput")),
        KeyBinding::new("ctrl-shift-z", Redo, Some("TextInput")),
        KeyBinding::new("ctrl-y", Redo, Some("TextInput")),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("TextInput")),
    ]);
}

fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    let mut utf16 = 0;
    for (index, ch) in text.char_indices() {
        if utf16 >= offset {
            return index;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use gpui::{EntityInputHandler as _, Focusable as _};

    #[gpui::test]
    fn retained_input_preserves_editability_appearance_and_owns_independent_state(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) =
            cx.add_window_view(|_, cx| super::TextInput::new("中文 draft", "Name", cx));
        for read_only in [false, true] {
            view.update(cx, |input, cx| {
                input.set_read_only(read_only, cx);
                let frozen = input.frozen(cx);
                assert_eq!(frozen.read_only, input.read_only);
                assert_eq!(frozen.content, input.content);
                assert_ne!(frozen.focus_handle, input.focus_handle);
                assert!(frozen.undo.is_empty() && frozen.redo.is_empty());
                input.set_value("Live draft changed", cx);
                assert_ne!(frozen.content, input.content);
                input.set_value("中文 draft", cx);
            });
        }
    }

    #[gpui::test]
    fn composition_commits_as_one_undo_unit_and_new_edits_clear_redo(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::bind_keys(cx);
            let input = super::TextInput::new("World ", "Text", cx);
            input.focus_handle(cx).focus(window, cx);
            input
        });
        cx.update(|window, cx| {
            view.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "zhong", None, window, cx);
                input.replace_and_mark_text_in_range(None, "中文🧱", None, window, cx);
                input.replace_text_in_range(None, "中文🧱", window, cx);
            });
        });
        crate::test_support::press(cx, "cmd-z");
        assert_eq!(view.read_with(cx, |input, _| input.value()), "World ");
        crate::test_support::press(cx, "cmd-shift-z");
        assert_eq!(view.read_with(cx, |input, _| input.value()), "World 中文🧱");
        crate::test_support::press(cx, "cmd-z");
        cx.simulate_input("新世界");
        crate::test_support::press(cx, "cmd-shift-z");
        assert_eq!(view.read_with(cx, |input, _| input.value()), "World 新世界");
    }

    #[gpui::test]
    fn input_backspace_deletes_a_grapheme_and_ime_selection_is_relative_to_inserted_text(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::bind_keys(cx);
            let input = super::TextInput::new("prefix ", "Text", cx);
            input.focus_handle(cx).focus(window, cx);
            input
        });
        cx.simulate_input("世界 🧱");
        crate::test_support::press(cx, "backspace");
        assert_eq!(view.read_with(cx, |input, _| input.value()), "prefix 世界 ");
        cx.update(|window, cx| {
            view.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "🧱界", Some(2..3), window, cx);
                assert_eq!(
                    input
                        .selected_text_range(false, window, cx)
                        .map(|s| s.range),
                    Some(12..13)
                );
                assert_eq!(input.marked_text_range(window, cx), Some(10..13));
            });
        });
    }
}
