//! Browsing a record is separate from choosing the account or game used by Home.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Collection state belongs to private product screens."
)]

#[cfg(test)]
#[path = "browser_behavior_tests.rs"]
mod tests;

use super::scroll_area::ScrollAreaExt as _;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use block_launcher::{
    catalog::{Instance, LoaderKind, Profile, ProfileAppearance, ProfileMode},
    recipe::RuntimeSelection,
};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    Window, accesskit::Role, div, px, relative, rgb,
};

use super::{
    Block, Form, Tab, TextInput, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState},
    design::Icon,
    input, note, patterns,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CollectionLoad {
    Loading,
    #[default]
    Ready,
    Failed(String),
}

#[derive(Debug, Default)]
pub(super) struct CollectionState {
    pub query: Option<Entity<TextInput>>,
    pub inspected: Option<String>,
    pub detail_visible: bool,
    pub scroll: ScrollHandle,
    pub detail_scroll: ScrollHandle,
    pub load: CollectionLoad,
    row_focus: Rc<RefCell<HashMap<String, FocusHandle>>>,
}

impl CollectionState {
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        let detail_scroll = ScrollHandle::new();
        detail_scroll.set_offset(self.detail_scroll.offset());
        Self {
            query: self
                .query
                .as_ref()
                .map(|input| super::frozen_input(input, cx)),
            inspected: self.inspected.clone(),
            detail_visible: self.detail_visible,
            scroll,
            detail_scroll,
            load: self.load.clone(),
            row_focus: Rc::default(),
        }
    }

    fn focus(&self, id: &str, cx: &Context<'_, Block>) -> FocusHandle {
        self.row_focus
            .borrow_mut()
            .entry(id.to_owned())
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowserKind {
    Accounts,
    Bundled,
    Instances,
    Profiles,
}

impl BrowserKind {
    const fn title(self) -> &'static str {
        match self {
            Self::Accounts => "Your accounts",
            Self::Bundled => "Your games",
            Self::Instances => "Instances",
            Self::Profiles => "Profiles",
        }
    }
    const fn noun(self) -> &'static str {
        match self {
            Self::Accounts => "accounts",
            Self::Bundled => "games",
            Self::Instances => "Instances",
            Self::Profiles => "Profiles",
        }
    }
    const fn search(self) -> &'static str {
        match self {
            Self::Accounts => "Search accounts",
            Self::Bundled => "Search games",
            Self::Instances => "Search Instances",
            Self::Profiles => "Search Profiles",
        }
    }
}

fn category_panel(kind: BrowserKind, rows: impl IntoElement) -> gpui::AnyElement {
    if !matches!(kind, BrowserKind::Instances | BrowserKind::Profiles) {
        return rows.into_any_element();
    }
    div()
        .id("games-category-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(rows)
        .into_any_element()
}

#[derive(Clone, Copy, Debug)]
enum RowIcon {
    Account,
    Instance,
    Profile(ProfileAppearance),
}

#[derive(Clone, Debug)]
struct Record {
    id: String,
    selector: String,
    title: String,
    icon: RowIcon,
    badges: Vec<(String, BadgeVariant)>,
    current: bool,
}

impl Block {
    pub(super) fn initialize_browsers(&mut self, cx: &mut Context<'_, Self>) {
        let games = if self.snapshot.profile_mode == ProfileMode::Bundled {
            BrowserKind::Bundled
        } else {
            BrowserKind::Instances
        };
        for (state, kind) in [
            (&mut self.shell.games, games),
            (&mut self.shell.profiles, BrowserKind::Profiles),
            (&mut self.shell.accounts, BrowserKind::Accounts),
        ] {
            let label = format!("{}…", kind.search());
            if state.query.is_none() {
                state.query = Some(input("", &label, cx));
            }
            if let Some(field) = &state.query {
                field.update(cx, |input, cx| {
                    input.set_search(true, cx);
                    input.set_placeholder(&label, cx);
                });
            }
        }
    }

    pub(super) fn inspected_instance(&self) -> Option<&Instance> {
        if (self.tab == Tab::Profiles || self.snapshot.profile_mode == ProfileMode::Bundled)
            && let Some(profile) = self.inspected_profile()
        {
            return self
                .snapshot
                .instances
                .iter()
                .find(|instance| instance.id == profile.instance_id);
        }
        self.snapshot
            .instances
            .iter()
            .find(|instance| Some(&instance.id) == self.shell.games.inspected.as_ref())
            .or_else(|| self.instance())
            .or_else(|| self.snapshot.instances.first())
    }

    pub(super) fn inspected_profile(&self) -> Option<&Profile> {
        let inspected = if self.tab == Tab::Profiles {
            self.shell.profiles.inspected.as_ref()
        } else {
            self.shell.games.inspected.as_ref()
        };
        let direct = self
            .snapshot
            .profiles
            .iter()
            .find(|profile| Some(&profile.id) == inspected);
        if direct.is_some() {
            return direct;
        }
        if self.tab == Tab::Profiles {
            return self.profile().or_else(|| self.snapshot.profiles.first());
        }
        let instance = self
            .snapshot
            .instances
            .iter()
            .find(|instance| Some(&instance.id) == inspected)
            .or_else(|| self.instance())
            .or_else(|| self.snapshot.instances.first())?;
        self.profile()
            .filter(|profile| profile.instance_id == instance.id)
            .or_else(|| {
                self.snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.instance_id == instance.id)
            })
    }

    pub(super) const fn collection(&self) -> &CollectionState {
        match self.tab {
            Tab::Accounts => &self.shell.accounts,
            Tab::Profiles => &self.shell.profiles,
            _ => &self.shell.games,
        }
    }
    pub(super) const fn collection_mut(&mut self) -> &mut CollectionState {
        match self.tab {
            Tab::Accounts => &mut self.shell.accounts,
            Tab::Profiles => &mut self.shell.profiles,
            _ => &mut self.shell.games,
        }
    }

    fn browser_kind(&self) -> BrowserKind {
        if self.tab == Tab::Accounts {
            BrowserKind::Accounts
        } else if self.snapshot.profile_mode == ProfileMode::Bundled {
            BrowserKind::Bundled
        } else if self.tab == Tab::Profiles {
            BrowserKind::Profiles
        } else {
            BrowserKind::Instances
        }
    }

    fn inspected_record(&self) -> Option<String> {
        match self.browser_kind() {
            BrowserKind::Accounts => self
                .accounts
                .iter()
                .find(|account| Some(account_id(account)) == self.shell.accounts.inspected)
                .or_else(|| {
                    self.accounts
                        .iter()
                        .find(|account| Some(account_id(account)) == self.selected_account)
                })
                .or_else(|| self.accounts.first())
                .map(account_id),
            BrowserKind::Instances => self
                .inspected_instance()
                .map(|instance| instance.id.clone()),
            BrowserKind::Bundled | BrowserKind::Profiles => {
                self.inspected_profile().map(|profile| profile.id.clone())
            }
        }
    }

    fn instance_badges(
        &self,
        instance: &Instance,
        profile: Option<&Profile>,
    ) -> Vec<(String, BadgeVariant)> {
        let release = self
            .version_catalog
            .iter()
            .find(|version| version.id == instance.minecraft_version)
            .map(|version| version.kind);
        let variant = match release {
            Some(minecraft_manifest::model::VersionType::Release) => BadgeVariant::Success,
            Some(minecraft_manifest::model::VersionType::Snapshot) => BadgeVariant::Warning,
            _ => BadgeVariant::Secondary,
        };
        vec![
            (instance.minecraft_version.clone(), variant),
            (loader_value(instance, profile), BadgeVariant::Secondary),
        ]
    }

    fn browser_records(&self, cx: &gpui::App) -> Vec<Record> {
        let query = self
            .collection()
            .query
            .as_ref()
            .map_or_else(String::new, |field| {
                field.read(cx).value().trim().to_lowercase()
            });
        match self.browser_kind() {
            BrowserKind::Accounts => self
                .accounts
                .iter()
                .filter(|account| {
                    format!(
                        "{} {} {}",
                        account.player_name,
                        self.account_authority(account),
                        self.account_provider(account)
                            .map_or("", |provider| provider.api_url.as_str())
                    )
                    .to_lowercase()
                    .contains(&query)
                })
                .map(|account| {
                    let id = account_id(account);
                    Record {
                        selector: format!("account-inspect-{id}"),
                        current: self.selected_account.as_ref() == Some(&id),
                        id,
                        title: account.player_name.clone(),
                        icon: RowIcon::Account,
                        badges: vec![(self.account_authority(account), BadgeVariant::Secondary)],
                    }
                })
                .collect(),
            BrowserKind::Instances => self
                .snapshot
                .instances
                .iter()
                .filter(|instance| {
                    format!(
                        "{} {} {}",
                        instance.name,
                        instance.minecraft_version,
                        loader_value(instance, None)
                    )
                    .to_lowercase()
                    .contains(&query)
                })
                .map(|instance| Record {
                    id: instance.id.clone(),
                    selector: format!("instance-{}", instance.id),
                    title: instance.name.clone(),
                    icon: RowIcon::Instance,
                    badges: self.instance_badges(instance, None),
                    current: false,
                })
                .collect(),
            BrowserKind::Bundled | BrowserKind::Profiles => self
                .snapshot
                .profiles
                .iter()
                .filter_map(|profile| {
                    let instance = self
                        .snapshot
                        .instances
                        .iter()
                        .find(|instance| instance.id == profile.instance_id);
                    let text = format!(
                        "{} {} {} {}",
                        profile.name,
                        instance.map_or("", |instance| instance.name.as_str()),
                        instance.map_or("", |instance| instance.minecraft_version.as_str()),
                        instance.map_or_else(String::new, |instance| loader_value(
                            instance,
                            Some(profile)
                        ))
                    );
                    if !text.to_lowercase().contains(&query) {
                        return None;
                    }
                    Some(Record {
                        id: profile.id.clone(),
                        selector: if self.browser_kind() == BrowserKind::Bundled {
                            format!("instance-{}", profile.instance_id)
                        } else {
                            format!("profile-{}", profile.id)
                        },
                        title: profile.name.clone(),
                        icon: RowIcon::Profile(profile.appearance),
                        badges: instance.map_or_else(
                            || vec![("Instance unavailable".into(), BadgeVariant::Destructive)],
                            |instance| self.instance_badges(instance, Some(profile)),
                        ),
                        current: self.selected_profile.as_ref() == Some(&profile.id),
                    })
                })
                .collect(),
        }
    }

    fn browser_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if event.keystroke.modifiers != gpui::Modifiers::none()
            || event.is_held
            || self
                .collection()
                .query
                .as_ref()
                .is_some_and(|query| query.read(cx).is_composing())
        {
            return;
        }
        let records = self.browser_records(cx);
        if records.is_empty() {
            return;
        }
        let current = self.inspected_record();
        let index = records
            .iter()
            .position(|record| Some(&record.id) == current.as_ref());
        let search_focused = self
            .collection()
            .query
            .as_ref()
            .is_some_and(|field| field.read(cx).focus_handle(cx).is_focused(window));
        if !search_focused
            && !records
                .iter()
                .any(|record| self.collection().focus(&record.id, cx).is_focused(window))
        {
            return;
        }
        let index =
            match event.keystroke.key.as_str() {
                "down" if search_focused => 0,
                "up" if search_focused => records.len() - 1,
                "down" => index.map_or(0, |index| (index + 1).min(records.len() - 1)),
                "up" => index.map_or(records.len() - 1, |index| index.saturating_sub(1)),
                "home"
                    if !self.collection().query.as_ref().is_some_and(|field| {
                        field.read(cx).focus_handle(cx).is_focused(window)
                    }) =>
                {
                    0
                }
                "end"
                    if !self.collection().query.as_ref().is_some_and(|field| {
                        field.read(cx).focus_handle(cx).is_focused(window)
                    }) =>
                {
                    records.len() - 1
                }
                _ => return,
            };
        self.collection_mut().inspected = Some(records[index].id.clone());
        self.collection()
            .focus(&records[index].id, cx)
            .focus(window, cx);
        self.collection().scroll.scroll_to_item(index);
        self.motion_dirty = true;
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn render_browser_detail(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let viewport = window.viewport_size();
        let kind = self.browser_kind();
        let state = self.collection();
        let records = self.browser_records(cx);
        let inspected = self.inspected_record();
        let roving = records
            .iter()
            .find(|record| Some(&record.id) == inspected.as_ref())
            .or_else(|| records.first())
            .map(|record| record.id.as_str());
        let count = match kind {
            BrowserKind::Accounts => self.accounts.len(),
            BrowserKind::Instances => self.snapshot.instances.len(),
            BrowserKind::Bundled | BrowserKind::Profiles => self.snapshot.profiles.len(),
        };
        let compact = patterns::is_narrow(viewport);
        let heading = self.browser_heading(kind, count, compact, cx);
        let rows = self.browser_rows(kind, count, &records, roving, cx);
        let rows = category_panel(kind, rows);
        let footer = self.browser_footer(kind, count, compact, viewport, cx);
        let browser = div()
            .id("browser-pane")
            .debug_selector(|| "browser-pane".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::browser_key))
            .child(heading)
            .child(rows)
            .child(
                footer
                    .id("browser-footer")
                    .debug_selector(|| "browser-footer".into()),
            );
        let detail = self.transition_content(
            super::motion::Scope::Detail,
            self.browser_detail_content(window, cx),
            window,
            cx,
        );
        if compact
            && (matches!(self.form, Some(Form::Profile(_))) || self.shell.instance_editor.is_some())
        {
            return div()
                .size_full()
                .min_h_0()
                .flex()
                .child(
                    div()
                        .id("compact-browser")
                        .debug_selector(|| "compact-browser".into())
                        .w(px(56.))
                        .h_full()
                        .min_h_0()
                        .flex_shrink_0()
                        .border_r_1()
                        .border_color(rgb(self.palette.border))
                        .bg(rgb(self.palette.sidebar))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .id("compact-list")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .children(
                                    records
                                        .iter()
                                        .map(|record| self.browser_icon_row(record, cx)),
                                ),
                        )
                        .child(patterns::management_footer(viewport, self.palette).px_0()),
                )
                .child(div().flex_1().min_w_0().h_full().child(detail));
        }
        patterns::management_layout(
            viewport,
            browser.into_any_element(),
            detail,
            state.detail_visible || self.form.is_some(),
            self.palette,
        )
    }

    fn browser_heading(
        &self,
        kind: BrowserKind,
        count: usize,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let state = self.collection();
        let mut heading = div()
            .p(px(if compact { 8. } else { 16. }))
            .flex()
            .flex_col()
            .gap(px(if compact { 8. } else { 16. }));
        if kind == BrowserKind::Accounts || self.snapshot.profile_mode == ProfileMode::Bundled {
            if !compact {
                heading = heading.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(note(kind.title()))
                        .child(if kind == BrowserKind::Accounts {
                            note(count.to_string()).into_any_element()
                        } else {
                            self.browser_help(cx).into_any_element()
                        }),
                );
            }
        } else {
            heading = heading.child(
                div()
                    .id("games-tabs")
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded_lg()
                            .p_1()
                            .bg(rgb(self.palette.secondary))
                            .flex()
                            .gap_1()
                            .child(self.browser_tab(
                                Tab::Instances,
                                "games-tab-instances",
                                "Instance",
                                cx,
                            ))
                            .child(self.browser_tab(
                                Tab::Profiles,
                                "games-tab-profiles",
                                "Profile",
                                cx,
                            )),
                    )
                    .child(self.browser_help(cx)),
            );
        }
        let create = if kind == BrowserKind::Accounts {
            UiAction::SignIn
        } else if kind == BrowserKind::Profiles {
            UiAction::NewProfile
        } else {
            UiAction::CreateInstance
        };
        let create_label = if kind == BrowserKind::Accounts {
            "Add account"
        } else if kind == BrowserKind::Profiles {
            "Create Profile"
        } else {
            "Create game setup"
        };
        heading = heading.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(
                    state
                        .query
                        .as_ref()
                        .map(|field| div().flex_1().min_w_0().child(field.clone())),
                )
                .child(
                    self.browser_control(
                        if kind == BrowserKind::Accounts {
                            "sign-in"
                        } else {
                            "create-instance"
                        },
                        create_label,
                        create,
                        ButtonVariant::Outline,
                        self.busy,
                        cx,
                    )
                    .w(px(32.))
                    .p_0()
                    .child(Icon::Plus.render(16., self.palette.foreground)),
                ),
        );
        heading
    }

    fn browser_rows(
        &self,
        kind: BrowserKind,
        count: usize,
        records: &[Record],
        roving: Option<&str>,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let state = self.collection();
        let mut rows = div()
            .id("collection-list")
            .debug_selector(|| "collection-list".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pb_3();
        if records.is_empty() && matches!(state.load, CollectionLoad::Loading) {
            rows = rows.child(self.browser_status("Loading saved records…", Icon::Loading));
        } else if records.is_empty() && !matches!(state.load, CollectionLoad::Failed(_)) {
            let message = if count == 0 {
                format!("No {} yet.", kind.noun())
            } else {
                format!("No {} found. Try another search.", kind.noun())
            };
            rows = rows.child(self.browser_status(
                message,
                if kind == BrowserKind::Accounts {
                    Icon::Account
                } else {
                    Icon::Search
                },
            ));
        } else {
            rows =
                rows.children(records.iter().map(|record| {
                    self.browser_row(record, roving == Some(record.id.as_str()), cx)
                }));
        }
        if let CollectionLoad::Failed(error) = &state.load {
            rows = rows.child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(note(error.clone()).text_color(rgb(self.palette.destructive)))
                    .child(self.browser_control(
                        "reload-library",
                        "Try again",
                        UiAction::ReloadLibrary,
                        ButtonVariant::Outline,
                        false,
                        cx,
                    )),
            );
        }
        rows.scroll_list_area(&state.scroll, "Library records", self.palette)
    }

    fn browser_footer(
        &self,
        kind: BrowserKind,
        count: usize,
        compact: bool,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut footer = patterns::management_footer(viewport, self.palette).px(px(if compact {
            12.
        } else {
            20.
        }));
        if kind == BrowserKind::Accounts {
            footer = footer.child(
                self.browser_control(
                    "authlib-providers",
                    "Authlib providers",
                    UiAction::Providers,
                    ButtonVariant::Ghost,
                    false,
                    cx,
                )
                .child(Icon::Server.render(14., self.palette.muted)),
            );
        } else {
            footer = footer
                .justify_between()
                .child(note(format!("{count} {}", kind.noun())))
                .child(
                    note(if self.snapshot.profile_mode == ProfileMode::Bundled {
                        "Bundled"
                    } else {
                        "Separated"
                    })
                    .text_size(px(10.)),
                );
        }
        footer
    }

    fn browser_control(
        &self,
        id: impl Into<gpui::SharedString>,
        label: &str,
        action: UiAction,
        variant: ButtonVariant,
        disabled: bool,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        let id = id.into();
        let text = if matches!(id.as_ref(), "sign-in" | "create-instance") {
            ""
        } else {
            label
        };
        components::button(
            id,
            text.to_owned(),
            self.palette,
            variant,
            ControlState {
                disabled,
                ..Default::default()
            },
            cx.listener(move |view, (): &(), window, cx| view.handle(action.clone(), window, cx)),
        )
    }

    fn browser_tab(
        &self,
        tab: Tab,
        id: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::button_role(
            id,
            label,
            self.palette,
            ButtonVariant::Ghost,
            ControlState::default(),
            Role::Tab,
            cx.listener(move |view, (): &(), window, cx| {
                view.handle(UiAction::Tab(tab), window, cx);
                view.browser_tab_focus(tab, cx).focus(window, cx);
            }),
        )
        .track_focus(&self.browser_tab_focus(tab, cx).tab_stop(self.tab == tab))
        .tab_index(if self.tab == tab { 0 } else { -1 })
        .flex_1()
        .bg(rgb(if self.tab == tab {
            self.palette.background
        } else {
            self.palette.secondary
        }))
        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
            if !event.is_held
                && event.keystroke.modifiers == gpui::Modifiers::none()
                && matches!(
                    event.keystroke.key.as_str(),
                    "left" | "right" | "home" | "end"
                )
            {
                let next = match event.keystroke.key.as_str() {
                    "home" => Tab::Instances,
                    "end" => Tab::Profiles,
                    _ if view.tab == Tab::Instances => Tab::Profiles,
                    _ => Tab::Instances,
                };
                view.handle(UiAction::Tab(next), window, cx);
                view.browser_tab_focus(next, cx).focus(window, cx);
                cx.stop_propagation();
            }
        }))
    }

    fn browser_tab_focus(&self, tab: Tab, cx: &Context<'_, Self>) -> FocusHandle {
        self.shell.games.focus(
            if tab == Tab::Instances {
                "category:instances"
            } else {
                "category:profiles"
            },
            cx,
        )
    }

    fn browser_help(&self, cx: &Context<'_, Self>) -> components::Control {
        let control = components::button(
            "games-help",
            "",
            self.palette,
            ButtonVariant::Ghost,
            ControlState::default(),
            cx.listener(|view, (): &(), window, cx| view.handle(UiAction::GamesHelp, window, cx)),
        )
        .w(px(28.))
        .p_0()
        .child(Icon::Help.render(16., self.palette.muted));
        super::tooltip::trigger(control, "What’s the difference?", cx)
    }

    fn browser_row(
        &self,
        record: &Record,
        roving: bool,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        let inspected = self.inspected_record().as_deref() == Some(record.id.as_str());
        let id = record.id.clone();
        let focus = self.collection().focus(&record.id, cx).tab_stop(roving);
        let mut row = components::button_role(
            record.selector.clone(),
            "",
            self.palette,
            ButtonVariant::Ghost,
            ControlState::default(),
            Role::ListBoxOption,
            cx.listener(move |view, (): &(), window, cx| {
                view.handle(UiAction::Inspect(id.clone()), window, cx);
            }),
        )
        .track_focus(&focus)
        .w_full()
        .h_auto()
        .min_h(px(80.))
        .rounded_none()
        .border_0()
        .px_5()
        .py_4()
        .flex()
        .items_center()
        .gap_3()
        .bg(rgb(if inspected {
            self.palette.secondary
        } else {
            self.palette.sidebar
        }))
        .child(self.record_avatar(record.icon, &record.title, 40.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(6.))
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(record.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(record.badges.iter().map(|(value, variant)| {
                            self.record_badge(
                                value.clone(),
                                *variant,
                                matches!(record.icon, RowIcon::Account),
                            )
                        })),
                ),
        );
        row = row.child(
            div().w(px(16.)).flex_shrink_0().children(
                record
                    .current
                    .then(|| Icon::Check.render(16., self.palette.foreground)),
            ),
        );
        row
    }

    fn record_avatar(&self, icon: RowIcon, name: &str, size: f32) -> gpui::Div {
        match icon {
            RowIcon::Profile(appearance) => {
                components::profile_avatar(appearance, self.palette, size)
            }
            RowIcon::Instance => div()
                .size(px(size))
                .flex_shrink_0()
                .rounded_lg()
                .bg(rgb(self.palette.secondary))
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::Blocks.render(size * 0.5, self.palette.muted)),
            RowIcon::Account => {
                components::avatar_fallback(name, self.palette, size, AvatarState::Fallback)
                    .rounded(px(if size > 40. { 16. } else { 8. }))
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .left(px(-3.))
                            .top(px(-3.))
                            .size(px(16.))
                            .rounded_full()
                            .bg(rgb(0x0025_63eb))
                            .border_2()
                            .border_color(rgb(self.palette.background))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::Grid.render(10., 0x00ff_ffff)),
                    )
            }
        }
    }

    fn browser_status(
        &self,
        label: impl Into<gpui::SharedString>,
        icon: Icon,
    ) -> gpui::Stateful<gpui::Div> {
        let label = label.into();
        div()
            .id("browser-status")
            .p_5()
            .py_8()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .text_center()
            .text_sm()
            .text_color(rgb(self.palette.muted))
            .child(icon.render(20., self.palette.muted))
            .child(label)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "The detail composition keeps its type-specific body and real footer actions together."
    )]
    pub(super) fn browser_detail_content(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        if self.shell.instance_editor.is_some() {
            return self.render_instance_editor(window, cx);
        }
        if matches!(self.form, Some(Form::Profile(_))) {
            return self.render_profile_editor(window, cx);
        }
        let viewport = window.viewport_size();
        let narrow = patterns::is_narrow(viewport);
        let mut body = div()
            .id("main-content")
            .debug_selector(|| "management-detail".into())
            .flex_1()
            .min_h_0()
            .scroll_area(
                &self.collection().detail_scroll,
                "Library details",
                self.palette,
                false,
            )
            .track_focus(&self.browser_detail_focus(cx))
            .flex()
            .flex_col()
            .gap(px(if narrow { 20. } else { 24. }))
            .p(px(if narrow { 16. } else { 32. }));
        if narrow {
            body = body.child(
                self.button(
                    "browser-back",
                    if self.tab == Tab::Accounts {
                        "Back to accounts"
                    } else {
                        "Back to games"
                    },
                    UiAction::BrowserBack,
                    cx,
                )
                .items_start()
                .child(Icon::ArrowLeft.render(14., self.palette.foreground)),
            );
        }
        let mut footer = patterns::management_footer(viewport, self.palette).justify_between();
        if self.tab == Tab::Accounts {
            if let Some(account) = self
                .accounts
                .iter()
                .find(|account| Some(account_id(account)) == self.inspected_record())
            {
                let id = account_id(account);
                let current = self.selected_account.as_ref() == Some(&id);
                let offline = account.provider == block_auth::AccountProvider::Offline;
                let external = account.provider == block_auth::AccountProvider::Authlib;
                body = body.child(self.detail_identity(RowIcon::Account, &account.player_name, vec![(self.account_authority(account), BadgeVariant::Secondary)], current))
                    .child(Self::detail_metadata("Account details", &[("Player name", account.player_name.clone()), ("Account type", account.provider.label().into()), ("Authentication", if offline { "Local identity; no online authorization" } else if external { "External authentication provider" } else { "Ownership rechecked at launch" }.into()), ("Environment", "Minecraft: Java Edition".into())], narrow))
                    .child(div().border_t_1().border_color(rgb(self.palette.border)).pt_5().flex().flex_col().gap_2()
                        .child(note(if offline { "A local player identity for offline play." } else if external { "An identity issued by an external authentication provider." } else { "A Microsoft identity for Minecraft: Java Edition." }))
                        .child(note(if offline { "The UUID is derived from the exact player name. No credentials are stored. This identity cannot access authenticated online services." } else if external { "Provider credentials must be verified before launch. Access and client tokens belong in your system credential store." } else { "Sign-in and ownership are checked again when launching. Credentials remain in your system’s secure credential store." })));
                if external {
                    body = body.child(Self::detail_metadata(
                        "Provider",
                        &[
                            ("Name", self.account_authority(account)),
                            (
                                "API address",
                                self.account_provider(account).map_or_else(
                                    || "Provider unavailable".into(),
                                    |provider| provider.api_url.clone(),
                                ),
                            ),
                            (
                                "Account",
                                account
                                    .authlib
                                    .as_ref()
                                    .map_or_else(String::new, |link| link.login.clone()),
                            ),
                        ],
                        narrow,
                    ));
                }
                footer = footer
                    .child(self.browser_control(
                        format!("account-logout-{id}"),
                        "Remove account",
                        UiAction::RequestSignOut(id.clone()),
                        ButtonVariant::Outline,
                        self.busy,
                        cx,
                    ))
                    .child(self.browser_control(
                        format!("account-select-{id}"),
                        if current {
                            "Current account"
                        } else {
                            "Use account"
                        },
                        UiAction::SelectAccount(id),
                        ButtonVariant::Primary,
                        current || self.busy,
                        cx,
                    ));
            } else {
                return self.empty_browser_detail(window, true, cx);
            }
        } else if let Some(instance) = self.inspected_instance() {
            let profile = self.inspected_profile();
            let kind = self.browser_kind();
            if kind == BrowserKind::Instances {
                body = body
                    .child(self.detail_identity(
                        RowIcon::Instance,
                        &instance.name,
                        self.instance_badges(instance, None),
                        false,
                    ))
                    .child(self.instance_components(instance, profile, narrow, cx))
                    .child(self.linked_profiles(instance, cx));
                if let Some(notice) = self.installation_notice(instance, None) {
                    body = body.child(notice);
                }
                footer = footer.child(note("Separated Profile")).child(
                    self.browser_control(
                        "new-profile",
                        "New Profile",
                        UiAction::NewProfile,
                        ButtonVariant::Primary,
                        self.busy,
                        cx,
                    )
                    .child(Icon::Plus.render(16., self.palette.on_primary)),
                );
            } else if let Some(profile) = profile {
                let current = self.selected_profile.as_ref() == Some(&profile.id);
                body = body.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(
                            self.detail_identity(
                                RowIcon::Profile(profile.appearance),
                                &profile.name,
                                self.instance_badges(instance, Some(profile)),
                                current,
                            )
                            .flex_1()
                            .min_w_0(),
                        )
                        .child(
                            self.button("profile-actions", "", UiAction::ProfileActions, cx)
                                .w(px(32.))
                                .p_0()
                                .child(Icon::More.render(16., self.palette.foreground)),
                        ),
                );
                if kind == BrowserKind::Bundled {
                    body =
                        body.child(self.instance_components(instance, Some(profile), narrow, cx));
                } else {
                    body = body.child(self.linked_instance(instance, cx));
                }
                body = body.child(self.launch_details(profile, narrow));
                if let Some(notice) = self.installation_notice(instance, Some(profile)) {
                    body = body.child(notice);
                }
                footer = footer
                    .child(
                        self.browser_control(
                            "edit-profile",
                            if kind == BrowserKind::Bundled {
                                "Edit launch settings"
                            } else {
                                "Edit Profile"
                            },
                            UiAction::EditProfile,
                            ButtonVariant::Outline,
                            self.busy,
                            cx,
                        )
                        .track_focus(&self.profile_editor_trigger_focus(cx)),
                    )
                    .child(self.browser_control(
                        "use-profile",
                        if current {
                            if kind == BrowserKind::Bundled {
                                "Current game"
                            } else {
                                "Current Profile"
                            }
                        } else if kind == BrowserKind::Bundled {
                            "Use game"
                        } else {
                            "Use Profile"
                        },
                        UiAction::SelectProfile(profile.id.clone()),
                        ButtonVariant::Primary,
                        current || self.busy,
                        cx,
                    ));
            } else {
                return self.empty_browser_detail(window, false, cx);
            }
        } else {
            return self.empty_browser_detail(window, false, cx);
        }
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(body)
            .child(
                footer
                    .id("detail-footer")
                    .debug_selector(|| "detail-footer".into()),
            )
            .into_any_element()
    }

    pub(super) fn reconcile_account_browser(&mut self, next: &[block_auth::Account]) {
        let Some(inspected) = &self.shell.accounts.inspected else {
            return;
        };
        if next.iter().any(|account| account_id(account) == *inspected) {
            return;
        }
        let index = self
            .accounts
            .iter()
            .position(|account| account_id(account) == *inspected)
            .unwrap_or(0);
        self.shell.accounts.inspected = next.get(index).or_else(|| next.last()).map(account_id);
    }

    fn installation_notice(
        &self,
        instance: &Instance,
        profile: Option<&Profile>,
    ) -> Option<gpui::Div> {
        let targets: Vec<_> = instance
            .targets
            .iter()
            .filter(|target| profile.is_none_or(|profile| profile.target_id == target.id))
            .filter(|target| {
                !matches!(target.status, block_launcher::catalog::InstallStatus::Ready)
            })
            .collect();
        if targets.is_empty() {
            return None;
        }
        Some(div().flex().flex_col().gap_2().border_t_1().border_color(rgb(self.palette.border)).pt_4().child(note("Installation"))
            .children(targets.into_iter().map(|target| {
                let (message, error) = match &target.status {
                    block_launcher::catalog::InstallStatus::Pending => ("Game files will install when you launch from Home.".to_owned(), false),
                    block_launcher::catalog::InstallStatus::Installing => ("Game files are being installed. Download progress is available on Home.".to_owned(), false),
                    block_launcher::catalog::InstallStatus::Failed { message } => (format!("Installation failed: {message}. Choose this game, then repair it from Home."), true),
                    block_launcher::catalog::InstallStatus::Ready => (String::new(), false),
                };
                div().id(gpui::SharedString::from(format!("installation-status-{}", target.id)))
                    .child(note(message).text_color(rgb(if error { self.palette.destructive } else { self.palette.muted })))
            })))
    }

    pub(super) fn browser_detail_focus(&self, cx: &Context<'_, Self>) -> FocusHandle {
        self.collection()
            .focus("detail-heading", cx)
            .tab_stop(false)
    }
    pub(super) fn profile_editor_trigger_focus(&self, cx: &Context<'_, Self>) -> FocusHandle {
        self.collection()
            .focus("profile-editor-trigger", cx)
            .tab_stop(true)
    }

    pub(super) fn restore_browser_focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.inspected_record() {
            self.collection().focus(&id, cx).focus(window, cx);
        } else if let Some(query) = &self.collection().query {
            query.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    fn browser_icon_row(&self, record: &Record, cx: &Context<'_, Self>) -> components::Control {
        let id = record.id.clone();
        let control = components::button(
            record.selector.clone(),
            "",
            self.palette,
            ButtonVariant::Ghost,
            ControlState::default(),
            cx.listener(move |view, (): &(), window, cx| {
                view.handle(UiAction::Inspect(id.clone()), window, cx);
            }),
        )
        .w_full()
        .h(px(64.))
        .p_0()
        .rounded_none()
        .border_0()
        .child(self.record_avatar(record.icon, &record.title, 32.));
        super::tooltip::trigger(control, record.title.clone(), cx)
    }

    fn record_badge(&self, label: String, variant: BadgeVariant, microsoft: bool) -> gpui::Div {
        let badge = components::badge(label, self.palette, variant);
        if microsoft {
            let dark = self.palette.background == super::Palette::DARK.background;
            badge
                .bg(rgb(if dark { 0x0026_303f } else { 0x00eb_f2fc }))
                .text_color(rgb(if dark { 0x00af_c4e4 } else { 0x0049_6b9a }))
                .border_0()
        } else {
            badge
        }
    }

    fn detail_identity(
        &self,
        icon: RowIcon,
        name: &str,
        badges: Vec<(String, BadgeVariant)>,
        current: bool,
    ) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_4()
            .child(self.record_avatar(
                icon,
                name,
                if matches!(icon, RowIcon::Account) {
                    80.
                } else {
                    56.
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name.to_owned()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .children(badges.into_iter().map(|(value, variant)| {
                                self.record_badge(value, variant, matches!(icon, RowIcon::Account))
                            }))
                            .children(current.then(|| {
                                components::badge("In use", self.palette, BadgeVariant::Success)
                                    .gap_1()
                                    .child(Icon::Check.render(12., self.palette.success))
                            })),
                    ),
            )
    }

    fn detail_metadata(title: &str, values: &[(&str, String)], narrow: bool) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_y_6()
                    .children(values.iter().map(|(label, value)| {
                        div()
                            .w(relative(if narrow { 1. } else { 0.5 }))
                            .min_w_0()
                            .pr_4()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(note((*label).to_owned()))
                            .child(div().text_sm().child(value.clone()))
                    })),
            )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One composition defines the four component cards and their immutable or editable behavior."
    )]
    fn instance_components(
        &self,
        instance: &Instance,
        profile: Option<&Profile>,
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut grid = div().flex().flex_wrap().gap_3();
        let width = if narrow { 1. } else { 0.48 };
        for (id, icon, label, value, description, action) in [
            (
                "component-minecraft",
                Icon::Blocks,
                "Minecraft",
                instance.minecraft_version.clone(),
                "Fixed at creation".to_owned(),
                None,
            ),
            (
                "component-loader",
                Icon::Layers,
                "Mod loader",
                loader_value(instance, profile),
                "Fixed at creation".to_owned(),
                None,
            ),
            (
                "component-optifine",
                Icon::Experiment,
                "OptiFine",
                instance
                    .targets
                    .iter()
                    .find(|target| profile.is_some_and(|profile| profile.target_id == target.id))
                    .and_then(|target| target.optifine.as_ref())
                    .map_or_else(
                        || "Not installed".to_owned(),
                        |selected| selected.version.clone(),
                    ),
                "Fixed at creation".to_owned(),
                None,
            ),
            (
                "component-instance-settings",
                Icon::Settings,
                "Instance settings",
                instance
                    .settings
                    .as_ref()
                    .map(|settings| &settings.runtime)
                    .or_else(|| profile.map(|profile| &profile.runtime))
                    .map_or("Automatic Java", |runtime| {
                        if matches!(runtime, RuntimeSelection::Managed) {
                            "Automatic Java"
                        } else {
                            "Custom Java"
                        }
                    })
                    .to_owned(),
                instance.directory.display().to_string(),
                Some(UiAction::EditInstance),
            ),
        ] {
            let contents = div()
                .flex()
                .flex_col()
                .items_start()
                .gap_3()
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon.render(16., self.palette.muted))
                        .child(note(label))
                        .children(
                            (label == "Minecraft" || label == "Mod loader")
                                .then(|| Icon::Lock.render(14., self.palette.muted)),
                        ),
                )
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(value))
                .child(note(description));
            if let Some(action) = action {
                grid = grid.child(
                    components::button(
                        id,
                        "",
                        self.palette,
                        ButtonVariant::Ghost,
                        ControlState {
                            disabled: self.busy,
                            ..Default::default()
                        },
                        cx.listener(move |view, (): &(), window, cx| {
                            view.handle(action.clone(), window, cx);
                        }),
                    )
                    .w(relative(width))
                    .h_auto()
                    .min_h(px(112.))
                    .p_4()
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .rounded_xl()
                    .child(contents),
                );
            } else {
                grid = grid.child(
                    components::card(self.palette)
                        .id(id)
                        .w(relative(width))
                        .min_h(px(112.))
                        .p_4()
                        .bg(rgb(self.palette.background))
                        .child(contents),
                );
            }
        }
        div().flex().flex_col().gap_3().child(grid).child(note("Minecraft version and mod loader are fixed. Create a new Instance for a different game setup."))
    }

    fn linked_profiles(&self, instance: &Instance, cx: &Context<'_, Self>) -> gpui::Div {
        let profiles: Vec<_> = self
            .snapshot
            .profiles
            .iter()
            .filter(|profile| profile.instance_id == instance.id)
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(note(format!("Used by {} Profiles", profiles.len())))
                    .child(self.button(
                        "instance-new-profile",
                        "New Profile",
                        UiAction::NewProfile,
                        cx,
                    )),
            )
            .children(profiles.into_iter().map(|profile| {
                self.button(
                    format!("linked-profile-{}", profile.id),
                    "",
                    UiAction::InspectLinked {
                        profiles: true,
                        id: profile.id.clone(),
                    },
                    cx,
                )
                .h_auto()
                .w_full()
                .p_2()
                .rounded_lg()
                .border_0()
                .bg(rgb(self.palette.background))
                .flex()
                .items_center()
                .gap_3()
                .child(components::profile_avatar(
                    profile.appearance,
                    self.palette,
                    32.,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(profile.name.clone()),
                )
                .child(Icon::ChevronRight.render(16., self.palette.muted))
            }))
    }

    fn linked_instance(&self, instance: &Instance, cx: &Context<'_, Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(note("Game Instance"))
            .child(
                self.button(
                    "linked-instance",
                    "",
                    UiAction::InspectLinked {
                        profiles: false,
                        id: instance.id.clone(),
                    },
                    cx,
                )
                .h_auto()
                .w_full()
                .p_4()
                .rounded_xl()
                .flex()
                .items_center()
                .gap_3()
                .child(self.record_avatar(RowIcon::Instance, &instance.name, 40.))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(instance.name.clone())
                        .child(note("Shared game files and components")),
                )
                .child(Icon::ExternalLink.render(16., self.palette.muted)),
            )
    }

    fn launch_details(&self, profile: &Profile, narrow: bool) -> gpui::Div {
        let server = (!profile.server.address.is_empty()).then(|| profile.server.address.clone());
        let width = if narrow { 1. } else { 0.48 };
        let card = |icon: Icon, label: &str, value: String, description: &str| {
            div()
                .w(relative(width))
                .min_w_0()
                .rounded_xl()
                .p_4()
                .bg(rgb(self.palette.secondary))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon.render(16., self.palette.muted))
                        .child(note(label.to_owned())),
                )
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(value))
                .child(note(description.to_owned()))
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Launch settings"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(card(
                        Icon::Server,
                        "Server",
                        server.unwrap_or_else(|| "Choose in game".into()),
                        if profile.server.connect_on_launch {
                            "Connect automatically"
                        } else {
                            "No automatic connection"
                        },
                    ))
                    .child(card(
                        Icon::Cpu,
                        "Memory",
                        format!("{} MiB", profile.memory_mib),
                        "Maximum allocation",
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::Terminal.render(14., self.palette.muted))
                    .child(note("JVM arguments")),
            )
            .child(
                div()
                    .rounded_lg()
                    .p_3()
                    .bg(rgb(self.palette.secondary))
                    .text_xs()
                    .child(if profile.jvm_args.is_empty() {
                        "Default arguments".into()
                    } else {
                        profile.jvm_args.join(" ")
                    }),
            )
    }

    fn empty_browser_detail(
        &self,
        window: &Window,
        accounts: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let loading = matches!(self.collection().load, CollectionLoad::Loading);
        let failed = matches!(self.collection().load, CollectionLoad::Failed(_));
        let title = if loading {
            "Loading your library"
        } else if failed {
            "Your library could not be loaded"
        } else {
            "Your next adventure starts here"
        };
        let description = if loading {
            "Reading saved records from this data directory."
        } else if failed {
            "Try again to read your saved records. Existing data has not been changed."
        } else if accounts {
            "Add an account to get ready to play."
        } else {
            "Create a game setup to get ready to play."
        };
        let mut content = patterns::empty_state(title, description, self.palette).child(
            if accounts { Icon::Account } else { Icon::Games }.render(32., self.palette.muted),
        );
        if !loading {
            content = content.child(self.browser_control(
                "welcome-create",
                if failed {
                    "Try again"
                } else if accounts {
                    "Add account"
                } else {
                    "Create game"
                },
                if failed {
                    UiAction::ReloadLibrary
                } else if accounts {
                    UiAction::SignIn
                } else {
                    UiAction::CreateInstance
                },
                ButtonVariant::Primary,
                self.busy,
                cx,
            ));
        }
        let mut section = div().size_full().min_h_0().flex().flex_col();
        if patterns::is_narrow(window.viewport_size()) {
            section = section.child(self.button(
                "browser-back",
                "Back to list",
                UiAction::BrowserBack,
                cx,
            ));
        }
        section
            .child(content)
            .child(patterns::management_footer(
                window.viewport_size(),
                self.palette,
            ))
            .into_any_element()
    }

    pub(super) fn render_games_help(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        div().flex().flex_col().gap_4().child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("The game, and how you launch it."))
            .child(note("An Instance fixes the game version and mod loader. A Profile stores your launch preferences."))
            .child(div().id("games-help-body").min_h_0().max_h(px(320.)).overflow_y_scroll().flex().flex_col().gap_3()
                .child(components::card(self.palette).p_3().child("Instance").child(note("Minecraft version and mod loader are fixed at creation. Its game folder and installation settings belong to this game setup.")))
                .child(components::card(self.palette).p_3().child("Bundled Profile").child(note("Every Instance has exactly one Profile. Games shows them together in one list.")))
                .child(components::card(self.palette).p_3().child("Separated Profile").child(note("Several Profiles can share one Instance, with independent accounts, memory and JVM arguments. You can convert from Bundled once; it cannot be reversed.")))
                .child(note(format!("Current mode: {} Profile", if self.snapshot.profile_mode == ProfileMode::Bundled { "Bundled" } else { "Separated" }))))
            .child(self.button("games-help-settings", "Profile mode", UiAction::ProfileModeSettings, cx)).into_any_element()
    }
}

fn loader_value(instance: &Instance, profile: Option<&Profile>) -> String {
    let target = profile
        .and_then(|profile| {
            instance
                .targets
                .iter()
                .find(|target| target.id == profile.target_id)
        })
        .or_else(|| {
            instance
                .targets
                .iter()
                .find(|target| target.kind != LoaderKind::Vanilla)
        })
        .or_else(|| instance.targets.first());
    target.map_or_else(
        || "No launch target".into(),
        block_launcher::catalog::LaunchTarget::display_name,
    )
}
