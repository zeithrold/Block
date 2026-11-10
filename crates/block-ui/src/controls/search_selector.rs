//! One searchable selection contract for Home's real accounts and instances.
//! Keyboard inspection never changes Home's current selection until activation.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Selector state belongs to its parent shell."
)]

use super::scroll_area::ScrollAreaExt as _;
use block_launcher::catalog::InstanceAppearance;
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, ScrollHandle, Styled as _, Window, div, point, px, rgb,
};

use super::{
    Block, TextInput, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState},
    input, note,
    shell::{Drawer, Surface},
};

#[derive(Debug)]
pub(super) struct SearchSelectorState {
    drawer: Drawer,
    query: Entity<TextInput>,
    active: Option<String>,
    last_query: String,
    scroll: ScrollHandle,
    row_focus: FocusHandle,
}

impl SearchSelectorState {
    pub(super) fn refresh_placeholder(
        &self,
        language: crate::Language,
        cx: &mut Context<'_, Block>,
    ) {
        let key = if self.drawer == Drawer::Accounts {
            "search.accounts_placeholder"
        } else {
            "search.instances_placeholder"
        };
        let placeholder = tr!(language, key);
        self.query
            .update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
    }

    pub(super) fn query_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        Self {
            drawer: self.drawer,
            query: super::frozen_input(&self.query, cx),
            active: self.active.clone(),
            last_query: self.last_query.clone(),
            scroll,
            row_focus: cx.focus_handle(),
        }
    }
}

#[derive(Clone, Debug)]
struct Choice {
    id: String,
    title: String,
    description: String,
    current: bool,
    enabled: bool,
    appearance: Option<InstanceAppearance>,
    badges: Vec<String>,
}

impl Block {
    pub(super) fn open_selector(
        &mut self,
        drawer: Drawer,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !matches!(drawer, Drawer::Accounts | Drawer::Instances) {
            self.shell.selector = None;
            return;
        }
        let language = self.language();
        let query = input(
            "",
            &if drawer == Drawer::Accounts {
                tr!(language, "search.accounts_placeholder")
            } else {
                tr!(language, "search.instances_placeholder")
            },
            cx,
        );
        let identity = query.entity_id();
        cx.observe(&query, move |view, _, cx| {
            if view
                .shell
                .selector
                .as_ref()
                .is_some_and(|state| state.query.entity_id() == identity)
            {
                view.selector_query_changed(cx);
            }
        })
        .detach();
        query.read(cx).focus_handle(cx).focus(window, cx);
        self.shell.selector = Some(SearchSelectorState {
            drawer,
            query,
            active: None,
            last_query: String::new(),
            scroll: ScrollHandle::new(),
            row_focus: cx.focus_handle().tab_stop(true),
        });
        let choices = self.selector_choices(drawer, "");
        let active = choices
            .iter()
            .position(|choice| choice.current && choice.enabled)
            .or_else(|| choices.iter().position(|choice| choice.enabled));
        if let Some(state) = &mut self.shell.selector {
            state.active = active.map(|index| choices[index].id.clone());
            if let Some(index) = active {
                state.scroll.scroll_to_item(index);
            }
        }
        cx.notify();
    }

    fn selector_query_changed(&mut self, cx: &mut Context<'_, Self>) {
        let Some(state) = &self.shell.selector else {
            return;
        };
        let value = state.query.read(cx).value();
        if value == state.last_query {
            return;
        }
        let query = value.trim().to_lowercase();
        if query == state.last_query.trim().to_lowercase() {
            // The search result did not change, but the raw input did. Repaint
            // the Clear action without resetting the current row or scroll.
            if let Some(state) = &mut self.shell.selector {
                state.last_query = value;
            }
            cx.notify();
            return;
        }
        let choices = self.selector_choices(state.drawer, &query);
        let active = active_index(&choices, state.active.as_deref());
        if let Some(state) = &mut self.shell.selector {
            state.last_query = value;
            state.active = active.map(|index| choices[index].id.clone());
            state.scroll.set_offset(point(px(0.), px(0.)));
            if let Some(index) = active {
                state.scroll.scroll_to_item(index);
            }
        }
        cx.notify();
    }

    /// Returns whether the selector owns this key. Home/End in the search
    /// field stay with text editing; unfinished IME composition keeps all keys.
    pub(super) fn selector_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(state) = &self.shell.selector else {
            return false;
        };
        if !self.shell.surfaces.top().is_some_and(
            |layer| matches!(layer.kind, Surface::Drawer(drawer) if drawer == state.drawer),
        ) {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
            || modifiers.shift
        {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if !matches!(key, "up" | "down" | "home" | "end" | "enter") {
            return false;
        }
        if state.query.read(cx).is_composing() {
            return false;
        }
        let query_focused = state.query.read(cx).focus_handle(cx).is_focused(window);
        let row_focused = state.row_focus.is_focused(window);
        // Native Base controls commit Enter/Space on key up. A Clear or Manage
        // button must therefore keep its key down instead of the
        // selector activating the inspected choice before that control fires.
        if !query_focused && !row_focused {
            return false;
        }
        if query_focused && matches!(key, "home" | "end") {
            return false;
        }
        let choices = self.selector_choices(
            state.drawer,
            &state.query.read(cx).value().trim().to_lowercase(),
        );
        let active = active_index(&choices, state.active.as_deref());
        if key == "enter" {
            // The focused row already has Base's native activation handler.
            // Only the search input needs a submit-to-inspected-choice action.
            if !query_focused {
                return false;
            }
            if let Some(index) = active {
                let action = choice_action(state.drawer, choices[index].id.clone());
                self.handle(action, window, cx);
            }
            return true;
        }
        let next = navigation_index(&choices, active, key);
        if let Some(state) = &mut self.shell.selector {
            state.active = next.map(|index| choices[index].id.clone());
            if let Some(index) = next {
                state.scroll.scroll_to_item(index);
            }
            if !query_focused {
                state.row_focus.focus(window, cx);
            }
        }
        cx.notify();
        true
    }

    /// Search, result rows and the management jump, without a surface-local
    /// heading: the owning sheet or dialog supplies its own title.
    pub(super) fn render_selector_body(
        &self,
        drawer: Drawer,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let Some(state) = self
            .shell
            .selector
            .as_ref()
            .filter(|state| state.drawer == drawer)
        else {
            return div()
                .child(note(tr!(self.language(), "search.reopen")))
                .into_any_element();
        };
        let has_query = !state.query.read(cx).value().is_empty();
        let language = self.language();
        let accounts = drawer == Drawer::Accounts;
        let rows = self.selector_results(drawer, state, window, cx);
        let query_entity = state.query.clone();
        div()
            .id("search-selector")
            .h_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if view.selector_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(components::field(
                        if accounts {
                            tr!(language, "search.accounts_field")
                        } else {
                            tr!(language, "search.games_field")
                        },
                        &state.query,
                    )))
                    .children(has_query.then(|| {
                        components::button(
                            "selector-clear",
                            tr!(language, "search.clear"),
                            self.palette,
                            ButtonVariant::Ghost,
                            ControlState::default(),
                            move |(), window, cx| {
                                query_entity.update(cx, |field, cx| {
                                    field.set_value("", cx);
                                    field.focus_handle(cx).focus(window, cx);
                                });
                            },
                        )
                    })),
            )
            .child(rows)
            .child(
                self.button(
                    if accounts {
                        "manage-accounts"
                    } else {
                        "manage-games"
                    },
                    if accounts {
                        tr!(language, "search.manage_accounts")
                    } else {
                        tr!(language, "search.manage_games")
                    },
                    UiAction::Tab(if accounts {
                        super::Tab::Accounts
                    } else {
                        super::Tab::Instances
                    }),
                    cx,
                )
                .flex_shrink_0(),
            )
            .into_any_element()
    }

    fn selector_results(
        &self,
        drawer: Drawer,
        state: &SearchSelectorState,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let query = state.query.read(cx).value().trim().to_lowercase();
        let choices = self.selector_choices(drawer, &query);
        let active = active_index(&choices, state.active.as_deref());
        let language = self.language();
        let accounts = drawer == Drawer::Accounts;

        let mut rows = div()
            .id("selector-results")
            .debug_selector(|| "selector-results".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .children(choices.iter().enumerate().map(|(index, choice)| {
                self.selector_row(drawer, choice, active == Some(index), state, window, cx)
            }));
        if choices.is_empty() {
            rows = rows.child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(if query.is_empty() {
                        if accounts {
                            tr!(language, "search.no_accounts")
                        } else {
                            tr!(language, "search.no_games")
                        }
                    } else {
                        tr!(language, "search.no_results")
                    })
                    .child(note(if query.is_empty() {
                        if accounts {
                            tr!(language, "search.add_account_hint")
                        } else {
                            tr!(language, "search.create_game_hint")
                        }
                    } else {
                        tr!(language, "search.try_another")
                    })),
            );
        }
        // Add painted chrome after data rows so ScrollHandle indices remain
        // data indices. The active option is the composite's single tab stop.
        rows.scroll_list_area(&state.scroll, "search.results", self.palette)
            .tab_index(-1)
    }

    fn selector_row(
        &self,
        drawer: Drawer,
        choice: &Choice,
        active: bool,
        state: &SearchSelectorState,
        _window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let accounts = drawer == Drawer::Accounts;
        let id = choice.id.clone();
        let action = choice_action(drawer, id.clone());
        let enabled = choice.enabled;
        let mut primary = components::button(
            format!(
                "switch-{}-{id}",
                if accounts { "account" } else { "instance" }
            ),
            "",
            self.palette,
            ButtonVariant::Ghost,
            ControlState {
                disabled: !enabled,
                ..ControlState::default()
            },
            cx.listener(move |view, (), window, cx| view.handle(action.clone(), window, cx)),
        )
        .tab_index(if active { 0 } else { -1 })
        .flex_1()
        .min_w_0()
        .h_auto()
        .p(px(15.))
        .justify_start()
        .items_center()
        .gap_3()
        .bg(rgb(if active {
            self.palette.accent
        } else {
            self.palette.popover
        }))
        .child(
            choice
                .appearance
                .map_or_else(
                    || {
                        components::account_avatar(
                            &choice.title,
                            self.account_avatars.get(&choice.id),
                            self.palette,
                            36.,
                            AvatarState::Fallback,
                        )
                    },
                    |appearance| components::instance_avatar(appearance, self.palette, 36.),
                )
                .debug_selector({
                    let selector = format!("selector-avatar-{id}");
                    move || selector
                }),
        )
        .child(self.selector_row_text(choice))
        .child(super::selection_motion::indicator(
            format!("selector-current-{id}").into(),
            true,
            if choice.current { 1. } else { 0. },
            self.palette,
        ));
        if active {
            primary = primary.track_focus(&state.row_focus);
        }
        div().flex().items_center().gap_1().child(primary)
    }

    fn selector_row_text(&self, choice: &Choice) -> gpui::Div {
        let selector = format!("selector-text-{}", choice.id);
        div()
            .debug_selector(move || selector)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .truncate()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(choice.title.clone()),
            )
            .child(div().flex().flex_wrap().gap_1().children(
                choice.badges.iter().map(|badge| {
                    components::badge(badge.clone(), self.palette, BadgeVariant::Info)
                }),
            ))
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(choice.description.clone()),
            )
    }

    fn selector_choices(&self, drawer: Drawer, query: &str) -> Vec<Choice> {
        let language = self.language();
        let mut choices: Vec<Choice> = if drawer == Drawer::Accounts {
            self.accounts
                .iter()
                .filter_map(|account| {
                    let id = account_id(account);
                    let description = match account.provider {
                        block_auth::AccountProvider::Microsoft => {
                            tr!(language, "account.microsoft")
                        }
                        block_auth::AccountProvider::Offline => tr!(language, "account.offline"),
                        block_auth::AccountProvider::Authlib => {
                            let authority = self.account_authority(account);
                            let url = self.account_provider(account).map_or_else(
                                || tr!(language, "account.provider_unavailable"),
                                |provider| provider.api_url.clone(),
                            );
                            tr!(
                                language,
                                "account.authlib",
                                authority = authority,
                                url = url,
                            )
                        }
                    };
                    format!("{} {description}", account.player_name)
                        .to_lowercase()
                        .contains(query)
                        .then(|| Choice {
                            current: self.selected_account.as_ref() == Some(&id),
                            id,
                            title: account.player_name.clone(),
                            description,
                            enabled: !self.busy,
                            appearance: None,
                            badges: vec![self.account_authority(account)],
                        })
                })
                .collect()
        } else if drawer == Drawer::Instances {
            self.snapshot
                .instances
                .iter()
                .filter_map(|instance| {
                    let target = instance
                        .targets
                        .iter()
                        .find(|target| target.id == instance.target_id);
                    let valid = target.is_some();
                    let description = if valid {
                        instance.game_directory.display().to_string()
                    } else {
                        tr!(language, "instance.target_unavailable")
                    };
                    format!("{} {description}", instance.name)
                        .to_lowercase()
                        .contains(query)
                        .then(|| Choice {
                            id: instance.id.clone(),
                            title: instance.name.clone(),
                            description,
                            current: self.selected_instance.as_ref() == Some(&instance.id),
                            enabled: valid && !self.busy,
                            appearance: Some(instance.appearance),
                            badges: {
                                let mut badges = vec![tr!(
                                    language,
                                    "badge.minecraft_version",
                                    version = instance.minecraft_version,
                                )];
                                if let Some(target) = target {
                                    badges.push(target.display_name());
                                }
                                badges
                            },
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        // Stable sorting promotes the current identity without reordering peers.
        choices.sort_by_key(|choice| !choice.current);
        choices
    }
}

fn choice_action(drawer: Drawer, id: String) -> UiAction {
    if drawer == Drawer::Accounts {
        UiAction::SelectAccount(id)
    } else {
        UiAction::SelectInstance(id)
    }
}

fn active_index(choices: &[Choice], active: Option<&str>) -> Option<usize> {
    choices
        .iter()
        .position(|choice| choice.enabled && Some(choice.id.as_str()) == active)
        .or_else(|| {
            choices
                .iter()
                .position(|choice| choice.current && choice.enabled)
        })
        .or_else(|| choices.iter().position(|choice| choice.enabled))
}

fn navigation_index(choices: &[Choice], active: Option<usize>, key: &str) -> Option<usize> {
    let enabled: Vec<_> = choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| choice.enabled.then_some(index))
        .collect();
    if enabled.is_empty() {
        return None;
    }
    match key {
        "home" => enabled.first().copied(),
        "end" => enabled.last().copied(),
        "up" => active
            .and_then(|active| enabled.iter().rev().find(|&&index| index < active).copied())
            .or_else(|| enabled.last().copied()),
        _ => active
            .and_then(|active| enabled.iter().find(|&&index| index > active).copied())
            .or_else(|| enabled.first().copied()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, active_index, navigation_index};
    use crate::{
        Tab, UiAction,
        shell::{Drawer, Surface},
        test_support::{ready_snapshot, test_account},
    };
    use gpui::{
        EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext, px, size,
    };

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        let bounds = crate::test_support::control_bounds(cx, selector);
        assert!(bounds.is_some(), "missing {selector}");
        if let Some(bounds) = bounds {
            let window_count = cx.windows().len();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            if cx.windows().len() > window_count
                && let Some(window) = cx.windows().last()
            {
                *cx = VisualTestContext::from_window(*window, &cx.cx);
            }
            crate::test_support::ensure_live_window(cx);
            cx.run_until_parked();
            crate::test_support::ensure_live_window(cx);
        }
    }

    #[test]
    fn navigation_skips_disabled_and_keeps_current_separate_from_active() {
        let choices: Vec<_> = [true, false, true]
            .into_iter()
            .enumerate()
            .map(|(index, enabled)| Choice {
                id: index.to_string(),
                title: index.to_string(),
                description: String::new(),
                current: index == 0,
                enabled,
                appearance: None,
                badges: Vec::new(),
            })
            .collect();
        assert_eq!(active_index(&choices, None), Some(0));
        assert_eq!(navigation_index(&choices, Some(0), "down"), Some(2));
        assert_eq!(navigation_index(&choices, Some(2), "down"), Some(0));
        assert_eq!(navigation_index(&choices, Some(0), "up"), Some(2));
        assert_eq!(navigation_index(&choices, Some(2), "home"), Some(0));
        assert_eq!(navigation_index(&choices, Some(0), "end"), Some(2));
        assert!(choices[0].current && !choices[2].current);
        assert_eq!(navigation_index(&[], None, "down"), None);
    }

    #[gpui::test]
    fn current_identity_is_sorted_first_with_real_metadata(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update(cx, |view, _| {
            view.snapshot = ready_snapshot();
            let mut second = view.snapshot.instances[0].clone();
            second.id = "instance-z".into();
            second.name = "Current last in catalog".into();
            view.snapshot.instances.push(second);
            view.selected_instance = Some("instance-z".into());
            let instances = view.selector_choices(Drawer::Instances, "");
            assert_eq!(instances[0].id, "instance-z");
            assert_eq!(instances[1].id, "instance-a");
            assert_eq!(instances[0].badges, ["Minecraft 1.20.6", "Vanilla"]);
            let result = test_account();
            assert!(result.is_ok());
            if let Ok(first) = result {
                let mut second = first.clone();
                let id = "00000000-0000-4000-8000-000000000002".parse();
                assert!(id.is_ok());
                if let Ok(id) = id {
                    second.reference.player_id = id;
                }
                let current = crate::account_id(&second);
                view.accounts = vec![first, second];
                view.selected_account = Some(current.clone());
                assert_eq!(view.selector_choices(Drawer::Accounts, "")[0].id, current);
            }
        });
    }

    #[gpui::test]
    fn account_and_game_selectors_paint_informational_badges_in_both_themes(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        for (appearance, palette) in [
            (crate::Appearance::Light, crate::Palette::LIGHT),
            (crate::Appearance::Dark, crate::Palette::DARK),
        ] {
            for drawer in [Drawer::Accounts, Drawer::Instances] {
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.preferences.appearance = appearance;
                        view.preferences.reduce_motion = true;
                        view.snapshot = ready_snapshot();
                        let account = test_account();
                        assert!(account.is_ok());
                        view.accounts = account.into_iter().collect();
                        view.router.navigate(Tab::Home);
                        view.shell.surfaces.clear(window, &view.focus, cx);
                        view.handle(UiAction::Drawer(Some(drawer)), window, cx);
                    });
                });
                cx.run_until_parked();
                let painted = cx.update(|window, _| {
                    window.painted_quads().iter().any(|quad| {
                        quad.background.as_solid().is_some_and(|color| {
                            let actual = color.to_rgb();
                            let expected = gpui::rgb(palette.info_background);
                            (actual.r - expected.r).abs() < 0.001
                                && (actual.g - expected.g).abs() < 0.001
                                && (actual.b - expected.b).abs() < 0.001
                        })
                    })
                });
                assert!(
                    painted,
                    "{appearance:?} {drawer:?}: colored badges must paint"
                );
                let (row_id, indicator_id) = if drawer == Drawer::Accounts {
                    (
                        "switch-account-00000000-0000-4000-8000-000000000001",
                        "selector-current-00000000-0000-4000-8000-000000000001-indicator",
                    )
                } else {
                    (
                        "switch-instance-instance-a",
                        "selector-current-instance-a-indicator",
                    )
                };
                let row = crate::test_support::control_bounds(cx, row_id);
                let indicator = cx.debug_bounds(indicator_id);
                let scale = cx.update(|window, _| window.scale_factor());
                assert!(
                    row.zip(indicator).is_some_and(|(row, indicator)| {
                        indicator.size == size(px(16.), px(16.))
                            && row.right() - indicator.right() == px(16.)
                            && (f32::from(row.center().y) - f32::from(indicator.center().y)).abs()
                                <= 0.5 / scale
                    }),
                    "{drawer:?}: every Radio is centered in the whole choice with a 16px inset"
                );
                if drawer == Drawer::Instances {
                    let avatar = cx.debug_bounds("selector-avatar-instance-a");
                    let text = cx.debug_bounds("selector-text-instance-a");
                    assert!(
                        row.zip(avatar)
                            .zip(text)
                            .is_some_and(|((row, avatar), text)| {
                                avatar.size == size(px(36.), px(36.))
                                    && avatar.left() - row.left() == px(16.)
                                    && text.left() - avatar.right() == px(12.)
                            }),
                        "Instance avatar keeps its inset and gap before the text"
                    );
                }
            }
        }
    }

    #[gpui::test]
    fn arrow_inspection_skips_unavailable_instances_and_only_enter_changes_home(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Home);
                view.snapshot = ready_snapshot();
                view.selected_instance = Some("instance-a".into());
                let mut missing = view.snapshot.instances[0].clone();
                missing.id = "unavailable".into();
                missing.target_id = "missing".into();
                let mut second = view.snapshot.instances[0].clone();
                second.id = "instance-b".into();
                second.name = "世界 Gamma".into();
                view.snapshot.instances.extend([missing, second]);
                view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
            });
        });
        crate::test_support::press(cx, "down");
        assert!(view.read_with(cx, |view, _| {
            view.selected_instance.as_deref() == Some("instance-a")
                && view
                    .shell
                    .selector
                    .as_ref()
                    .is_some_and(|state| state.active.as_deref() == Some("instance-b"))
        }));
        crate::test_support::press(cx, "enter");
        assert!(
            view.read_with(cx, |view, _| view.selected_instance.as_deref()
                == Some("instance-b")
                && !view.shell.surfaces.is_open())
        );
    }

    #[gpui::test]
    fn empty_search_and_unfinished_ime_do_not_select_or_dismiss(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.snapshot = ready_snapshot();
                view.selected_instance = Some("instance-a".into());
                view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
            });
        });
        crate::test_support::control_bounds(cx, "selector-results");
        cx.simulate_input("no matching instance");
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, _| {
            view.shell.surfaces.is_open()
                && view
                    .shell
                    .selector
                    .as_ref()
                    .is_some_and(|state| state.active.is_none())
        }));
        crate::test_support::press(cx, "home");
        cx.simulate_input("前");
        assert!(view.read_with(cx, |view, cx| {
            view.shell
                .selector
                .as_ref()
                .is_some_and(|state| state.query.read(cx).value() == "前no matching instance")
        }));
        click(cx, "selector-clear");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                if let Some(state) = &view.shell.selector {
                    state.query.update(cx, |field, cx| {
                        field.focus_handle(cx).focus(window, cx);
                        field.replace_and_mark_text_in_range(None, "中文", None, window, cx);
                    });
                }
            });
        });
        crate::test_support::press(cx, "down enter escape");
        assert!(view.read_with(cx, |view, cx| {
            view.selected_instance.as_deref() == Some("instance-a")
                && view.shell.surfaces.is_open()
                && view
                    .shell
                    .selector
                    .as_ref()
                    .is_some_and(|state| state.query.read(cx).is_composing())
        }));
    }

    #[gpui::test]
    fn account_switcher_keeps_removal_in_management(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.accounts = vec![account];
                    view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
                });
            });
            crate::test_support::control_bounds(cx, "selector-results");
            assert!(
                cx.debug_bounds("selector-remove-00000000-0000-4000-8000-000000000001")
                    .is_none(),
                "Switching accounts must not expose a destructive row action"
            );
            assert!(
                crate::test_support::control_bounds(cx, "selector-clear").is_none(),
                "An empty search needs no Clear action"
            );
            cx.simulate_input("Test");
            click(cx, "selector-clear");
            assert!(crate::test_support::control_bounds(cx, "selector-clear").is_none());
            click(cx, "manage-accounts");
            assert!(view.read_with(cx, |view, _| {
                view.router.tab() == Tab::Accounts
                    && view.accounts.len() == 1
                    && view.selected_account.is_none()
                    && !view.shell.surfaces.is_open()
            }));
            // Account management, not the temporary selector, owns removal.
            cx.simulate_resize(size(px(360.), px(720.)));
            click(cx, "account-inspect-00000000-0000-4000-8000-000000000001");
            click(cx, "account-logout-00000000-0000-4000-8000-000000000001");
            assert!(view.read_with(cx, |view, _| matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::RemoveAccount(_))
            )));
            crate::test_support::press(cx, "escape");
            assert!(view.read_with(cx, |view, _| {
                view.accounts.len() == 1 && !view.shell.surfaces.is_open()
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn keyboard_clear_keeps_its_native_action_without_choosing_the_inspected_account(
        cx: &mut TestAppContext,
    ) {
        let account = test_account();
        assert!(account.is_ok());
        if let Ok(account) = account {
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.accounts = vec![account];
                    view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
                });
            });
            crate::test_support::control_bounds(cx, "selector-results");
            cx.simulate_input("Test");
            crate::test_support::press(cx, "tab");
            crate::test_support::press(cx, "enter");
            assert!(view.read_with(cx, |view, cx| {
                view.selected_account.is_none()
                    && matches!(
                        view.shell.surfaces.top().map(|layer| &layer.kind),
                        Some(Surface::Drawer(Drawer::Accounts))
                    )
                    && view
                        .shell
                        .selector
                        .as_ref()
                        .is_some_and(|state| state.query.read(cx).value().is_empty())
            }));
            cx.update(|window, cx| {
                view.read_with(cx, |view, cx| {
                    assert!(
                        view.shell
                            .selector
                            .as_ref()
                            .is_some_and(|state| state.query_focus(cx).is_focused(window)),
                        "clearing the search must return focus to the input"
                    );
                });
            });
            cx.simulate_input("Test");
            assert!(view.read_with(cx, |view, cx| {
                view.shell
                    .selector
                    .as_ref()
                    .is_some_and(|state| state.query.read(cx).value() == "Test")
            }));
        }
    }

    #[gpui::test]
    fn whitespace_input_still_offers_a_clear_action(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
            });
        });
        crate::test_support::control_bounds(cx, "selector-results");
        cx.simulate_input("  ");
        assert!(
            crate::test_support::control_bounds(cx, "selector-clear").is_some(),
            "nonempty input should be clearable even when its search terms are blank"
        );
        click(cx, "selector-clear");
        assert!(view.read_with(cx, |view, cx| {
            view.shell
                .selector
                .as_ref()
                .is_some_and(|state| state.query.read(cx).value().is_empty())
        }));
        assert!(crate::test_support::control_bounds(cx, "selector-clear").is_none());
    }

    #[gpui::test]
    fn roving_first_and_last_reveal_the_active_row_without_changing_current(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(360.), px(540.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.snapshot = ready_snapshot();
                let original = view.snapshot.instances[0].clone();
                view.snapshot.instances = (0..40)
                    .map(|index| {
                        let mut instance = original.clone();
                        instance.id = format!("instance-{index}");
                        instance.name = format!("Game {index}");
                        instance
                    })
                    .collect();
                view.selected_instance = Some("instance-39".into());
                view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
            });
        });
        crate::test_support::press(cx, "tab home");
        cx.run_until_parked();
        let viewport = cx.debug_bounds("selector-results");
        // The current instance is deliberately promoted ahead of catalog order.
        let first = cx.debug_bounds("switch-instance-instance-39");
        assert!(viewport.zip(first).is_some_and(
            |(viewport, row)| row.top() >= viewport.top() && row.bottom() <= viewport.bottom()
        ));
        crate::test_support::press(cx, "end");
        cx.run_until_parked();
        let last = cx.debug_bounds("switch-instance-instance-38");
        assert!(viewport.zip(last).is_some_and(
            |(viewport, row)| row.top() >= viewport.top() && row.bottom() <= viewport.bottom()
        ));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_instance.clone()),
            Some("instance-39".into())
        );
    }
}
