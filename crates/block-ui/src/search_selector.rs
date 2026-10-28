//! One searchable selection contract for Home's real accounts and profiles.
//! Keyboard inspection never changes Home's current selection until activation.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Selector state belongs to its parent shell."
)]

use super::scroll_area::ScrollAreaExt as _;
use block_launcher::catalog::ProfileAppearance;
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, ScrollHandle, Styled as _, Window, div, point, px, rgb,
};

use super::{
    Block, TextInput, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState},
    design::Icon,
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
    appearance: Option<ProfileAppearance>,
    badges: Vec<String>,
}

impl Block {
    pub(super) fn open_selector(
        &mut self,
        drawer: Drawer,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !matches!(drawer, Drawer::Accounts | Drawer::Profiles) {
            self.shell.selector = None;
            return;
        }
        let query = input(
            "",
            if drawer == Drawer::Accounts {
                "Search accounts…"
            } else {
                "Search profiles…"
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
                .child(note("Open the selector again to search."))
                .into_any_element();
        };
        let has_query = !state.query.read(cx).value().is_empty();
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
                            "Search accounts"
                        } else {
                            "Search games"
                        },
                        &state.query,
                    )))
                    .children(has_query.then(|| {
                        components::button(
                            "selector-clear",
                            "Clear",
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
                        "Manage accounts"
                    } else {
                        "Manage games"
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
        let accounts = drawer == Drawer::Accounts;

        let mut rows = div()
            .id("selector-results")
            .debug_selector(|| "selector-results".into())
            .flex_1()
            .min_h_0()
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
                            "No accounts yet"
                        } else {
                            "No games yet"
                        }
                    } else {
                        "No results"
                    })
                    .child(note(if query.is_empty() {
                        if accounts {
                            "Add an account to choose a player identity."
                        } else {
                            "Create a game to start playing."
                        }
                    } else {
                        "Try another name or clear the search."
                    })),
            );
        }
        // Add painted chrome after data rows so ScrollHandle indices remain
        // data indices. The active option is the composite's single tab stop.
        rows.scroll_list_area(&state.scroll, "Search results", self.palette)
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
                if accounts { "account" } else { "profile" }
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
        .min_h(px(64.))
        .p_3()
        .justify_start()
        .gap_3()
        .bg(rgb(if active {
            self.palette.accent
        } else {
            self.palette.popover
        }))
        .child(choice.appearance.map_or_else(
            || {
                components::account_avatar(
                    &choice.title,
                    self.account_avatars.get(&choice.id),
                    self.palette,
                    36.,
                    AvatarState::Fallback,
                )
            },
            |appearance| components::profile_avatar(appearance, self.palette, 36.),
        ))
        .child(self.selector_row_text(choice))
        .children(
            choice
                .current
                .then(|| Icon::Check.render(16., self.palette.foreground)),
        );
        if active {
            primary = primary.track_focus(&state.row_focus);
        }
        div().flex().items_center().gap_1().child(primary)
    }

    fn selector_row_text(&self, choice: &Choice) -> gpui::Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(div().truncate().child(choice.title.clone()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(choice.badges.iter().map(|badge| {
                        components::badge(badge.clone(), self.palette, BadgeVariant::Secondary)
                    })),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(choice.description.clone()),
            )
    }

    fn selector_choices(&self, drawer: Drawer, query: &str) -> Vec<Choice> {
        let mut choices: Vec<Choice> = if drawer == Drawer::Accounts {
            self.accounts
                .iter()
                .filter_map(|account| {
                    let id = account_id(account);
                    let description = match account.provider {
                        block_auth::AccountProvider::Microsoft => {
                            "Microsoft · Java Edition".to_owned()
                        }
                        block_auth::AccountProvider::Offline => {
                            "Local play · No online authorization".to_owned()
                        }
                        block_auth::AccountProvider::Authlib => format!(
                            "{} · {}",
                            self.account_authority(account),
                            self.account_provider(account)
                                .map_or("Provider unavailable", |provider| provider
                                    .api_url
                                    .as_str())
                        ),
                    };
                    format!("{} {description}", account.player_name)
                        .to_lowercase()
                        .contains(query)
                        .then(|| Choice {
                            current: if self.tab == super::Tab::Onboarding {
                                self.shell.onboarding.account.as_ref() == Some(&id)
                            } else {
                                self.selected_account.as_ref() == Some(&id)
                            },
                            id,
                            title: account.player_name.clone(),
                            description,
                            enabled: !self.busy,
                            appearance: None,
                            badges: vec![self.account_authority(account)],
                        })
                })
                .collect()
        } else if drawer == Drawer::Profiles {
            self.snapshot
                .profiles
                .iter()
                .filter_map(|profile| {
                    let instance = self
                        .snapshot
                        .instances
                        .iter()
                        .find(|instance| instance.id == profile.instance_id);
                    let valid = instance.is_some_and(|instance| {
                        instance
                            .targets
                            .iter()
                            .any(|target| target.id == profile.target_id)
                    });
                    let description = instance.map_or_else(
                        || "Instance unavailable".into(),
                        |instance| {
                            if valid {
                                format!(
                                    "{} · Minecraft {}",
                                    instance.name, instance.minecraft_version
                                )
                            } else {
                                "Launch target unavailable".into()
                            }
                        },
                    );
                    format!("{} {description}", profile.name)
                        .to_lowercase()
                        .contains(query)
                        .then(|| Choice {
                            id: profile.id.clone(),
                            title: profile.name.clone(),
                            description,
                            current: self.selected_profile.as_ref() == Some(&profile.id),
                            enabled: valid && !self.busy,
                            appearance: Some(profile.appearance),
                            badges: instance.map_or_else(Vec::new, |instance| {
                                let mut badges =
                                    vec![format!("Minecraft {}", instance.minecraft_version)];
                                if let Some(target) = instance
                                    .targets
                                    .iter()
                                    .find(|target| target.id == profile.target_id)
                                {
                                    badges.push(target.display_name());
                                }
                                badges
                            }),
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
        UiAction::SelectProfile(id)
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
    fn current_identity_is_sorted_first_with_real_metadata_and_setup_uses_its_account_draft(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update(cx, |view, cx| {
            view.snapshot = ready_snapshot();
            let mut second = view.snapshot.profiles[0].clone();
            second.id = "profile-z".into();
            second.name = "Current last in catalog".into();
            view.snapshot.profiles.push(second);
            view.selected_profile = Some("profile-z".into());
            let profiles = view.selector_choices(Drawer::Profiles, "");
            assert_eq!(profiles[0].id, "profile-z");
            assert_eq!(profiles[1].id, "profile-a");
            assert_eq!(profiles[0].badges, ["Minecraft 1.20.6", "Vanilla"]);
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
                view.begin_onboarding(cx);
                view.choose_onboarding_account(current.clone());
                assert_eq!(view.selector_choices(Drawer::Accounts, "")[0].id, current);
                assert!(view.selected_account.is_none());
            }
        });
    }

    #[gpui::test]
    fn arrow_inspection_skips_unavailable_profiles_and_only_enter_changes_home(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.tab = Tab::Home;
                view.snapshot = ready_snapshot();
                view.selected_profile = Some("profile-a".into());
                let mut missing = view.snapshot.profiles[0].clone();
                missing.id = "unavailable".into();
                missing.instance_id = "missing".into();
                let mut second = view.snapshot.profiles[0].clone();
                second.id = "profile-b".into();
                second.name = "世界 Gamma".into();
                view.snapshot.profiles.extend([missing, second]);
                view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
            });
        });
        crate::test_support::press(cx, "down");
        assert!(view.read_with(cx, |view, _| {
            view.selected_profile.as_deref() == Some("profile-a")
                && view
                    .shell
                    .selector
                    .as_ref()
                    .is_some_and(|state| state.active.as_deref() == Some("profile-b"))
        }));
        crate::test_support::press(cx, "enter");
        assert!(
            view.read_with(cx, |view, _| view.selected_profile.as_deref()
                == Some("profile-b")
                && view.selected_instance.as_deref() == Some("instance-a")
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
                view.selected_profile = Some("profile-a".into());
                view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
            });
        });
        crate::test_support::control_bounds(cx, "selector-results");
        cx.simulate_input("no matching profile");
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
                .is_some_and(|state| state.query.read(cx).value() == "前no matching profile")
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
            view.selected_profile.as_deref() == Some("profile-a")
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
                view.tab == Tab::Accounts
                    && view.accounts.len() == 1
                    && view.selected_account.is_none()
                    && !view.shell.surfaces.is_open()
            }));
            // Account management, not the temporary selector, owns removal.
            cx.simulate_resize(size(px(1152.), px(720.)));
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
                let original = view.snapshot.profiles[0].clone();
                view.snapshot.profiles = (0..40)
                    .map(|index| {
                        let mut profile = original.clone();
                        profile.id = format!("profile-{index}");
                        profile.name = format!("Game {index}");
                        profile
                    })
                    .collect();
                view.selected_profile = Some("profile-39".into());
                view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
            });
        });
        crate::test_support::press(cx, "tab home");
        cx.run_until_parked();
        let viewport = cx.debug_bounds("selector-results");
        // The current profile is deliberately promoted ahead of catalog order.
        let first = cx.debug_bounds("switch-profile-profile-39");
        assert!(viewport.zip(first).is_some_and(
            |(viewport, row)| row.top() >= viewport.top() && row.bottom() <= viewport.bottom()
        ));
        crate::test_support::press(cx, "end");
        cx.run_until_parked();
        let last = cx.debug_bounds("switch-profile-profile-38");
        assert!(viewport.zip(last).is_some_and(
            |(viewport, row)| row.top() >= viewport.top() && row.bottom() <= viewport.bottom()
        ));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_profile.clone()),
            Some("profile-39".into())
        );
    }
}
