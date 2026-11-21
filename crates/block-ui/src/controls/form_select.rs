//! Form drafts own the value. The mounted field owns only Kit's interaction state.
use crate::{Block, Palette, design};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, WeakEntity, Window,
    div, px, rgb,
};
use gpui_base::TestTextExt as _;
use gpui_component::{
    IndexPath,
    select::{Select, SelectEvent, SelectItem, SelectState},
};

#[path = "form_select_keys.rs"]
mod keys;

#[path = "form_select_options.rs"]
mod options;
pub(crate) use options::{Options, WindowSize};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher_form)]
pub(super) struct DismissFormSelect;

#[derive(Clone, PartialEq)]
struct Row<V> {
    value: V,
    label: SharedString,
    caption: SharedString,
    index: usize,
}
impl<V: Clone + PartialEq> SelectItem for Row<V> {
    type Value = V;
    fn value(&self) -> &V {
        &self.value
    }
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn display_title(&self) -> Option<gpui::AnyElement> {
        Some(self.caption.clone().into_any_element())
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let index = self.index;
        div()
            .debug_selector(move || format!("select-choice-{index}"))
            .child(self.label.clone().test_text(self.label.clone()))
    }
}
struct Mounted<V: Clone + PartialEq + 'static> {
    picker: Entity<SelectState<Vec<Row<V>>>>,
    rows: Vec<Row<V>>,
    trigger: FocusHandle,
    prefix: String,
    typed_at: Option<std::time::Instant>,
    keys: Option<gpui::Subscription>,
}
#[derive(IntoElement)]
struct Field<V: Clone + PartialEq + 'static> {
    id: &'static str,
    label: SharedString,
    caption: SharedString,
    palette: Palette,
    options: Options<V>,
    owner: WeakEntity<Block>,
    disabled: bool,
}
impl<V: Clone + PartialEq + 'static> RenderOnce for Field<V> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            label,
            caption,
            palette,
            options,
            owner,
            disabled,
        } = self;
        let rows: Vec<_> = options
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, (value, label))| Row {
                value,
                label: label.into(),
                caption: caption.clone(),
                index,
            })
            .collect();
        let selected = rows
            .iter()
            .position(|r| Some(&r.value) == options.selected.as_ref())
            .map(IndexPath::new);
        let mounted = window.use_keyed_state((id, usize::from(disabled)), cx, |window, cx| {
            let picker = cx.new(|cx| SelectState::new(rows.clone(), selected, window, cx));
            let trigger = picker.focus_handle(cx);
            cx.subscribe_in(
                &picker,
                window,
                move |_, _, event: &SelectEvent<Vec<Row<V>>>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        _ = owner.update(cx, |view, cx| {
                            view.handle((options.confirm)(value.clone()), window, cx)
                        });
                    }
                },
            )
            .detach();

            cx.observe_in(
                &picker,
                window,
                |field: &mut Mounted<V>, picker, window, cx| {
                    let list = picker.focus_handle(cx);
                    if list == field.trigger {
                        field.keys = None;
                        field.prefix.clear();
                    } else if field.keys.is_none() {
                        let mounted = cx.weak_entity();
                        field.keys = Some(cx.intercept_keystrokes(move |event, window, cx| {
                            _ = mounted.update(cx, |field, cx| field.key(event, window, cx));
                        }));
                        if picker.read(cx).selected_index(cx).is_none() {
                            // The list needs its first layout before it can move its cursor.
                            window.on_next_frame(move |window, cx| {
                                if list.is_focused(window)
                                    && picker.read(cx).selected_index(cx).is_none()
                                {
                                    list.dispatch_action(
                                        &gpui_base::actions::SelectDown,
                                        window,
                                        cx,
                                    );
                                }
                            });
                        }
                    }
                },
            )
            .detach();
            Mounted {
                picker,
                rows: rows.clone(),
                trigger,
                prefix: String::new(),
                typed_at: None,
                keys: None,
            }
        });
        let picker = mounted.update(cx, |field, cx| {
            if field.rows != rows {
                field
                    .picker
                    .update(cx, |p, cx| p.set_items(rows.clone(), window, cx));
                field.rows = rows;
            }
            let value = selected.map(|ix| &field.rows[ix.row].value);
            if field.picker.read(cx).selected_value() != value {
                field
                    .picker
                    .update(cx, |p, cx| p.set_selected_index(selected, window, cx));
            }
            field.picker.clone()
        });
        let dismiss = move |window: &mut Window, cx: &mut App| {
            let field = mounted.read(cx);
            if field.picker.focus_handle(cx) == field.trigger {
                return false;
            }
            // Cancel at the trigger: Kit restores it synchronously, before a
            // dirty-exit dialog can take focus. List cancellation is deferred.
            let trigger = field.trigger.clone();
            trigger.dispatch_action(&gpui_base::actions::Cancel, window, cx);
            true
        };
        div()
            .id(id)
            .debug_selector(move || id.into())
            .on_key_down({
                let dismiss = dismiss.clone();
                move |event, window, cx| {
                    if crate::settings::is_back_key(event) && dismiss(window, cx) {
                        cx.stop_propagation();
                    }
                }
            })
            .on_action(move |_: &DismissFormSelect, window, cx| {
                dismiss(window, cx);
            })
            .w_full()
            .h(px(design::CONTROL_HEIGHT))
            .child(
                Select::new(&picker)
                    .id(format!("{id}-select"))
                    .accessibility_label(label)
                    .placeholder(caption)
                    .disabled(disabled)
                    .w_full()
                    .h(px(design::CONTROL_HEIGHT))
                    .rounded(px(design::CONTROL_RADIUS))
                    .border_color(rgb(palette.input))
                    .bg(rgb(palette.background))
                    .text_color(rgb(palette.foreground))
                    .text_size(px(design::BODY_FONT_SIZE)),
            )
    }
}
impl Block {
    pub(crate) fn product_select_field<V: Clone + PartialEq + 'static>(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        caption: &str,
        options: Options<V>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let label = label.into();
        let disabled = options.rows.first().is_none_or(|(value, _)| {
            self.busy && !Self::available_while_busy(&(options.confirm)(value.clone()))
        });
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child(label.clone()))
            .child(Field {
                id,
                label,
                caption: caption.to_owned().into(),
                options,
                palette: self.palette,
                owner: cx.weak_entity(),
                disabled,
            })
    }
}
