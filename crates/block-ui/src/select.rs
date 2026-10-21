//! Product bindings for the shared anchored selector surface.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Selectors are internal to the product shell."
)]
use super::{Block, Form, Tab, UiAction, design::Icon, shell::Surface};
use block_launcher::catalog::{DataPolicy, ProfileColor, ProfileIcon};
use gpui::{
    App, Bounds, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, Point, RenderOnce, Styled as _, Window, div, point, px, rgb,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProductChoice {
    Appearance,
    Proxy,
    DefaultJava,
    DefaultWindow,
    ProfileRuntime,
    ProfileDataPolicy,
    InstanceRuntime,
    InstanceWindow,
    WizardRuntime,
    WizardWindow,
}

/// Presentation only: Base Select is the sole semantic/focus/keyboard root.
#[derive(IntoElement)]
struct ProductSelectTrigger {
    base: gpui_base::Select,
    focus: FocusHandle,
    palette: super::Palette,
    disabled: bool,
}

impl RenderOnce for ProductSelectTrigger {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let focused = !self.disabled && self.focus.is_focused(window);
        self.base
            .w_full()
            .min_w_0()
            .h(px(super::design::CONTROL_HEIGHT))
            .rounded(px(super::design::CONTROL_RADIUS))
            .border_1()
            .border_color(rgb(if focused {
                self.palette.ring
            } else {
                self.palette.input
            }))
            .bg(rgb(self.palette.background))
            .text_color(rgb(self.palette.foreground))
            .text_size(px(super::design::BODY_FONT_SIZE))
            .shadow(if focused {
                super::design::focus_ring(self.palette.ring)
            } else {
                Vec::new()
            })
            .opacity(if self.disabled { 0.5 } else { 1. })
    }
}

#[derive(Clone, Debug)]
pub(super) struct SelectChoice<A> {
    pub label: String,
    pub selected: bool,
    pub action: A,
}

#[derive(Clone, Debug)]
pub(super) struct SelectState<A> {
    pub title: String,
    pub choices: Vec<SelectChoice<A>>,
    pub anchor: Point<Pixels>,
    pub width: Pixels,
    pub menu: Option<MenuKind>,
    pub active: usize,
    pub focus: FocusHandle,
    pub product: Option<ProductChoice>,
    identity: SelectIdentity,
    typeahead: String,
    typed_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuKind {
    ProfileActions,
    App,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SelectIdentity {
    Product(ProductChoice),
    Appearance { wizard: bool, color: bool },
    ProfileActions(String),
    AppMenu,
}

impl SelectIdentity {
    const fn action(&self) -> UiAction {
        match self {
            Self::Product(kind) => UiAction::OpenProductSelect(*kind),
            Self::Appearance { wizard, color } => UiAction::ChooseAppearance {
                wizard: *wizard,
                color: *color,
            },
            Self::ProfileActions(_) => UiAction::ProfileActions,
            Self::AppMenu => UiAction::Drawer(Some(super::Drawer::Menu)),
        }
    }
}

#[derive(Clone, Copy)]
struct AppMenuItem {
    id: &'static str,
    label: &'static str,
    tab: Tab,
    icon: Icon,
}
const APP_MENU: [AppMenuItem; 6] = [
    AppMenuItem {
        id: "menu-home",
        label: "Home",
        tab: Tab::Home,
        icon: Icon::Home,
    },
    AppMenuItem {
        id: "menu-accounts",
        label: "Accounts",
        tab: Tab::Accounts,
        icon: Icon::Account,
    },
    AppMenuItem {
        id: "menu-games",
        label: "Games",
        tab: Tab::Instances,
        icon: Icon::Games,
    },
    AppMenuItem {
        id: "menu-settings",
        label: "Settings",
        tab: Tab::Settings,
        icon: Icon::Settings,
    },
    AppMenuItem {
        id: "menu-welcome",
        label: "Welcome guide",
        tab: Tab::Onboarding,
        icon: Icon::Welcome,
    },
    AppMenuItem {
        id: "menu-about",
        label: "About",
        tab: Tab::About,
        icon: Icon::Info,
    },
];

impl<A> SelectState<A> {
    fn typeahead(&mut self, text: &str, now: Instant) -> Option<usize> {
        if self
            .typed_at
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_millis(700))
        {
            self.typeahead.clear();
        }
        self.typed_at = Some(now);
        let text = text.to_lowercase();
        let repeated =
            !self.typeahead.is_empty() && self.typeahead.graphemes(true).all(|part| part == text);
        if !repeated {
            self.typeahead.push_str(&text);
        }
        let start = if self.typeahead == text {
            self.active + 1
        } else {
            self.active
        };
        (0..self.choices.len())
            .map(|offset| (start + offset) % self.choices.len())
            .find(|index| {
                self.choices[*index]
                    .label
                    .to_lowercase()
                    .starts_with(&self.typeahead)
            })
    }
}

impl Block {
    fn can_open_select(&self, identity: &SelectIdentity) -> bool {
        (!self.busy || Self::available_while_busy(&identity.action()))
            && !self.shell.surfaces.layers().any(|layer| {
                matches!(&layer.kind, Surface::Select(state) if &state.identity == identity)
            })
    }

    pub(super) fn open_product_select(
        &mut self,
        kind: ProductChoice,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_product_select_from_trigger(kind, anchor, None, window, cx);
    }

    fn open_product_select_from_trigger(
        &mut self,
        kind: ProductChoice,
        anchor: Option<Bounds<Pixels>>,
        trigger: Option<&FocusHandle>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // An accessibility client can issue a second action before the next
        // semantic frame. The already-open product selector owns that request.
        let identity = SelectIdentity::Product(kind);
        if !self.can_open_select(&identity) {
            return;
        }
        let Some((title, choices)) = self.product_choices(kind, cx) else {
            return;
        };
        // Accessible activation need not focus the native Base trigger. Capture
        // that real control as the restoration target only after rejecting
        // stale busy/duplicate requests, which must not steal popup focus.
        if let Some(trigger) = trigger {
            trigger.focus(window, cx);
        }
        self.open_select_at(title, choices, identity, anchor, window, cx);
    }

    fn product_choices(
        &self,
        kind: ProductChoice,
        cx: &Context<'_, Self>,
    ) -> Option<(&'static str, Vec<SelectChoice<UiAction>>)> {
        match kind {
            ProductChoice::ProfileRuntime => {
                let Some(Form::Profile(form)) = &self.form else {
                    return None;
                };
                Some((
                    "Java runtime",
                    runtime_choices(form.managed, UiAction::ProfileRuntimeAutomatic),
                ))
            }
            ProductChoice::ProfileDataPolicy => {
                let Some(Form::Profile(form)) = &self.form else {
                    return None;
                };
                Some((
                    "Game data",
                    [
                        ("Isolated profile directory", DataPolicy::Isolated),
                        ("Shared Instance directory", DataPolicy::Shared),
                    ]
                    .into_iter()
                    .map(|(label, value)| SelectChoice {
                        label: label.into(),
                        selected: form.original.data_policy == value,
                        action: UiAction::ProfileDataPolicy(value),
                    })
                    .collect(),
                ))
            }
            ProductChoice::InstanceRuntime => Some((
                "Java runtime",
                runtime_choices(
                    self.instance_runtime_automatic()?,
                    UiAction::InstanceRuntimeAutomatic,
                ),
            )),
            ProductChoice::InstanceWindow => {
                let (width, height, custom) = self.instance_window_values(cx)?;
                Some((
                    "Window size",
                    window_choices(
                        &width,
                        &height,
                        custom,
                        |width, height| UiAction::InstanceWindow { width, height },
                        UiAction::CustomInstanceWindow,
                    ),
                ))
            }
            ProductChoice::WizardRuntime => {
                let profile = self.shell.wizard.profile.as_ref()?;
                Some((
                    "Java runtime",
                    runtime_choices(profile.managed, UiAction::WizardRuntime),
                ))
            }
            ProductChoice::WizardWindow => {
                let profile = self.shell.wizard.profile.as_ref()?;
                Some((
                    "Window size",
                    window_choices(
                        &profile.width.read(cx).value(),
                        &profile.height.read(cx).value(),
                        profile.window_custom,
                        |width, height| UiAction::WizardWindow { width, height },
                        UiAction::WizardCustomWindow,
                    ),
                ))
            }
            _ => self.settings_choices(kind, cx),
        }
    }

    fn settings_choices(
        &self,
        kind: ProductChoice,
        cx: &Context<'_, Self>,
    ) -> Option<(&'static str, Vec<SelectChoice<UiAction>>)> {
        let draft = self.settings_form.as_ref()?;
        Some(match kind {
            ProductChoice::Appearance => (
                "Appearance",
                [
                    ("Follow system", super::Appearance::System),
                    ("Light", super::Appearance::Light),
                    ("Dark", super::Appearance::Dark),
                ]
                .into_iter()
                .map(|(label, value)| SelectChoice {
                    label: label.into(),
                    selected: draft.appearance == value,
                    action: UiAction::SetAppearance(value),
                })
                .collect(),
            ),
            ProductChoice::Proxy => (
                "Proxy",
                [
                    ("Use system proxy", 0),
                    ("Direct connection", 1),
                    ("Manual HTTP / HTTPS proxy", 2),
                ]
                .into_iter()
                .map(|(label, mode)| SelectChoice {
                    label: label.into(),
                    selected: draft.proxy_mode == mode,
                    action: UiAction::ProxyMode(mode),
                })
                .collect(),
            ),
            ProductChoice::DefaultJava => (
                "Default Java runtime",
                runtime_choices(draft.java_automatic, UiAction::DefaultJavaAutomatic),
            ),
            ProductChoice::DefaultWindow => (
                "Default window size",
                window_choices(
                    &draft.width.read(cx).value(),
                    &draft.height.read(cx).value(),
                    draft.window_custom,
                    |width, height| UiAction::DefaultWindow { width, height },
                    UiAction::CustomDefaultWindow,
                ),
            ),
            ProductChoice::ProfileRuntime
            | ProductChoice::ProfileDataPolicy
            | ProductChoice::InstanceRuntime
            | ProductChoice::InstanceWindow
            | ProductChoice::WizardRuntime
            | ProductChoice::WizardWindow => return None,
        })
    }

    pub(super) fn product_select_field(
        &self,
        id: &'static str,
        label: &'static str,
        value: &str,
        kind: ProductChoice,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let focus = self.shell.surfaces.trigger_focus(id, cx);
        let disabled = self.busy && !Self::available_while_busy(&UiAction::OpenProductSelect(kind));
        let open = self.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.product == Some(kind)),
        );
        let anchor = Rc::new(Cell::new(None));
        let pointer_anchor = anchor.clone();
        let pointer_focus = focus.clone();
        let accessible_focus = focus.clone();
        let view = cx.weak_entity();
        let selector = gpui_base::Select::new(format!("{id}-select"))
            .open(open)
            .disabled(disabled)
            .focus_handle(&focus)


            .on_open_change(move |open, window, cx| {
                let _ = view.update(cx, |view, cx| {
                    if open {
                        view.open_product_select_from_trigger(
                            kind,
                            anchor.get(),
                            Some(&accessible_focus),
                            window,
                            cx,
                        );
                    } else if view.shell.surfaces.top().is_some_and(|layer|
                        matches!(&layer.kind, Surface::Select(state) if state.product == Some(kind))) {
                        // Base owns focus restoration for its own Cancel path.
                        view.shell.surfaces.discard();
                    }
                    view.motion_dirty = true;
                    cx.notify();
                });
            })
            .child(div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .size_full()
                .px(px(10.))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child({
                    let anchor = pointer_anchor.clone();
                    gpui::canvas(move |bounds, _, _| anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().inset_0()
                })
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |view, _, window, cx| {
                    if disabled { return; }
                    view.open_product_select_from_trigger(
                        kind,
                        pointer_anchor.get(),
                        Some(&pointer_focus),
                        window,
                        cx,
                    );
                    view.motion_dirty = true;
                    cx.notify();
                    cx.stop_propagation();
                }))
                .child(div().flex_1().min_w_0().text_ellipsis().child(value.to_owned()))
                .child(Icon::Selector.render(14., self.palette.muted)));
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child(label))
            .child(ProductSelectTrigger {
                base: selector,
                focus,
                palette: self.palette,
                disabled,
            })
    }

    fn open_select_at(
        &mut self,
        title: &str,
        choices: Vec<SelectChoice<UiAction>>,
        identity: SelectIdentity,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_open_select(&identity) {
            return;
        }
        let menu = match identity {
            SelectIdentity::AppMenu => Some(MenuKind::App),
            SelectIdentity::ProfileActions(_) => Some(MenuKind::ProfileActions),
            SelectIdentity::Product(_) | SelectIdentity::Appearance { .. } => None,
        };
        let product = match identity {
            SelectIdentity::Product(kind) => Some(kind),
            _ => None,
        };
        let width = if menu == Some(MenuKind::App) {
            px(192.)
        } else {
            anchor.map_or(px(224.), |bounds| bounds.size.width.max(px(144.)))
        }
        .min((window.viewport_size().width - px(16.)).max(px(1.)));
        let active = choices
            .iter()
            .position(|choice| choice.selected)
            .unwrap_or(0);
        let focus = cx.focus_handle().tab_stop(true);
        self.shell.surfaces.open(
            Surface::Select(SelectState {
                title: title.into(),
                choices,
                menu,
                active,
                focus: focus.clone(),
                product,
                identity,
                typeahead: String::new(),
                typed_at: None,
                anchor: anchor.map_or_else(
                    || point(px(16.), px(56.)),
                    |bounds| point(bounds.left(), bounds.bottom() + px(6.)),
                ),
                width,
            }),
            window,
            cx,
        );
        focus.focus(window, cx);
    }

    pub(super) fn open_appearance_select(
        &mut self,
        wizard: bool,
        color: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_appearance_select_at(wizard, color, None, window, cx);
    }

    pub(super) fn open_appearance_select_at(
        &mut self,
        wizard: bool,
        color: bool,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let appearance = if wizard {
            self.wizard_appearance()
        } else if let Some(Form::Profile(form)) = &self.form {
            form.original.appearance
        } else {
            return;
        };
        let choices: Vec<_> = if color {
            ProfileColor::ALL
                .into_iter()
                .map(|value| SelectChoice {
                    label: value.label().to_owned(),
                    selected: value == appearance.color,
                    action: if wizard {
                        UiAction::WizardProfileColor(value)
                    } else {
                        UiAction::ProfileColor(value)
                    },
                })
                .collect()
        } else {
            ProfileIcon::ALL
                .into_iter()
                .map(|value| SelectChoice {
                    label: value.label().to_owned(),
                    selected: value == appearance.icon,
                    action: if wizard {
                        UiAction::WizardProfileIcon(value)
                    } else {
                        UiAction::ProfileIcon(value)
                    },
                })
                .collect()
        };
        self.open_select_at(
            if color {
                "Profile color"
            } else {
                "Profile icon"
            },
            choices,
            SelectIdentity::Appearance { wizard, color },
            anchor,
            window,
            cx,
        );
    }

    pub(super) fn open_profile_actions_at(
        &mut self,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(profile) = self.inspected_profile() else {
            return;
        };
        let id = profile.id.clone();
        let mut choices = Vec::new();
        if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Separated {
            choices.push(SelectChoice {
                label: "Copy settings".into(),
                selected: false,
                action: UiAction::CopyProfileId(id.clone()),
            });
        }
        choices.push(SelectChoice {
            label: "Delete from library".into(),
            selected: false,
            action: UiAction::DeleteProfileId(id.clone()),
        });
        self.open_select_at(
            "Profile actions",
            choices,
            SelectIdentity::ProfileActions(id),
            anchor,
            window,
            cx,
        );
    }

    pub(super) fn open_app_menu_at(
        &mut self,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let current = if self.tab == Tab::Profiles {
            Tab::Instances
        } else {
            self.tab
        };
        let choices = APP_MENU
            .into_iter()
            .filter(|item| {
                self.tab != Tab::Onboarding || matches!(item.tab, Tab::Settings | Tab::About)
            })
            .map(|item| SelectChoice {
                label: item.label.into(),
                selected: item.tab == current,
                action: UiAction::Tab(item.tab),
            })
            .collect();
        self.open_select_at(
            "Block menu",
            choices,
            SelectIdentity::AppMenu,
            anchor,
            window,
            cx,
        );
    }

    pub(super) fn select_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(layer) = self.shell.surfaces.top_mut() else {
            return false;
        };
        let Surface::Select(state) = &mut layer.kind else {
            return false;
        };
        let modifiers = event.keystroke.modifiers;
        if modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
            || state.choices.is_empty()
        {
            return false;
        }
        let count = state.choices.len();
        let inspection = match event.keystroke.key.as_str() {
            "down" => (state.active + 1) % count,
            "up" => (state.active + count - 1) % count,
            "home" => 0,
            "end" => count - 1,
            _ => {
                let text = event
                    .keystroke
                    .key_char
                    .as_deref()
                    .unwrap_or(&event.keystroke.key);
                if text.graphemes(true).count() != 1
                    || text.trim().is_empty()
                    || text.chars().any(char::is_control)
                {
                    return false;
                }
                if let Some(index) = state.typeahead(text, cx.background_executor().now()) {
                    state.active = index;
                }
                state.focus.focus(window, cx);
                self.motion_dirty = true;
                cx.notify();
                return true;
            }
        };
        state.active = inspection;
        state.typeahead.clear();
        state.typed_at = None;
        state.focus.focus(window, cx);
        self.motion_dirty = true;
        cx.notify();
        true
    }

    fn app_menu_header(&self) -> gpui::Div {
        div()
            .child(
                div()
                    .px_2()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(super::shell::brand(36.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child("Block"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(self.palette.muted))
                                    .child(format!("v{}", self.build_info.version)),
                            ),
                    ),
            )
            .child(div().h(px(1.)).my_1().bg(rgb(self.palette.border)))
    }

    pub(super) fn select_options(
        &self,
        state: &SelectState<UiAction>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let revision = self.shell.surfaces.layers().find_map(|layer| {
            matches!(&layer.kind, Surface::Select(current) if current.focus == state.focus)
                .then_some(layer.revision)
        });
        let mut content = div();
        if state.menu == Some(MenuKind::App) {
            content = content.child(self.app_menu_header());
        }
        content.children(state.choices.iter().enumerate().map(|(index, choice)| {
            let disabled = self.busy && !Self::available_while_busy(&choice.action);
            let control_state = super::components::ControlState {
                disabled,
                ..Default::default()
            };
            let action = choice.action.clone();
            let on_activate = cx.listener(move |view, (): &(), window, cx| {
                if !view.shell.surfaces.top().is_some_and(|layer| {
                    Some(layer.revision) == revision && matches!(layer.kind, Surface::Select(_))
                }) {
                    return;
                }
                view.handle(UiAction::SelectValue(Box::new(action.clone())), window, cx);
            });
            let app_item = (state.menu == Some(MenuKind::App))
                .then(|| app_menu_item(&choice.action))
                .flatten();
            let id = app_item.map_or_else(
                || format!("select-choice-{index}"),
                |item| item.id.to_owned(),
            );
            let mut option = if state.menu.is_some() {
                super::components::button_role(
                    id,
                    choice.label.clone(),
                    self.palette,
                    super::components::ButtonVariant::Ghost,
                    control_state,
                    gpui::accesskit::Role::MenuItem,
                    on_activate,
                )
                .w_full()
                .justify_start()
            } else {
                super::components::select_option(
                    id,
                    choice.label.clone(),
                    choice.selected,
                    self.palette,
                    control_state,
                    on_activate,
                )
            }
            .tab_stop(index == state.active && !disabled);
            if index == state.active {
                option = option.track_focus(&state.focus.clone().tab_stop(!disabled));
            }
            if let Some(item) = app_item {
                option = option
                    .flex_row_reverse()
                    .justify_end()
                    .gap_2()
                    .child(item.icon.render(16., self.palette.muted));
                if choice.selected {
                    option = option.bg(rgb(self.palette.accent));
                }
            }
            let section_start = app_item
                .is_some_and(|item| matches!(item.tab, Tab::Onboarding | Tab::About))
                && index > 0
                && state
                    .choices
                    .get(index - 1)
                    .and_then(|choice| app_menu_item(&choice.action))
                    .is_some_and(|item| !matches!(item.tab, Tab::Onboarding | Tab::About));
            div()
                .children(
                    section_start.then(|| div().h(px(1.)).my_1().bg(rgb(self.palette.border))),
                )
                .child(option)
        }))
    }
}

fn app_menu_item(action: &UiAction) -> Option<&'static AppMenuItem> {
    let UiAction::Tab(tab) = action else {
        return None;
    };
    APP_MENU.iter().find(|item| item.tab == *tab)
}

fn runtime_choices(
    automatic: bool,
    action: impl Fn(bool) -> UiAction,
) -> Vec<SelectChoice<UiAction>> {
    [("Automatic Java", true), ("Custom executable", false)]
        .into_iter()
        .map(|(label, value)| SelectChoice {
            label: label.into(),
            selected: automatic == value,
            action: action(value),
        })
        .collect()
}

fn window_choices(
    width: &str,
    height: &str,
    custom: bool,
    preset: impl Fn(u32, u32) -> UiAction,
    custom_action: UiAction,
) -> Vec<SelectChoice<UiAction>> {
    let current = (
        width.trim().parse::<u32>().ok(),
        height.trim().parse::<u32>().ok(),
    );
    let mut choices: Vec<_> = super::settings::WINDOW_PRESETS
        .into_iter()
        .map(|(width, height)| SelectChoice {
            label: format!("{width} × {height}"),
            selected: !custom && current == (Some(width), Some(height)),
            action: preset(width, height),
        })
        .collect();
    choices.push(SelectChoice {
        label: "Custom size…".into(),
        selected: custom,
        action: custom_action,
    });
    choices
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
