//! Single-line native text editing, adapted from GPUI 0.2.2's input example.
//! Adapted from the published GPUI 0.2.2 `examples/input.rs` (Apache-2.0).
//! Modified for Block: styling, safe paint errors, IME offsets, graphemes and horizontal scrolling.
//! The upstream license is preserved in `../GPUI-LICENSE-APACHE`.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InteractiveElement as _,
    IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, ParentElement as _, Pixels, Point, Render, ShapedLine, SharedString,
    StatefulInteractiveElement as _, Style, Styled as _, TextRun, UTF16Selection, UnderlineStyle,
    Window, accesskit, div, fill, point, px, relative, rgb, rgba, size,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FieldSemantics {
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub required: bool,
    pub invalid: bool,
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
    search: bool,
    frozen: bool,
    field_semantics: Option<FieldSemantics>,
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
            search: false,
            frozen: false,
            field_semantics: None,
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
            search: self.search,
            frozen: true,
            field_semantics: self.field_semantics.clone(),
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

    pub(super) fn set_field_semantics(
        &mut self,
        semantics: FieldSemantics,
        cx: &mut Context<'_, Self>,
    ) {
        if self.field_semantics.as_ref() != Some(&semantics) {
            self.field_semantics = Some(semantics);
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

    fn set_accessible_selection(
        &mut self,
        selection: &accesskit::TextSelection,
        run_id: accesskit::NodeId,
        cx: &mut Context<'_, Self>,
    ) {
        if self.is_composing() || selection.anchor.node != run_id || selection.focus.node != run_id
        {
            return;
        }
        let byte_offset = |index| {
            self.content
                .char_indices()
                .nth(index)
                .map(|(offset, _)| offset)
                .or_else(|| (index == self.content.chars().count()).then_some(self.content.len()))
        };
        let (Some(anchor), Some(focus)) = (
            byte_offset(selection.anchor.character_index),
            byte_offset(selection.focus.character_index),
        ) else {
            return;
        };
        let range = anchor.min(focus)..anchor.max(focus);
        let reversed = focus < anchor;
        if self.selected_range != range || self.selection_reversed != reversed {
            self.selected_range = range;
            self.selection_reversed = reversed;
            cx.notify();
        }
    }

    fn replace_accessible_text(
        &mut self,
        text: &str,
        replace_all: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only || (!replace_all && self.is_composing()) {
            return;
        }
        let range = replace_all.then(|| 0..self.offset_to_utf16(self.content.len()));
        self.replace_text_in_range(range, text, window, cx);
    }
}

fn accessible_text_run(
    content: &str,
    selected_range: &Range<usize>,
    reversed: bool,
    run_id: accesskit::NodeId,
) -> (accesskit::Node, accesskit::TextSelection) {
    let mut run = accesskit::Node::new(accesskit::Role::TextRun);
    run.set_value(content.to_owned());
    run.set_character_lengths(
        content
            .chars()
            .map(|character| u8::try_from(character.len_utf8()).unwrap_or(4))
            .collect::<Vec<_>>(),
    );
    let position = |offset| accesskit::TextPosition {
        node: run_id,
        character_index: content[..offset].chars().count(),
    };
    let start = position(selected_range.start);
    let end = position(selected_range.end);
    let (anchor, focus) = if reversed { (end, start) } else { (start, end) };
    (run, accesskit::TextSelection { anchor, focus })
}

fn set_accessible_states(
    node: &mut accesskit::Node,
    read_only: bool,
    invalid: bool,
    required: bool,
) {
    if read_only {
        node.set_read_only();
    }
    if invalid {
        node.set_invalid(accesskit::Invalid::True);
    }
    if required {
        node.set_required();
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
        let viewport_width = bounds.size.width.max(px(0.));
        let max_scroll = (line.width() - viewport_width).max(px(0.));
        let mut scroll_offset = input.scroll_offset.clamp(px(0.), max_scroll);
        if cursor_pos - scroll_offset > viewport_width {
            scroll_offset = (cursor_pos - viewport_width).min(max_scroll);
        } else if cursor_pos < scroll_offset {
            scroll_offset = cursor_pos.max(px(0.));
        }
        // Keep a caret at the right edge inside the viewport without scrolling
        // an otherwise fully visible line out of view.
        let cursor_width = px(2.).min(viewport_width);
        let cursor_x =
            (cursor_pos - scroll_offset).clamp(px(0.), (viewport_width - cursor_width).max(px(0.)));
        let origin = point(bounds.left() - scroll_offset, bounds.top());
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_x, bounds.top()),
                        size(cursor_width, bounds.bottom() - bounds.top()),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let palette = cx
            .try_global::<super::Palette>()
            .copied()
            .unwrap_or(super::Palette::LIGHT);
        let invalid = self
            .field_semantics
            .as_ref()
            .is_some_and(|field| field.invalid);
        let border = if invalid {
            palette.destructive
        } else if self.search {
            palette.secondary
        } else {
            palette.input
        };
        let ring = if invalid {
            palette.destructive
        } else {
            palette.ring
        };
        let label = self
            .field_semantics
            .as_ref()
            .map_or_else(|| self.placeholder.clone(), |field| field.label.clone());
        let description = self
            .field_semantics
            .as_ref()
            .and_then(|field| field.description.clone());
        let required = self
            .field_semantics
            .as_ref()
            .is_some_and(|field| field.required);
        let content = self.content.clone();
        let selection = self.selected_range.clone();
        let reversed = self.selection_reversed;
        let read_only = self.read_only;
        let run_id = Rc::new(Cell::new(None));
        let selection_run_id = run_id.clone();
        let selection_input = cx.entity();
        let mut control = div()
            .id("text-input")
            .aria_label(label)
            .aria_value(self.content.clone())
            .aria_placeholder(self.placeholder.clone())
            .on_a11y_action(accesskit::Action::SetTextSelection, move |data, _, cx| {
                if let (Some(accesskit::ActionData::SetTextSelection(selection)), Some(run_id)) =
                    (data, selection_run_id.get())
                {
                    selection_input.update(cx, |input, cx| {
                        input.set_accessible_selection(selection, run_id, cx);
                    });
                }
            })
            .a11y_synthetic_children(move |builder| {
                let id = builder.synthetic_node_id(0_u8);
                run_id.set(Some(id));
                let (run, selection) = accessible_text_run(&content, &selection, reversed, id);
                set_accessible_states(builder.parent_node(), read_only, invalid, required);
                builder.parent_node().set_text_selection(selection);
                builder.push_child(id, run);
            })
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
            );
        // Outgoing transition copies are visual only. Exposing their inputs
        // would let accessibility actions edit a discarded snapshot.
        if !self.frozen {
            control = control.role(if self.search {
                accesskit::Role::SearchInput
            } else {
                accesskit::Role::TextInput
            });
        }
        if let Some(description) = description {
            control = control.aria_description(description);
        }
        if !self.read_only {
            let set_value_input = cx.entity();
            let replace_input = cx.entity();
            control = control
                .on_a11y_action(accesskit::Action::SetValue, move |data, window, cx| {
                    if let Some(accesskit::ActionData::Value(value)) = data {
                        set_value_input.update(cx, |input, cx| {
                            input.replace_accessible_text(value, true, window, cx);
                        });
                    }
                })
                .on_a11y_action(
                    accesskit::Action::ReplaceSelectedText,
                    move |data, window, cx| {
                        if let Some(accesskit::ActionData::Value(value)) = data {
                            replace_input.update(cx, |input, cx| {
                                input.replace_accessible_text(value, false, window, cx);
                            });
                        }
                    },
                );
        }
        control
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
    use gpui::{
        AppContext as _, Context, Element as _, Entity, EntityInputHandler as _, Focusable as _,
        IntoElement, ParentElement as _, Render, Styled as _, Window, accesskit, div, px,
    };

    struct ResizableInput {
        input: Entity<super::TextInput>,
        width: f32,
    }

    impl Render for ResizableInput {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div().w(px(self.width)).child(self.input.clone())
        }
    }

    struct LabeledInput {
        input: Entity<super::TextInput>,
        plain: bool,
    }

    impl Render for LabeledInput {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            if self.plain {
                crate::components::field("Plain name", &self.input)
            } else {
                crate::components::field_with(
                    "Launch name",
                    &self.input,
                    crate::Palette::LIGHT,
                    crate::components::FieldOptions {
                        hint: Some("Shown in the library.".into()),
                        error: Some("Enter a name.".into()),
                        required: true,
                    },
                )
            }
        }
    }

    fn input_node(
        input: &Entity<super::TextInput>,
        cx: &mut gpui::VisualTestContext,
    ) -> accesskit::Node {
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                let element = input.render(window, cx).into_element();
                let role = element.a11y_role().unwrap_or(accesskit::Role::Unknown);
                let mut node = accesskit::Node::new(role);
                element.write_a11y_info(&mut node);
                node
            })
        })
    }

    #[gpui::test]
    fn rendered_input_exposes_text_role_value_and_supported_actions(cx: &mut gpui::TestAppContext) {
        let (input, cx) =
            cx.add_window_view(|_, cx| super::TextInput::new("trail", "Find a trail", cx));
        let node = input_node(&input, cx);
        assert_eq!(node.role(), accesskit::Role::TextInput);
        assert_eq!(node.label(), Some("Find a trail"));
        assert_eq!(node.value(), Some("trail"));
        assert!(node.supports_action(accesskit::Action::Focus));
        assert!(node.supports_action(accesskit::Action::SetValue));
        assert!(node.supports_action(accesskit::Action::ReplaceSelectedText));
        assert!(node.supports_action(accesskit::Action::SetTextSelection));

        input.update(cx, |input, cx| input.set_search(true, cx));
        assert_eq!(input_node(&input, cx).role(), accesskit::Role::SearchInput);
        input.update(cx, |input, cx| input.set_read_only(true, cx));
        let node = input_node(&input, cx);
        assert!(node.supports_action(accesskit::Action::Focus));
        assert!(node.supports_action(accesskit::Action::SetTextSelection));
        assert!(!node.supports_action(accesskit::Action::SetValue));
        assert!(!node.supports_action(accesskit::Action::ReplaceSelectedText));
    }

    #[gpui::test]
    fn rendered_field_supplies_and_updates_input_semantics(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| LabeledInput {
            input: cx.new(|cx| super::TextInput::new("trail", "Placeholder", cx)),
            plain: false,
        });
        cx.run_until_parked();
        let input = view.read_with(cx, |view, _| view.input.clone());
        let node = input_node(&input, cx);
        assert_eq!(node.label(), Some("Launch name"));
        assert_eq!(
            node.description(),
            Some("Shown in the library. Enter a name.")
        );
        input.read_with(cx, |input, _| {
            let mut node = accesskit::Node::new(accesskit::Role::TextInput);
            let flags = input
                .field_semantics
                .as_ref()
                .map(|field| (field.invalid, field.required));
            assert_eq!(flags, Some((true, true)));
            if let Some((invalid, required)) = flags {
                super::set_accessible_states(&mut node, input.read_only, invalid, required);
            }
            assert_eq!(node.invalid(), Some(accesskit::Invalid::True));
            assert!(node.is_required());
            assert!(!node.is_read_only());
        });

        view.update(cx, |view, cx| {
            view.plain = true;
            cx.notify();
        });
        cx.run_until_parked();
        let node = input_node(&input, cx);
        assert_eq!(node.label(), Some("Plain name"));
        assert_eq!(node.description(), None);
        input.read_with(cx, |input, _| {
            assert!(
                input
                    .field_semantics
                    .as_ref()
                    .is_some_and(|field| !field.invalid && !field.required)
            );
        });
        // Repeated redraws must not alternate the plain and validated states.
        for _ in 0..3 {
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            input.read_with(cx, |input, _| {
                assert!(
                    input
                        .field_semantics
                        .as_ref()
                        .is_some_and(|field| !field.invalid && !field.required)
                );
            });
        }
    }

    #[gpui::test]
    fn accessible_selection_and_edits_use_the_native_editor_state(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            super::bind_keys(cx);
            let input = super::TextInput::new("A🧱界Z", "Name", cx);
            input.focus_handle(cx).focus(window, cx);
            input
        });
        let run_id = accesskit::NodeId(17);
        let position = |character_index| accesskit::TextPosition {
            node: run_id,
            character_index,
        };
        let selection = accesskit::TextSelection {
            anchor: position(3),
            focus: position(1),
        };
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.set_accessible_selection(&selection, run_id, cx);
                let selected = input.selected_text_range(false, window, cx);
                assert!(
                    selected.is_some_and(|selected| selected.range == (1..4) && selected.reversed)
                );
                let (run, exposed) = super::accessible_text_run(
                    &input.content,
                    &input.selected_range,
                    input.selection_reversed,
                    run_id,
                );
                assert_eq!(run.value(), Some("A🧱界Z"));
                assert_eq!(run.character_lengths(), &[1, 4, 3, 1]);
                assert_eq!(exposed, selection);
                let foreign = accesskit::TextSelection {
                    anchor: accesskit::TextPosition {
                        node: accesskit::NodeId(18),
                        character_index: 0,
                    },
                    focus: position(0),
                };
                input.set_accessible_selection(&foreign, run_id, cx);
                assert_eq!(input.selected_range, 1..8);
                input.replace_accessible_text("旅", false, window, cx);
                assert_eq!(input.value(), "A旅Z");
            });
        });
        crate::test_support::press(cx, "cmd-z");
        assert_eq!(input.read_with(cx, |input, _| input.value()), "A🧱界Z");
        crate::test_support::press(cx, "cmd-shift-z");
        assert_eq!(input.read_with(cx, |input, _| input.value()), "A旅Z");
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_accessible_text("New\npath", true, window, cx);
                assert_eq!(input.value(), "New path");
                assert_eq!(input.selected_range, 8..8);
            });
        });
        crate::test_support::press(cx, "cmd-z");
        assert_eq!(input.read_with(cx, |input, _| input.value()), "A旅Z");
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.set_read_only(true, cx);
                let mut node = accesskit::Node::new(accesskit::Role::TextInput);
                super::set_accessible_states(&mut node, input.read_only, false, false);
                assert!(node.is_read_only());
                let selection = accesskit::TextSelection {
                    anchor: position(0),
                    focus: position(1),
                };
                input.set_accessible_selection(&selection, run_id, cx);
                assert_eq!(input.selected_range, 0..1);
                input.replace_accessible_text("changed", false, window, cx);
                input.replace_accessible_text("changed", true, window, cx);
                assert_eq!(input.value(), "A旅Z");
            });
        });
    }

    #[gpui::test]
    fn accessible_selection_does_not_interrupt_ime_composition(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|_, cx| super::TextInput::new("Trail ", "Name", cx));
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "中文", Some(1..2), window, cx);
                let range = input.selected_range.clone();
                let selection = accesskit::TextSelection {
                    anchor: accesskit::TextPosition {
                        node: accesskit::NodeId(1),
                        character_index: 0,
                    },
                    focus: accesskit::TextPosition {
                        node: accesskit::NodeId(1),
                        character_index: 0,
                    },
                };
                input.set_accessible_selection(&selection, accesskit::NodeId(1), cx);
                input.replace_accessible_text("changed", false, window, cx);
                assert_eq!(input.value(), "Trail 中文");
                assert_eq!(input.selected_range, range);
                assert_eq!(input.marked_text_range(window, cx), Some(6..8));
            });
        });
    }

    #[gpui::test]
    fn rendered_input_clamps_scroll_after_narrow_or_zero_width_grows(
        cx: &mut gpui::TestAppContext,
    ) {
        for initial_width in [0., 42.] {
            let (view, cx) = cx.add_window_view(|window, cx| {
                let input = cx.new(|cx| super::TextInput::new("Harbor trail", "Name", cx));
                input.read(cx).focus_handle(cx).focus(window, cx);
                ResizableInput {
                    input,
                    width: initial_width,
                }
            });
            cx.run_until_parked();
            let input = view.read_with(cx, |view, _| view.input.clone());
            input.read_with(cx, |input, _| {
                let geometry = input.last_layout.as_ref().zip(input.last_bounds);
                assert!(geometry.is_some(), "input must be painted");
                if let Some((line, bounds)) = geometry {
                    assert!(line.width() > bounds.size.width);
                    assert!(input.scroll_offset > px(0.));
                }
            });

            view.update(cx, |view, cx| {
                view.width = 400.;
                cx.notify();
            });
            cx.run_until_parked();
            input.read_with(cx, |input, _| {
                let geometry = input.last_layout.as_ref().zip(input.last_bounds);
                assert!(geometry.is_some(), "input must be painted");
                if let Some((line, bounds)) = geometry {
                    assert!(line.width() <= bounds.size.width);
                    assert_eq!(input.scroll_offset, px(0.));
                    assert_eq!(
                        bounds.left() + line.x_for_index(0) - input.scroll_offset,
                        bounds.left()
                    );
                    assert!(
                        bounds.left() + line.x_for_index(input.content.len()) - input.scroll_offset
                            <= bounds.right()
                    );
                }
            });
        }
    }

    #[gpui::test]
    fn rendered_input_keeps_a_long_line_scrolled_to_the_active_caret(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::bind_keys(cx);
            let input = cx.new(|cx| {
                super::TextInput::new("A very long trail through the mountains", "Name", cx)
            });
            input.read(cx).focus_handle(cx).focus(window, cx);
            ResizableInput { input, width: 42. }
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.width = 150.;
            cx.notify();
        });
        cx.run_until_parked();
        let input = view.read_with(cx, |view, _| view.input.clone());
        input.read_with(cx, |input, _| {
            let geometry = input.last_layout.as_ref().zip(input.last_bounds);
            assert!(geometry.is_some(), "input must be painted");
            if let Some((line, bounds)) = geometry {
                assert!(line.width() > bounds.size.width);
                assert!(input.scroll_offset > px(0.));
                assert_eq!(input.scroll_offset, line.width() - bounds.size.width);
                let cursor = line.x_for_index(input.cursor_offset()) - input.scroll_offset;
                assert!(cursor >= px(0.) && cursor <= bounds.size.width);
            }
        });
        crate::test_support::press(cx, "home");
        cx.run_until_parked();
        assert_eq!(input.read_with(cx, |input, _| input.scroll_offset), px(0.));
        crate::test_support::press(cx, "end");
        cx.run_until_parked();
        assert!(input.read_with(cx, |input, _| input.scroll_offset) > px(0.));
    }

    #[gpui::test]
    fn rendered_input_preserves_utf16_selection_and_range_bounds_after_growth(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| super::TextInput::new("路 🧱 harbor trail", "Name", cx));
            input.read(cx).focus_handle(cx).focus(window, cx);
            ResizableInput { input, width: 42. }
        });
        let input = view.read_with(cx, |view, _| view.input.clone());
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.set_selected_text_range(0..4, window, cx);
            });
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.width = 400.;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                assert_eq!(
                    input
                        .selected_text_range(false, window, cx)
                        .map(|selection| selection.range),
                    Some(0..4)
                );
                assert_eq!(input.scroll_offset, px(0.));
                let geometry = input.last_layout.as_ref().zip(input.last_bounds);
                assert!(geometry.is_some(), "input must be painted");
                if let Some((line, bounds)) = geometry {
                    let expected_right = bounds.left() + line.x_for_index("路 🧱".len());
                    let selection_bounds = input.bounds_for_range(0..4, bounds, window, cx);
                    assert!(selection_bounds.is_some_and(|selection_bounds| {
                        selection_bounds.left() == bounds.left()
                            && selection_bounds.right() == expected_right
                    }));
                }
            });
        });
    }

    #[gpui::test]
    fn retained_input_preserves_editability_appearance_and_owns_independent_state(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) =
            cx.add_window_view(|_, cx| super::TextInput::new("中文 draft", "Name", cx));
        let frozen = view.update(cx, |input, cx| cx.new(|cx| input.frozen(cx)));
        assert_eq!(input_node(&view, cx).role(), accesskit::Role::TextInput);
        assert_eq!(input_node(&frozen, cx).role(), accesskit::Role::Unknown);
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
