//! Native controls shared by product pages. Events are supplied by callers; this
//! module has no launcher commands, persistence, or prototype dependencies.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Shared controls remain scoped to the private UI module."
)]

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use gpui::{
    App, CursorStyle, Div, Entity, Focusable as _, FontWeight, Image, ImageFormat,
    InteractiveElement as _, ObjectFit, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img, px, relative,
    rgb,
};
use unicode_segmentation::UnicodeSegmentation as _;

pub(super) use super::base_controls::Control;

use super::{
    appearance::Palette,
    design::{
        BODY_FONT_SIZE, BODY_LINE_HEIGHT, CARD_RADIUS, CONTROL_HEIGHT, CONTROL_RADIUS, FIELD_GAP,
        Icon, SMALL_FONT_SIZE, focus_ring,
    },
    text_input::{FieldSemantics, TextInput},
};

/// Informational wizard progress. Completion comes from the caller's real state;
/// the rail itself never navigates or adds keyboard tab stops.
#[derive(Clone, Copy)]
pub(super) struct StepperStep<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub complete: bool,
}

pub(super) fn stepper(
    id: impl Into<SharedString>,
    _label: &str,
    steps: &[StepperStep<'_>],
    current: usize,
    palette: Palette,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .role(gpui::Role::List)
        .min_w_0()
        .flex()
        .flex_col()
        .gap_5()
        .children(steps.iter().enumerate().map(|(index, step)| {
            let active = index == current;
            let foreground = if active {
                palette.on_primary
            } else {
                palette.muted
            };
            div()
                .id(("step", index))
                .role(gpui::Role::ListItem)
                .min_w_0()
                .flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .size(px(28.))
                        .flex_shrink_0()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(if active {
                            palette.primary
                        } else {
                            palette.border
                        }))
                        .bg(rgb(if active {
                            palette.primary
                        } else {
                            palette.background
                        }))
                        .text_color(rgb(foreground))
                        .text_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(step.complete.then(|| Icon::Check.render(14., foreground)))
                        .children((!step.complete).then(|| (index + 1).to_string())),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_sm()
                        .text_color(rgb(if active {
                            palette.foreground
                        } else {
                            palette.muted
                        }))
                        .child(step.title.to_owned())
                        .children((!step.description.is_empty()).then(|| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(step.description.to_owned())
                        })),
                )
        }))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ControlState {
    pub disabled: bool,
    pub busy: bool,
    pub invalid: bool,
}

impl ControlState {
    pub(super) const fn interactive(self) -> bool {
        !self.disabled && !self.busy
    }
}

/// The primitive owns activation/focus/AX. This helper adds only Block's busy
/// and invalid presentation and consumes a disabled pointer press at its root.
fn control_state(control: Control, state: ControlState) -> Control {
    let mut control = control;
    if state.interactive() {
        control = control.cursor_pointer();
    } else {
        control = control
            .cursor(CursorStyle::OperationNotAllowed)
            .opacity(0.5)
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
    }
    control
}

fn interactive_control(
    id: impl Into<SharedString>,
    label: SharedString,
    state: ControlState,
    role: gpui::Role,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    let id = id.into();
    let selector = id.to_string();
    control_state(
        Control::button(
            id,
            label,
            role,
            !state.interactive(),
            move |_, window, cx| {
                on_activate(&(), window, cx);
            },
        ),
        state,
    )
    .debug_selector(move || selector)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ButtonVariant {
    Primary,
    #[default]
    Secondary,
    Outline,
    Ghost,
    Destructive,
    Link,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ButtonSize {
    ExtraSmall,
    #[default]
    Medium,
    Icon,
}

impl ButtonSize {
    const fn height(self) -> f32 {
        match self {
            Self::ExtraSmall => 24.,
            Self::Medium | Self::Icon => CONTROL_HEIGHT,
        }
    }
}

pub(super) fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    palette: Palette,
    variant: ButtonVariant,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    button_sized(
        id,
        label,
        palette,
        variant,
        ButtonSize::Medium,
        state,
        on_activate,
    )
}

pub(super) fn button_sized(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    palette: Palette,
    variant: ButtonVariant,
    size: ButtonSize,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    styled_control(
        id,
        label,
        palette,
        (variant, gpui::accesskit::Role::Button, None),
        size,
        state,
        on_activate,
    )
}

/// Keep visual variants and activation gates shared while exposing native roles.
pub(super) fn button_role(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    palette: Palette,
    variant: ButtonVariant,
    state: ControlState,
    role: gpui::accesskit::Role,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    styled_control(
        id,
        label,
        palette,
        (variant, role, None),
        ButtonSize::Medium,
        state,
        on_activate,
    )
}

fn styled_control(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    palette: Palette,
    style: (ButtonVariant, gpui::accesskit::Role, Option<bool>),
    size: ButtonSize,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    let (variant, role, selected) = style;
    let (background, foreground, border, hover) = match variant {
        ButtonVariant::Primary => (
            palette.primary,
            palette.on_primary,
            palette.primary,
            mix(palette.primary, palette.background, 80),
        ),
        ButtonVariant::Secondary => (
            palette.secondary,
            palette.foreground,
            palette.secondary,
            mix(palette.secondary, palette.foreground, 95),
        ),
        ButtonVariant::Outline => (
            palette.background,
            palette.foreground,
            palette.border,
            palette.accent,
        ),
        ButtonVariant::Ghost | ButtonVariant::Link => (
            palette.background,
            palette.foreground,
            palette.background,
            palette.accent,
        ),
        ButtonVariant::Destructive => (
            palette.destructive_background,
            palette.destructive,
            palette.destructive_background,
            mix(palette.destructive_background, palette.destructive, 90),
        ),
    };
    let focus = if state.invalid {
        palette.destructive
    } else {
        palette.ring
    };
    let label = label.into();
    let role = if variant == ButtonVariant::Link {
        gpui::Role::Link
    } else {
        role
    };
    let control = interactive_control(id, label.clone(), state, role, on_activate);
    let _ = selected;
    let mut control = control
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .gap(px(6.))
        .h(px(size.height()))
        .px(px(10.))
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(rgb(if state.invalid { focus } else { border }))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .text_size(px(if size == ButtonSize::ExtraSmall {
            SMALL_FONT_SIZE
        } else {
            BODY_FONT_SIZE
        }))
        .line_height(px(BODY_LINE_HEIGHT))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .focus_visible(move |style| style.border_color(rgb(focus)).shadow(focus_ring(focus)));
    if state.interactive() {
        control = control
            .hover(move |style| {
                let style = style.bg(rgb(hover));
                if variant == ButtonVariant::Link {
                    style.underline()
                } else {
                    style
                }
            })
            .active(move |style| {
                let style = style.bg(rgb(hover)).opacity(0.85);
                // Preserve an absolute consumer's positioning and the surrounding
                // flow. Changing `position` here would move a pressed action out
                // of its hit target; paired margins keep its occupied size.
                if role == gpui::accesskit::Role::Button {
                    style.mt(px(1.)).mb(px(-1.))
                } else {
                    style
                }
            });
    }
    if size == ButtonSize::Icon {
        control = control.w(px(size.height())).px_0();
    }
    if state.busy {
        control = control.child(Icon::Loading.render(14., foreground));
    }
    if !label.is_empty() {
        control = control.child(label);
    }
    control
}

/// A real listbox option shares the button event gate and visible focus.
pub(super) fn select_option(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    palette: Palette,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    styled_control(
        id,
        label,
        palette,
        (
            ButtonVariant::Ghost,
            gpui::accesskit::Role::ListBoxOption,
            Some(selected),
        ),
        ButtonSize::Medium,
        state,
        on_activate,
    )
    .w_full()
    .justify_between()
    .children(selected.then(|| Icon::Check.render(14., palette.foreground)))
}

/// Selects use the same activation gate and focus appearance as other controls.
pub(super) fn select_trigger(
    id: impl Into<SharedString>,
    value: impl Into<SharedString>,
    palette: Palette,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    button_role(
        id,
        "",
        palette,
        ButtonVariant::Outline,
        state,
        gpui::accesskit::Role::ComboBox,
        on_activate,
    )
    .w_full()
    .min_w_0()
    .justify_between()
    .child(div().flex_1().min_w_0().text_ellipsis().child(value.into()))
    .child(Icon::ChevronDown.render(14., palette.muted))
}

/// Percentage is integer-weighted to avoid alpha changes affecting text as well.
fn mix(first: u32, second: u32, first_percent: u32) -> u32 {
    let mut result = 0;
    for shift in [0, 8, 16] {
        let channel = (((first >> shift) & 255) * first_percent
            + ((second >> shift) & 255) * (100 - first_percent)
            + 50)
            / 100;
        result |= channel << shift;
    }
    result
}

#[derive(Clone, Debug, Default)]
pub(super) struct FieldOptions {
    pub hint: Option<SharedString>,
    pub error: Option<SharedString>,
    pub required: bool,
}

/// Preserve the existing input entity and its IME, selection, and clipboard path.
pub(super) fn field(label: impl Into<SharedString>, input: &Entity<TextInput>) -> Div {
    labeled_field(label.into(), input, &FieldOptions::default())
}

fn labeled_field(label: SharedString, input: &Entity<TextInput>, options: &FieldOptions) -> Div {
    let description = [options.hint.as_ref(), options.error.as_ref()]
        .into_iter()
        .flatten()
        .map(AsRef::<str>::as_ref)
        .collect::<Vec<_>>()
        .join(" ");
    let semantics = FieldSemantics {
        label: label.clone(),
        description: (!description.is_empty()).then(|| description.into()),
        required: options.required,
        invalid: options.error.is_some(),
    };
    let label = if options.required {
        format!("{label} *").into()
    } else {
        label
    };
    let input_focus = input.clone();
    let semantic_input = input.clone();
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(FIELD_GAP))
        .child(
            div()
                .id(SharedString::from(format!(
                    "field-label-{}",
                    input.entity_id()
                )))
                .text_size(px(BODY_FONT_SIZE))
                .line_height(px(BODY_LINE_HEIGHT))
                .font_weight(FontWeight::MEDIUM)
                .cursor(CursorStyle::IBeam)
                .on_click(move |_, window, cx| {
                    input_focus.read(cx).focus_handle(cx).focus(window, cx);
                    cx.stop_propagation();
                })
                .child(label),
        )
        .child(input.clone())
        .child(
            gpui::canvas(
                move |_, _, cx| {
                    semantic_input.update(cx, |input, cx| {
                        input.set_field_semantics(semantics, cx);
                    });
                },
                |_, (), _, _| {},
            )
            .absolute()
            .inset_0(),
        )
}

pub(super) fn field_with(
    label: impl Into<SharedString>,
    input: &Entity<TextInput>,
    palette: Palette,
    options: FieldOptions,
) -> Div {
    let mut field =
        labeled_field(label.into(), input, &options).text_color(rgb(palette.foreground));
    if let Some(hint) = options.hint {
        field = field.child(
            div()
                .text_size(px(SMALL_FONT_SIZE))
                .line_height(px(18.))
                .text_color(rgb(palette.muted))
                .child(hint),
        );
    }
    if let Some(error) = options.error {
        field = field.child(
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .text_size(px(SMALL_FONT_SIZE))
                .line_height(px(18.))
                .text_color(rgb(palette.destructive))
                .child(Icon::Error.render(14., palette.destructive))
                .child(error),
        );
    }
    field
}

pub(super) fn card(palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .overflow_hidden()
        .gap(px(16.))
        .p(px(16.))
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.card))
        .text_color(rgb(palette.foreground))
        .text_size(px(BODY_FONT_SIZE))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum BadgeVariant {
    #[default]
    Secondary,
    Outline,
    Destructive,
    Info,
    Success,
    Warning,
}

/// Human memory label: whole GiB stays whole, otherwise one decimal place.
pub(super) fn memory_label(mib: u32) -> String {
    if mib.is_multiple_of(1024) {
        format!("{} GB", mib / 1024)
    } else {
        format!("{:.1} GB", f64::from(mib) / 1024.0)
    }
}

/// Editable GB text for a stored MiB value. Whole GB stays whole; finer values
/// already present in a catalog keep up to three decimal places.
pub(super) fn memory_gb_text(mib: u32) -> String {
    if mib.is_multiple_of(1024) {
        return (mib / 1024).to_string();
    }
    let mut text = format!("{:.3}", f64::from(mib) / 1024.0);
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Parse the editable GB field into MiB and enforce the product range.
pub(super) fn parse_memory_gb(value: &str) -> Result<u32, String> {
    let text = value.trim();
    let message = "Choose a memory limit between 1 and 32 GB.".to_owned();
    let gb = text.parse::<f64>().map_err(|_| message.clone())?;
    if !gb.is_finite() {
        return Err(message);
    }
    let mib = (gb * 1024.0).round();
    if !(f64::from(block_launcher::MIN_MEMORY_MIB)..=f64::from(block_launcher::MAX_MEMORY_MIB))
        .contains(&mib)
    {
        return Err(message);
    }
    if (mib / 1024.0 - gb).abs() > 0.000_5 {
        return Err("Enter a memory limit in GB with up to three decimal places.".into());
    }
    Ok(mib as u32)
}

pub(super) fn badge(
    label: impl Into<SharedString>,
    palette: Palette,
    variant: BadgeVariant,
) -> Div {
    let (background, foreground, border) = match variant {
        BadgeVariant::Secondary => (palette.secondary, palette.foreground, palette.secondary),
        BadgeVariant::Outline => (palette.background, palette.foreground, palette.border),
        BadgeVariant::Destructive => (
            palette.destructive_background,
            palette.destructive,
            palette.destructive_background,
        ),
        BadgeVariant::Info => (
            palette.info_background,
            palette.info,
            palette.info_background,
        ),
        BadgeVariant::Success => (
            palette.success_background,
            palette.success,
            palette.success_background,
        ),
        BadgeVariant::Warning => (
            palette.warning_background,
            palette.warning,
            palette.warning_background,
        ),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .self_center()
        .h(px(20.))
        .px(px(8.))
        .rounded_full()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .text_size(px(SMALL_FONT_SIZE))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(label.into())
}

pub(super) fn checkbox(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
    palette: Palette,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    choice(id, label, checked, false, palette, state, on_activate)
}

pub(super) fn radio(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
    palette: Palette,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    choice(id, label, checked, true, palette, state, on_activate)
}

/// Choice presentation stays with each consumer; keyboard and focus stay here.
#[derive(Clone)]
pub(super) struct RadioOption<A> {
    pub id: String,
    pub checked: bool,
    pub disabled: bool,
    pub action: A,
}

type RadioCallback<A> = Rc<dyn Fn(&A, &mut Window, &mut App)>;

pub(super) struct RadioGroup<A> {
    id: SharedString,
    options: Rc<Vec<RadioOption<A>>>,
    focus: Rc<Vec<gpui::FocusHandle>>,
    active: Option<usize>,
    on_choose: RadioCallback<A>,
    scroll: Option<gpui::ScrollHandle>,
}

impl<A: Clone + 'static> RadioGroup<A> {
    pub(super) fn new<T: 'static>(
        id: impl Into<SharedString>,
        _label: impl Into<SharedString>,
        options: Vec<RadioOption<A>>,
        cache: &RefCell<HashMap<String, gpui::FocusHandle>>,
        cx: &gpui::Context<'_, T>,
        on_choose: impl Fn(&A, &mut Window, &mut App) + 'static,
    ) -> Self {
        let id = id.into();
        let active = options
            .iter()
            .position(|item| item.checked && !item.disabled)
            .or_else(|| options.iter().position(|item| !item.disabled));
        let focus = options
            .iter()
            .enumerate()
            .map(|(index, item)| {
                cache
                    .borrow_mut()
                    .entry(format!("{id}:{}", item.id))
                    .or_insert_with(|| cx.focus_handle())
                    .clone()
                    .tab_stop(Some(index) == active)
                    .tab_index(if Some(index) == active { 0 } else { -1 })
            })
            .collect();
        Self {
            id,
            options: Rc::new(options),
            focus: Rc::new(focus),
            active,
            on_choose: Rc::new(on_choose),
            scroll: None,
        }
    }

    pub(super) fn with_scroll(mut self, scroll: gpui::ScrollHandle) -> Self {
        self.scroll = Some(scroll);
        self
    }

    pub(super) fn container(&self) -> gpui_base::RadioGroup {
        gpui_base::RadioGroup::new(self.id.clone()).min_w_0()
    }

    pub(super) fn item(&self, id: &str, control: Control) -> Control {
        let Some(index) = self.options.iter().position(|item| item.id == id) else {
            return control;
        };
        if self.options[index].disabled {
            return control;
        }
        let options = self.options.clone();
        let handles = self.focus.clone();
        let active = Some(index) == self.active;
        let on_choose = self.on_choose.clone();
        let scroll = self.scroll.clone();
        control
            .track_focus(&self.focus[index])
            .tab_stop(active)
            .tab_index(if active { 0 } else { -1 })
            .on_key_down(move |event, window, cx| {
                if event.keystroke.modifiers != gpui::Modifiers::none() {
                    return;
                }
                let available = options
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| (!item.disabled).then_some(index))
                    .collect::<Vec<_>>();
                let Some(position) = available.iter().position(|candidate| *candidate == index)
                else {
                    return;
                };
                let next = match event.keystroke.key.as_str() {
                    "left" | "up" => available[(position + available.len() - 1) % available.len()],
                    "right" | "down" => available[(position + 1) % available.len()],
                    "home" => available[0],
                    "end" => available[available.len() - 1],
                    _ => return,
                };
                if !options[next].checked {
                    on_choose(&options[next].action, window, cx);
                }
                handles[next].focus(window, cx);
                if let Some(scroll) = &scroll {
                    scroll.scroll_to_item(next);
                }
                cx.stop_propagation();
            })
    }
}

fn choice(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
    round: bool,
    palette: Palette,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    let color = if state.invalid {
        palette.destructive
    } else {
        palette.ring
    };
    let mut indicator = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(16.))
        .flex_shrink_0()
        .rounded(px(if round { 8. } else { 4. }))
        .border_1()
        .border_color(rgb(if checked {
            palette.primary
        } else {
            palette.input
        }))
        .bg(rgb(if checked {
            palette.primary
        } else {
            palette.input_background
        }));
    if checked {
        indicator = if round {
            indicator.child(
                div()
                    .size(px(7.))
                    .rounded_full()
                    .bg(rgb(palette.on_primary)),
            )
        } else {
            indicator.child(Icon::Check.render(14., palette.on_primary))
        };
    }
    let label = label.into();
    let id = id.into();
    let selector = id.to_string();
    let control = if round {
        Control::radio(
            id,
            label.clone(),
            checked,
            !state.interactive(),
            on_activate,
        )
    } else {
        Control::checkbox(
            id,
            label.clone(),
            checked,
            !state.interactive(),
            on_activate,
        )
    };
    control_state(control, state)
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .min_h(px(CONTROL_HEIGHT))
        .gap(px(8.))
        .rounded(px(4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .text_size(px(BODY_FONT_SIZE))
        .line_height(px(BODY_LINE_HEIGHT))
        .text_color(rgb(palette.foreground))
        .focus_visible(move |style| style.border_color(rgb(color)).shadow(focus_ring(color)))
        .child(indicator)
        .child(label)
}

/// Initials use complete graphemes, including non-Latin names and emoji.
fn initials(name: &str) -> SharedString {
    let value = name
        .split_whitespace()
        .filter_map(|part| part.graphemes(true).next())
        .take(2)
        .collect::<String>();
    if value.is_empty() {
        "?".into()
    } else {
        value.to_uppercase().into()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum AvatarState {
    Loading,
    #[default]
    Fallback,
}

/// Fallback text and spinner follow the avatar box instead of a fixed cap, so a
/// 24 px list avatar and an 80 px detail avatar both stay legible.
fn fallback_text_size(size: f32) -> f32 {
    (size * 0.36).clamp(10., 24.)
}

fn fallback_icon_size(size: f32) -> f32 {
    (size * 0.4).clamp(12., 28.)
}

pub(super) fn avatar_fallback(name: &str, palette: Palette, size: f32, state: AvatarState) -> Div {
    let avatar = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.secondary))
        .text_color(rgb(palette.muted))
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(fallback_text_size(size)));
    match state {
        // A skin that is still loading is not unavailable: show progress rather
        // than initials that would look like the final identity.
        AvatarState::Loading => {
            avatar.child(Icon::Loading.render(fallback_icon_size(size), palette.muted))
        }
        AvatarState::Fallback => avatar.child(initials(name)),
    }
}

/// A real account skin, cached in memory as the source PNG the renderer draws.
#[derive(Clone)]
pub(super) struct AccountAvatar {
    pub image: Arc<Image>,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for AccountAvatar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountAvatar")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// A validated provider icon from the provider's own declared URL.
#[derive(Clone)]
pub(super) struct ProviderBadge {
    pub image: Arc<Image>,
}

impl std::fmt::Debug for ProviderBadge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderBadge")
            .finish_non_exhaustive()
    }
}

/// The head's front face on the 64-unit skin grid.
const SKIN_FACE: (f32, f32) = (8., 8.);
/// The head's outer layer, drawn over the face. Classic 64×32 skins carry it in
/// the same place as the modern 64×64 grid.
const SKIN_HAT: (f32, f32) = (40., 8.);
/// One head region is 8 units square, so exactly one region fills the avatar.
const HEAD_UNITS: f32 = 8.;
/// Avatar border width. A crop sized for the border box would sit one pixel low
/// and lose the far edge, so region edits use the inset content box.
const AVATAR_BORDER: f32 = 1.;

/// Where the whole skin image sits inside a square content box so that one
/// 8-unit head region covers it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkinCrop {
    /// Scaled size of the complete skin image.
    image: (f32, f32),
    /// Image offset that brings the face region to the content box's top-left.
    face: (f32, f32),
    /// Image offset that brings the hat region to the same corner.
    hat: (f32, f32),
}

fn skin_crop(width: u32, height: u32, inner: f32) -> SkinCrop {
    let units = inner / HEAD_UNITS;
    let factor = width as f32 / 64.;
    let scale = units / factor;
    let offset = |region: (f32, f32)| (-(region.0 * factor * scale), -(region.1 * factor * scale));
    SkinCrop {
        image: (width as f32 * scale, height as f32 * scale),
        face: offset(SKIN_FACE),
        hat: offset(SKIN_HAT),
    }
}

/// Draw the face plus hat layer from the real skin, or the initials fallback.
///
/// The whole validated skin is drawn once per layer and the avatar's clipped
/// 8-unit window selects the region, so classic, 64×32 and HD grids share one
/// rule and no image service is involved.
pub(super) fn account_avatar(
    name: &str,
    avatar: Option<&AccountAvatar>,
    palette: Palette,
    size: f32,
    state: AvatarState,
) -> Div {
    let Some(avatar) = avatar.filter(|_| state != AvatarState::Loading) else {
        return avatar_fallback(name, palette, size, state);
    };
    let inner = (size - 2. * AVATAR_BORDER).max(1.);
    let crop = skin_crop(avatar.width, avatar.height, inner);
    let layer = |offset: (f32, f32), selector: &'static str| {
        div()
            .absolute()
            .left(px(offset.0))
            .top(px(offset.1))
            .size(px(inner))
            .child(
                img(avatar.image.clone())
                    .debug_selector(move || selector.to_owned())
                    .w(px(crop.image.0))
                    .h(px(crop.image.1)),
            )
    };
    div()
        .size(px(size))
        .flex_shrink_0()
        .relative()
        .overflow_hidden()
        .rounded_full()
        .border(px(AVATAR_BORDER))
        .border_color(rgb(palette.border))
        .bg(rgb(palette.secondary))
        .child(layer(crop.face, "account-avatar-face"))
        .child(layer(crop.hat, "account-avatar-hat"))
}

/// Map a validated provider-icon kind to the renderer's image format.
pub(super) fn icon_image_format(kind: block_launcher::avatars::ImageKind) -> ImageFormat {
    use block_launcher::avatars::ImageKind;
    match kind {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Gif => ImageFormat::Gif,
        ImageKind::Webp => ImageFormat::Webp,
        ImageKind::Bmp => ImageFormat::Bmp,
        ImageKind::Svg => ImageFormat::Svg,
    }
}

/// Draw the provider's real icon, or a monogram with its initials.
pub(super) fn provider_icon(
    name: &str,
    icon: Option<&ProviderBadge>,
    palette: Palette,
    size: f32,
) -> Div {
    let base = div()
        .size(px(size))
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(if size > 24. { 6. } else { 4. }))
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.secondary))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(palette.muted))
        .font_weight(FontWeight::MEDIUM)
        .text_size(px((size * 0.42).clamp(10., 16.)));
    match icon {
        // Fill the border box's content area: a `size`-square image would sit one
        // pixel low and be clipped by the 1 px border on the far edges.
        Some(icon) => base.child(
            img(icon.image.clone())
                .debug_selector(|| "provider-icon-image".into())
                .size_full()
                .object_fit(ObjectFit::Cover),
        ),
        None => base.child(initials(name)),
    }
}

pub(super) fn profile_avatar(
    appearance: block_launcher::catalog::ProfileAppearance,
    palette: Palette,
    size: f32,
) -> Div {
    use block_launcher::catalog::{ProfileColor, ProfileIcon};
    let icon = match appearance.icon {
        ProfileIcon::Blocks => Icon::Blocks,
        ProfileIcon::Layers => Icon::Layers,
        ProfileIcon::Pickaxe => Icon::Pickaxe,
        ProfileIcon::Sword => Icon::Sword,
        ProfileIcon::Compass => Icon::Compass,
        ProfileIcon::Hammer => Icon::Hammer,
        ProfileIcon::Leaf => Icon::Leaf,
        ProfileIcon::FlaskConical => Icon::Experiment,
    };
    let dark = palette.background == Palette::DARK.background;
    let color = match appearance.color {
        ProfileColor::Gray => palette.muted,
        ProfileColor::Blue => {
            if dark {
                0x0060_a5fa
            } else {
                0x0025_63eb
            }
        }
        ProfileColor::Green => {
            if dark {
                0x004a_de80
            } else {
                0x0016_a34a
            }
        }
        ProfileColor::Orange => {
            if dark {
                0x00fb_923c
            } else {
                0x00ea_580c
            }
        }
        ProfileColor::Purple => {
            if dark {
                0x00c0_84fc
            } else {
                0x0093_33ea
            }
        }
        ProfileColor::Pink => {
            if dark {
                0x00f4_72b6
            } else {
                0x00db_2777
            }
        }
    };
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(CONTROL_RADIUS))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(mix(color, palette.background, 12)))
        .child(icon.render(size * 0.5, color))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProgressState {
    Indeterminate,
    Measured { completed: u64, total: u64 },
}

impl ProgressState {
    pub(super) const fn from_counts(completed: u64, total: Option<u64>) -> Self {
        match total {
            Some(total) if total > 0 => Self::Measured { completed, total },
            _ => Self::Indeterminate,
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "A visual ratio need not retain integer precision."
    )]
    pub(super) fn fraction(self) -> Option<f32> {
        match self {
            Self::Measured { completed, total } if total > 0 => {
                Some(completed.min(total) as f32 / total as f32)
            }
            Self::Indeterminate | Self::Measured { .. } => None,
        }
    }
}

/// No timer-generated percentage. Unknown totals have a quiet track and an
/// explicit label instead of pretending that a partial-width bar is measured.
#[expect(
    clippy::cast_precision_loss,
    reason = "Native progress numeric values are doubles; the displayed byte counts retain integer precision."
)]
pub(super) fn progress(state: ProgressState, palette: Palette) -> Stateful<Div> {
    let mut track = div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(palette.secondary));
    if let Some(fraction) = state.fraction() {
        track = track.child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(rgb(palette.primary)),
        );
    }
    let mut progress = div().flex().flex_col().gap(px(8.)).child(track);
    if state.fraction().is_none() {
        progress = progress.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(palette.muted))
                .child(Icon::Loading.render(12., palette.muted))
                .child("In progress"),
        );
    }
    progress.id("progress").role(gpui::Role::ProgressIndicator)
}

/// A compact companion to the same measured/unknown progress contract. The
/// open ring means unknown total; it never invents a percentage or clock value.
pub(super) fn progress_ring(state: ProgressState, palette: Palette) -> Div {
    let fraction = state.fraction();
    div().size(px(20.)).flex_shrink_0().child(
        gpui::canvas(
            |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                let center = bounds.center();
                let radius = px(8.);
                for (portion, color) in [
                    (1., palette.border),
                    (fraction.unwrap_or(0.25), palette.foreground),
                ] {
                    let mut path = gpui::PathBuilder::stroke(px(2.));
                    for index in 0_u16..=64 {
                        let angle = -std::f32::consts::FRAC_PI_2
                            + std::f32::consts::TAU * portion * f32::from(index) / 64.;
                        let position =
                            center + gpui::point(radius * angle.cos(), radius * angle.sin());
                        if index == 0 {
                            path.move_to(position);
                        } else {
                            path.line_to(position);
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgb(color));
                    }
                }
            },
        )
        .size_full(),
    )
}

#[cfg(test)]
mod tests {
    use super::{ButtonVariant, ControlState, ProgressState, button, field, initials};
    use crate::{appearance::Palette, text_input::TextInput};
    use gpui::{
        AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
        IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ParentElement as _, Render,
        StatefulInteractiveElement as _, Styled as _, Window, div, point, px,
    };

    fn press(cx: &mut gpui::VisualTestContext, key: &str) {
        let parsed = Keystroke::parse(key);
        assert!(parsed.is_ok(), "invalid test keystroke: {key}");
        if let Ok(keystroke) = parsed {
            cx.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(KeyUpEvent { keystroke });
        }
    }

    #[test]
    fn unknown_download_totals_never_claim_a_percentage() {
        assert_eq!(ProgressState::from_counts(40, None).fraction(), None);
        assert_eq!(ProgressState::from_counts(40, Some(0)).fraction(), None);
        assert_eq!(
            ProgressState::from_counts(40, Some(80)).fraction(),
            Some(0.5)
        );
        assert_eq!(
            ProgressState::from_counts(100, Some(80)).fraction(),
            Some(1.)
        );
    }

    #[test]
    fn skin_crop_geometry_covers_classic_and_hd_skins() {
        // The face and hat offsets are measured on the 64-unit grid, so an
        // 8-unit head region must fill a 40 px content box whatever the source
        // resolution.
        for (width, height) in [(64, 64), (64, 32), (128, 128), (128, 64)] {
            let crop = super::skin_crop(width, height, 40.);
            assert_eq!(crop.face, (-40., -40.), "{width}×{height} face");
            assert_eq!(crop.hat, (-200., -40.), "{width}×{height} hat");
            assert_eq!(
                crop.image.0, 320.,
                "{width}×{height} scaled skin width: 8 avatar widths"
            );
            assert_eq!(
                crop.image.1,
                320. * height as f32 / width as f32,
                "{width}×{height} scaled skin height"
            );
        }
        // The hat region is inside every accepted grid, including 64×32.
        for (width, height) in [(64, 32), (64, 64)] {
            let crop = super::skin_crop(width, height, 40.);
            assert!(-crop.hat.1 + 40. <= crop.image.1, "{width}×{height} hat");
        }
    }

    #[test]
    fn avatar_fallback_text_and_spinner_follow_the_avatar_box() {
        assert!((super::fallback_text_size(24.) - 10.).abs() < f32::EPSILON);
        assert!((super::fallback_text_size(40.) - 14.4).abs() < 1e-4);
        assert!((super::fallback_text_size(80.) - 24.).abs() < f32::EPSILON);
        assert!((super::fallback_icon_size(32.) - 12.8).abs() < 1e-4);
        assert!((super::fallback_icon_size(80.) - 28.).abs() < 1e-4);
    }

    struct AvatarFixture {
        skin: Option<(u32, u32)>,
    }

    impl Render for AvatarFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            // Layout is what the crop rule controls here. The bytes stay empty
            // because the renderer owns decoding, not the geometry assertions.
            let avatar = self.skin.map(|(width, height)| super::AccountAvatar {
                image: std::sync::Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    Vec::new(),
                )),
                width,
                height,
            });
            div()
                .id("avatar-host")
                .debug_selector(|| "avatar-host".into())
                .w(px(40.))
                .h(px(40.))
                .child(super::account_avatar(
                    "Test Player",
                    avatar.as_ref(),
                    Palette::LIGHT,
                    40.,
                    super::AvatarState::Fallback,
                ))
        }
    }

    #[gpui::test]
    fn account_avatar_places_the_face_and_hat_crop_inside_its_bounds(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_, cx) = crate::test_support::window(cx, |_, _| AvatarFixture {
            skin: Some((64, 64)),
        });
        cx.run_until_parked();
        let host = cx.debug_bounds("avatar-host");
        let face = cx.debug_bounds("account-avatar-face");
        let hat = cx.debug_bounds("account-avatar-hat");
        assert!(host.is_some() && face.is_some() && hat.is_some());
        if let (Some(host), Some(face), Some(hat)) = (host, face, hat) {
            assert_eq!(host.size.width, px(40.));
            // The avatar's 1 px border insets the crop: the drawn skin is eight
            // content widths across and shifted so its 8-unit face region fills
            // the content box, and the hat region starts four widths further
            // left because it begins at x = 40 of 64 units.
            let inner = 38.;
            assert_eq!(face.size.width, px(inner * 8.));
            assert_eq!(face.origin.x, host.origin.x + px(1.) - px(inner));
            assert_eq!(face.origin.y, host.origin.y + px(1.) - px(inner));
            assert_eq!(hat.origin.x, host.origin.x + px(1.) - px(inner * 5.));
            assert_eq!(hat.origin.y, host.origin.y + px(1.) - px(inner));
        }
    }

    #[gpui::test]
    fn account_avatar_uses_initial_fallback_without_a_skin(cx: &mut gpui::TestAppContext) {
        let (_, cx) = crate::test_support::window(cx, |_, _| AvatarFixture { skin: None });
        cx.run_until_parked();
        assert!(cx.debug_bounds("avatar-host").is_some());
        assert!(cx.debug_bounds("account-avatar-face").is_none());
        assert!(cx.debug_bounds("account-avatar-hat").is_none());
    }

    struct ProviderIconFixture;

    impl Render for ProviderIconFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            let badge = super::ProviderBadge {
                image: std::sync::Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    Vec::new(),
                )),
            };
            div()
                .id("provider-icon-host")
                .debug_selector(|| "provider-icon-host".into())
                .w(px(24.))
                .h(px(24.))
                .child(super::provider_icon(
                    "Community server",
                    Some(&badge),
                    Palette::LIGHT,
                    24.,
                ))
        }
    }

    #[gpui::test]
    fn provider_icon_image_fills_the_bordered_content_box(cx: &mut gpui::TestAppContext) {
        let (_, cx) = crate::test_support::window(cx, |_, _| ProviderIconFixture);
        cx.run_until_parked();
        let host = cx.debug_bounds("provider-icon-host");
        let image = cx.debug_bounds("provider-icon-image");
        assert!(host.is_some() && image.is_some());
        if let (Some(host), Some(image)) = (host, image) {
            // The 1 px border insets the image instead of clipping its far edges.
            assert_eq!(image.origin.x, host.origin.x + px(1.));
            assert_eq!(image.origin.y, host.origin.y + px(1.));
            assert_eq!(image.size.width, px(22.));
            assert_eq!(image.size.height, px(22.));
        }
    }

    #[test]
    fn avatar_fallback_keeps_extended_graphemes_and_handles_empty_names() {
        assert_eq!(initials("  ").as_ref(), "?");
        assert_eq!(initials("Ada Lovelace").as_ref(), "AL");
        assert_eq!(initials("👩🏽‍💻 开发者").as_ref(), "👩🏽‍💻开");
        assert_eq!(initials("e\u{301} artist").as_ref(), "E\u{301}A");
    }

    struct ControlFixture {
        focus: FocusHandle,
        activations: usize,
        ancestor_activations: usize,
        state: ControlState,
        input: Entity<TextInput>,
    }

    struct CardFixture {
        activations: usize,
    }
    impl Render for CardFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div().p_4().child(
                super::card(Palette::LIGHT)
                    .id("clip-card")
                    .debug_selector(|| "clip-card".into())
                    .relative()
                    .w(px(160.))
                    .h(px(80.))
                    .p_0()
                    .child(
                        button(
                            "overflow-action",
                            "Action",
                            Palette::LIGHT,
                            ButtonVariant::Primary,
                            ControlState::default(),
                            cx.listener(|view, &(): &(), _, _| view.activations += 1),
                        )
                        .absolute()
                        .left(px(140.))
                        .top(px(20.))
                        .w(px(80.)),
                    ),
            )
        }
    }
    #[gpui::test]
    fn card_clipping_prevents_overflowing_children_from_receiving_outside_clicks(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |_, _| CardFixture { activations: 0 });
        cx.run_until_parked();
        let card = cx.debug_bounds("clip-card");
        let action = cx.debug_bounds("overflow-action");
        assert!(card.is_some() && action.is_some());
        if let (Some(card), Some(action)) = (card, action) {
            cx.simulate_click(
                point(card.right() + px(8.), action.center().y),
                Modifiers::none(),
            );
            assert_eq!(view.read_with(cx, |view, _| view.activations), 0);
            cx.simulate_click(
                point(card.right() - px(5.), action.center().y),
                Modifiers::none(),
            );
            assert_eq!(
                view.read_with(cx, |view, _| view.activations),
                1,
                "card={card:?}; action={action:?}; after={:?}",
                cx.debug_bounds("overflow-action")
            );
        }
    }

    impl Render for ControlFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .id("control-fixture")
                .flex()
                .flex_col()
                .gap_4()
                .p_4()
                .on_click(cx.listener(|view, _, _, _| view.ancestor_activations += 1))
                .child(
                    button(
                        "control-button",
                        "Save",
                        Palette::LIGHT,
                        ButtonVariant::Primary,
                        self.state,
                        cx.listener(|view, &(): &(), _, _| view.activations += 1),
                    )
                    .track_focus(&self.focus),
                )
                .child(
                    field("Name", &self.input)
                        .id("control-field")
                        .debug_selector(|| "control-field".into()),
                )
        }
    }

    #[gpui::test]
    fn native_button_mouse_and_keyboard_share_a_gate_and_do_not_bubble(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            ControlFixture {
                focus,
                activations: 0,
                ancestor_activations: 0,
                state: ControlState::default(),
                input: cx.new(|cx| TextInput::new("", "Name", cx)),
            }
        });
        press(cx, "enter");
        press(cx, "space");
        assert_eq!(view.read_with(cx, |view, _| view.activations), 2);
        press(cx, "cmd-enter");
        press(cx, "alt-space");
        assert_eq!(view.read_with(cx, |view, _| view.activations), 2);
        let bounds = cx.debug_bounds("control-button");
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Modifiers::none());
            let pressed = cx.debug_bounds("control-button");
            assert!(pressed.is_some_and(|pressed| pressed.top() == bounds.top() + px(1.)));
            cx.simulate_mouse_up(bounds.center(), gpui::MouseButton::Left, Modifiers::none());
            assert!(
                cx.debug_bounds("control-button")
                    .is_some_and(|released| released.top() == bounds.top())
            );
        }
        assert_eq!(view.read_with(cx, |view, _| view.activations), 3);
        assert_eq!(view.read_with(cx, |view, _| view.ancestor_activations), 0);

        for state in [
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            ControlState {
                busy: true,
                ..ControlState::default()
            },
        ] {
            view.update(cx, |view, cx| {
                view.state = state;
                cx.notify();
            });
            cx.run_until_parked();
            press(cx, "enter");
            press(cx, "space");
            if let Some(bounds) = cx.debug_bounds("control-button") {
                let window_count = cx.windows().len();
                cx.simulate_click(bounds.center(), Modifiers::none());
                cx.run_until_parked();
                if cx.windows().len() > window_count
                    && let Some(window) = cx.windows().last()
                {
                    *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
                }
                crate::test_support::ensure_live_window(cx);
            }
            assert_eq!(view.read_with(cx, |view, _| view.activations), 3);
            assert_eq!(view.read_with(cx, |view, _| view.ancestor_activations), 0);
        }
    }

    #[gpui::test]
    fn clicking_a_field_label_focuses_the_existing_native_input(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            ControlFixture {
                focus,
                activations: 0,
                ancestor_activations: 0,
                state: ControlState::default(),
                input: cx.new(|cx| TextInput::new("中文 👩🏽‍💻", "Name", cx)),
            }
        });
        let bounds = cx.debug_bounds("control-field");
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            cx.simulate_click(
                point(bounds.left() + px(12.), bounds.top() + px(8.)),
                Modifiers::none(),
            );
        }
        cx.update(|window, cx| {
            assert!(
                view.read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert_eq!(view.read(cx).input.read(cx).value(), "中文 👩🏽‍💻");
        });
    }
}

#[cfg(test)]
#[path = "radio_group_tests.rs"]
mod radio_group_tests;

#[cfg(test)]
mod memory_tests {
    use super::{memory_gb_text, parse_memory_gb};

    #[test]
    fn memory_bounds_migrate_to_the_prototype_one_to_thirty_two_gb_range() {
        assert_eq!(parse_memory_gb("1"), Ok(1024));
        assert_eq!(parse_memory_gb("32"), Ok(32768));
        assert_eq!(parse_memory_gb("1.5"), Ok(1536));
        assert_eq!(parse_memory_gb("2.5"), Ok(2560));
        assert_eq!(
            parse_memory_gb("1023.999"),
            Err("Choose a memory limit between 1 and 32 GB.".into())
        );
        assert_eq!(
            parse_memory_gb("0.25"),
            Err("Choose a memory limit between 1 and 32 GB.".into())
        );
        assert_eq!(
            parse_memory_gb("33"),
            Err("Choose a memory limit between 1 and 32 GB.".into())
        );
        assert_eq!(
            parse_memory_gb("not-a-number"),
            Err("Choose a memory limit between 1 and 32 GB.".into())
        );
        assert_eq!(memory_gb_text(1024), "1");
        assert_eq!(memory_gb_text(1536), "1.5");
        assert_eq!(memory_gb_text(2560), "2.5");
        assert_eq!(memory_gb_text(32768), "32");
    }
}
