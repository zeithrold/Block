//! Browsing a record is separate from choosing the account or game used by Home.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Collection state belongs to private product screens."
)]

#[cfg(test)]
#[path = "browser_behavior_tests.rs"]
mod tests;

#[path = "../accounts/browser_accounts.rs"]
mod accounts;

use super::scroll_area::ScrollAreaExt as _;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use block_launcher::{
    catalog::{Instance, LoaderKind, Profile, ProfileAppearance, ProfileMode},
    recipe::RuntimeSelection,
};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, ScrollHandle, Styled as _, Window, accesskit::Role, div, px,
    relative, rgb,
};

use super::{
    Block, Form, Language, Tab, TextInput, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState},
    design::Icon,
    input, note, patterns,
};

type DetailBody = super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CollectionLoad {
    Loading,
    #[default]
    Ready,
    Failed(crate::i18n::UiText),
}

#[derive(Debug, Default)]
pub(super) struct CollectionState {
    pub query: Option<Entity<TextInput>>,
    pub inspected: Option<String>,
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
    /// Semantic keys, so every locale keeps its own wording for a kind.
    const fn title_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.title.accounts",
            Self::Bundled => "games.browser.title.games",
            Self::Instances => "games.browser.title.instances",
            Self::Profiles => "games.browser.title.profiles",
        }
    }
    const fn search_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.search.accounts",
            Self::Bundled => "games.browser.search.games",
            Self::Instances => "games.browser.search.instances",
            Self::Profiles => "games.browser.search.profiles",
        }
    }
    const fn empty_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.empty.accounts",
            Self::Bundled => "games.browser.empty.games",
            Self::Instances => "games.browser.empty.instances",
            Self::Profiles => "games.browser.empty.profiles",
        }
    }
    const fn no_match_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.no_match.accounts",
            Self::Bundled => "games.browser.no_match.games",
            Self::Instances => "games.browser.no_match.instances",
            Self::Profiles => "games.browser.no_match.profiles",
        }
    }
    const fn count_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.count.accounts",
            Self::Bundled => "games.browser.count.games",
            Self::Instances => "games.browser.count.instances",
            Self::Profiles => "games.browser.count.profiles",
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

/// One card of the Instance component grid. `fixed` marks values chosen when the
/// Instance was created, so the lock marker never depends on a rendered label.
struct ComponentCard {
    id: &'static str,
    icon: Icon,
    label: String,
    value: String,
    description: String,
    fixed: bool,
    action: Option<UiAction>,
}

impl Block {
    pub(super) fn initialize_browsers(&mut self, cx: &mut Context<'_, Self>) {
        let language = self.language();
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
            let label = tr!(language, kind.search_key());
            if state.query.is_none() {
                state.query = Some(input("", &label, cx));
            }
            // The saved language replaces the hint on the retained field, so the
            // query entity, its selection and its IME state stay untouched.
            if let Some(field) = &state.query {
                field.update(cx, |input, cx| {
                    input.set_search(true, cx);
                    input.set_placeholder(&label, cx);
                });
            }
        }
    }

    pub(super) fn inspected_instance(&self) -> Option<&Instance> {
        if (self.router.tab() == Tab::Profiles
            || self.snapshot.profile_mode == ProfileMode::Bundled)
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
        let inspected = if self.router.tab() == Tab::Profiles {
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
        if self.router.tab() == Tab::Profiles {
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
        match self.router.tab() {
            Tab::Accounts => &self.shell.accounts,
            Tab::Profiles => &self.shell.profiles,
            _ => &self.shell.games,
        }
    }
    pub(super) const fn collection_mut(&mut self) -> &mut CollectionState {
        match self.router.tab() {
            Tab::Accounts => &mut self.shell.accounts,
            Tab::Profiles => &mut self.shell.profiles,
            _ => &mut self.shell.games,
        }
    }

    fn browser_kind(&self) -> BrowserKind {
        if self.router.tab() == Tab::Accounts {
            BrowserKind::Accounts
        } else if self.snapshot.profile_mode == ProfileMode::Bundled {
            BrowserKind::Bundled
        } else if self.router.tab() == Tab::Profiles {
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
            Some(block_launcher::manifest::model::VersionType::Release) => BadgeVariant::Success,
            Some(block_launcher::manifest::model::VersionType::Snapshot) => BadgeVariant::Warning,
            _ => BadgeVariant::Secondary,
        };
        vec![
            (instance.minecraft_version.clone(), variant),
            (
                loader_value(instance, profile, self.language()),
                BadgeVariant::Secondary,
            ),
        ]
    }

    fn browser_records(&self, cx: &gpui::App) -> Vec<Record> {
        let language = self.language();
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
                        loader_value(instance, None, language)
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
                            Some(profile),
                            language
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
                            || {
                                vec![(
                                    tr!(language, "games.browser.instance_unavailable"),
                                    BadgeVariant::Destructive,
                                )]
                            },
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
        cx: &Context<'_, Self>,
    ) -> super::page_host::ManagementSlots {
        let kind = self.browser_kind();
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
        let heading = self.browser_heading(kind, count, cx);
        let rows = self.browser_rows(kind, count, &records, roving, cx);
        let rows = category_panel(kind, rows);
        let footer = self.browser_footer(kind, count, cx);
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
        super::page_host::ManagementSlots {
            sidebar: browser.into_any_element(),
            detail: self.browser_detail_content(cx),
        }
    }

    fn browser_heading(
        &self,
        kind: BrowserKind,
        count: usize,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let state = self.collection();
        let language = self.language();
        let mut heading = div().p_4().flex().flex_col().gap_4();
        if kind == BrowserKind::Accounts || self.snapshot.profile_mode == ProfileMode::Bundled {
            heading = heading.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(note(tr!(language, kind.title_key())))
                    .child(if kind == BrowserKind::Accounts {
                        note(count.to_string()).into_any_element()
                    } else {
                        self.browser_help(cx).into_any_element()
                    }),
            );
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
                                tr!(language, "games.browser.instance"),
                                cx,
                            ))
                            .child(self.browser_tab(
                                Tab::Profiles,
                                "games-tab-profiles",
                                tr!(language, "games.browser.profile"),
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
            tr!(language, "games.action.add_account")
        } else if kind == BrowserKind::Profiles {
            tr!(language, "games.action.create_profile")
        } else {
            tr!(language, "games.action.create_game_setup")
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
        let language = self.language();
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
            rows = rows.child(self.browser_status(
                tr!(language, "games.browser.loading_records"),
                Icon::Loading,
            ));
        } else if records.is_empty() && !matches!(state.load, CollectionLoad::Failed(_)) {
            let message = if count == 0 {
                tr!(language, kind.empty_key())
            } else {
                tr!(language, kind.no_match_key())
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
                    .child(note(error.text(language)).text_color(rgb(self.palette.destructive)))
                    .child(self.browser_control(
                        "reload-library",
                        tr!(language, "games.action.try_again"),
                        UiAction::ReloadLibrary,
                        ButtonVariant::Outline,
                        false,
                        cx,
                    )),
            );
        }
        rows.scroll_list_area(&state.scroll, "games.browser.records", self.palette)
    }

    fn browser_footer(&self, kind: BrowserKind, count: usize, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let mut footer = patterns::management_footer(self.palette).px_5();
        if kind == BrowserKind::Accounts {
            footer = footer.child(
                self.browser_control(
                    "authlib-providers",
                    tr!(language, "games.action.authlib_providers"),
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
                .child(note(tr!(language, kind.count_key(), count = count)))
                .child(
                    note(tr!(
                        language,
                        if self.snapshot.profile_mode == ProfileMode::Bundled {
                            "games.mode.bundled"
                        } else {
                            "games.mode.separated"
                        }
                    ))
                    .text_size(px(10.)),
                );
        }
        footer
    }

    fn browser_control(
        &self,
        id: impl Into<gpui::SharedString>,
        label: impl Into<gpui::SharedString>,
        action: UiAction,
        variant: ButtonVariant,
        disabled: bool,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        let id = id.into();
        let text = if matches!(id.as_ref(), "sign-in" | "create-instance") {
            gpui::SharedString::from("")
        } else {
            label.into()
        };
        components::button(
            id,
            text,
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
        label: impl Into<gpui::SharedString>,
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
        .track_focus(
            &self
                .browser_tab_focus(tab, cx)
                .tab_stop(self.router.tab() == tab),
        )
        .tab_index(if self.router.tab() == tab { 0 } else { -1 })
        .flex_1()
        .bg(rgb(if self.router.tab() == tab {
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
                    _ if view.router.tab() == Tab::Instances => Tab::Profiles,
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
        super::tooltip::trigger(control, tr!(self.language(), "games.browser.help"), cx)
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
        .child(self.record_avatar(
            record.icon,
            &record.title,
            self.account_avatars.get(&record.id),
            40.,
        ))
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

    fn record_avatar(
        &self,
        icon: RowIcon,
        name: &str,
        avatar: Option<&components::AccountAvatar>,
        size: f32,
    ) -> gpui::Div {
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
                components::account_avatar(name, avatar, self.palette, size, AvatarState::Fallback)
                    .rounded(px(if size > 40. { 16. } else { 8. }))
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

    pub(super) fn browser_detail_content(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        if self.shell.instance_editor.is_some() {
            return self.render_instance_editor(cx);
        }
        if matches!(self.form, Some(Form::Profile(_))) {
            return self.render_profile_editor(cx);
        }
        let Some((body, footer)) = self.detail_sections(cx) else {
            return self.empty_browser_detail(self.router.tab() == Tab::Accounts, cx);
        };
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

    /// Body and footer for the inspected record, or `None` when the collection
    /// holds no record to inspect.
    fn detail_sections(&self, cx: &Context<'_, Self>) -> Option<(DetailBody, gpui::Div)> {
        let body = self.detail_body(cx);
        if self.router.tab() == Tab::Accounts {
            return self.account_sections(body, cx);
        }
        self.instance_sections(body, cx)
    }

    /// The scrolling region that account and game details both fill with their
    /// own content.
    fn detail_body(&self, cx: &Context<'_, Self>) -> DetailBody {
        div()
            .id("main-content")
            .debug_selector(|| "management-detail".into())
            .flex_1()
            .min_h_0()
            .scroll_area(
                &self.collection().detail_scroll,
                "games.browser.details",
                self.palette,
                false,
            )
            .track_focus(&self.browser_detail_focus(cx))
            .flex()
            .flex_col()
            .gap_6()
            .p_8()
    }

    fn detail_footer(&self) -> gpui::Div {
        patterns::management_footer(self.palette).justify_between()
    }

    fn account_sections(
        &self,
        body: DetailBody,
        cx: &Context<'_, Self>,
    ) -> Option<(DetailBody, gpui::Div)> {
        let language = self.language();
        let account = self
            .accounts
            .iter()
            .find(|account| Some(account_id(account)) == self.inspected_record())?;
        let id = account_id(account);
        let current = self.selected_account.as_ref() == Some(&id);
        let body = body.child(self.account_details(account, current));
        let footer = self
            .detail_footer()
            .child(self.browser_control(
                format!("account-logout-{id}"),
                tr!(language, "games.action.remove_account"),
                UiAction::RequestSignOut(id.clone()),
                ButtonVariant::Outline,
                self.busy,
                cx,
            ))
            .child(self.browser_control(
                format!("account-select-{id}"),
                if current {
                    tr!(language, "games.action.current_account")
                } else {
                    tr!(language, "games.action.use_account")
                },
                UiAction::SelectAccount(id),
                ButtonVariant::Primary,
                current || self.busy,
                cx,
            ));
        Some((body, footer))
    }

    fn instance_sections(
        &self,
        body: DetailBody,
        cx: &Context<'_, Self>,
    ) -> Option<(DetailBody, gpui::Div)> {
        let instance = self.inspected_instance()?;
        if self.browser_kind() == BrowserKind::Instances {
            return Some(self.separated_instance_sections(body, instance, cx));
        }
        let profile = self.inspected_profile()?;
        Some(self.profile_sections(body, instance, profile, cx))
    }

    /// An Instance with the Profiles that run on it.
    fn separated_instance_sections(
        &self,
        body: DetailBody,
        instance: &Instance,
        cx: &Context<'_, Self>,
    ) -> (DetailBody, gpui::Div) {
        let language = self.language();
        let mut body = body
            .child(self.detail_identity(
                RowIcon::Instance,
                &instance.name,
                None,
                self.instance_badges(instance, None),
                false,
            ))
            .child(self.instance_components(instance, None, cx))
            .child(self.linked_profiles(instance, cx));
        if let Some(notice) = self.installation_notice(instance, None) {
            body = body.child(notice);
        }
        let footer = self
            .detail_footer()
            .child(note(tr!(language, "games.mode.separated_profile")))
            .child(
                self.browser_control(
                    "new-profile",
                    tr!(language, "games.action.new_profile"),
                    UiAction::NewProfile,
                    ButtonVariant::Primary,
                    self.busy,
                    cx,
                )
                .child(Icon::Plus.render(16., self.palette.on_primary)),
            );
        (body, footer)
    }

    /// A Profile with the components it launches through, or the Instance it
    /// links to when the game keeps its launch settings on the game itself.
    fn profile_sections(
        &self,
        body: DetailBody,
        instance: &Instance,
        profile: &Profile,
        cx: &Context<'_, Self>,
    ) -> (DetailBody, gpui::Div) {
        let language = self.language();
        let kind = self.browser_kind();
        let current = self.selected_profile.as_ref() == Some(&profile.id);
        let mut body = body.child(
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(
                    self.detail_identity(
                        RowIcon::Profile(profile.appearance),
                        &profile.name,
                        None,
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
        body = if kind == BrowserKind::Bundled {
            body.child(self.instance_components(instance, Some(profile), cx))
        } else {
            body.child(self.linked_instance(instance, cx))
        };
        body = body.child(self.launch_details(profile));
        if let Some(notice) = self.installation_notice(instance, Some(profile)) {
            body = body.child(notice);
        }
        let footer = self
            .detail_footer()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.browser_control(
                            "edit-profile",
                            if kind == BrowserKind::Bundled {
                                tr!(language, "games.action.edit_launch_settings")
                            } else {
                                tr!(language, "games.action.edit_profile")
                            },
                            UiAction::EditProfile,
                            ButtonVariant::Outline,
                            self.busy,
                            cx,
                        )
                        .track_focus(&self.profile_editor_trigger_focus(cx)),
                    )
                    .child(self.browser_control(
                        "repair-profile",
                        tr!(language, "games.action.repair"),
                        UiAction::RepairProfile(profile.id.clone()),
                        ButtonVariant::Outline,
                        self.busy,
                        cx,
                    )),
            )
            .child(self.browser_control(
                "use-profile",
                if current {
                    if kind == BrowserKind::Bundled {
                        tr!(language, "games.action.current_game")
                    } else {
                        tr!(language, "games.action.current_profile")
                    }
                } else if kind == BrowserKind::Bundled {
                    tr!(language, "games.action.use_game")
                } else {
                    tr!(language, "games.action.use_profile")
                },
                UiAction::SelectProfile(profile.id.clone()),
                ButtonVariant::Primary,
                current || self.busy,
                cx,
            ));
        (body, footer)
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
        let language = self.language();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .border_t_1()
                .border_color(rgb(self.palette.border))
                .pt_4()
                .child(note(tr!(language, "games.detail.installation")))
                .children(targets.into_iter().map(|target| {
                    let (message, error) = match &target.status {
                        block_launcher::catalog::InstallStatus::Pending => {
                            (tr!(language, "games.install.pending"), false)
                        }
                        block_launcher::catalog::InstallStatus::Installing => {
                            (tr!(language, "games.install.installing"), false)
                        }
                        block_launcher::catalog::InstallStatus::Failed { message } => (
                            if profile.is_some() {
                                tr!(language, "games.install.failed_repair", message = message)
                            } else {
                                tr!(
                                    language,
                                    "games.install.failed_open_profile",
                                    message = message
                                )
                            },
                            true,
                        ),
                        block_launcher::catalog::InstallStatus::Ready => (String::new(), false),
                    };
                    div()
                        .id(gpui::SharedString::from(format!(
                            "installation-status-{}",
                            target.id
                        )))
                        .child(note(message).text_color(rgb(if error {
                            self.palette.destructive
                        } else {
                            self.palette.muted
                        })))
                })),
        )
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

    fn record_badge(&self, label: String, variant: BadgeVariant, account: bool) -> gpui::Div {
        if account {
            components::badge(label, self.palette, BadgeVariant::Info)
        } else {
            components::badge(label, self.palette, variant)
        }
    }

    fn detail_identity(
        &self,
        icon: RowIcon,
        name: &str,
        avatar: Option<&components::AccountAvatar>,
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
                avatar,
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
                                components::badge(
                                    tr!(self.language(), "games.browser.in_use"),
                                    self.palette,
                                    BadgeVariant::Success,
                                )
                                .gap_1()
                                .child(Icon::Check.render(12., self.palette.success))
                            })),
                    ),
            )
    }

    fn detail_metadata(title: &str, values: &[(&str, String)]) -> gpui::Div {
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
                            .w(relative(0.5))
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

    fn instance_components(
        &self,
        instance: &Instance,
        profile: Option<&Profile>,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let language = self.language();
        let mut grid = div().flex().flex_wrap().gap_3();
        let width = 0.48;
        let components = [
            ComponentCard {
                id: "component-minecraft",
                icon: Icon::Blocks,
                // Minecraft and OptiFine are product names, not translated copy.
                label: "Minecraft".to_owned(),
                value: instance.minecraft_version.clone(),
                description: tr!(language, "games.components.fixed_at_creation"),
                fixed: true,
                action: None,
            },
            ComponentCard {
                id: "component-loader",
                icon: Icon::Layers,
                label: tr!(language, "games.components.mod_loader"),
                value: loader_value(instance, profile, language),
                description: tr!(language, "games.components.fixed_at_creation"),
                fixed: true,
                action: None,
            },
            ComponentCard {
                id: "component-optifine",
                icon: Icon::Experiment,
                label: "OptiFine".to_owned(),
                value: instance
                    .targets
                    .iter()
                    .find(|target| profile.is_some_and(|profile| profile.target_id == target.id))
                    .and_then(|target| target.optifine.as_ref())
                    .map_or_else(
                        || tr!(language, "games.components.not_installed"),
                        |selected| selected.version.clone(),
                    ),
                description: tr!(language, "games.components.editable_in_settings"),
                fixed: false,
                action: Some(UiAction::EditInstanceOptiFine),
            },
            ComponentCard {
                id: "component-instance-settings",
                icon: Icon::Settings,
                label: tr!(language, "games.components.instance_settings"),
                value: match instance
                    .settings
                    .as_ref()
                    .map(|settings| &settings.runtime)
                    .or_else(|| profile.map(|profile| &profile.runtime))
                {
                    Some(RuntimeSelection::Managed) | None => {
                        tr!(language, "games.java.automatic")
                    }
                    Some(RuntimeSelection::Local { .. }) => {
                        tr!(language, "games.java.custom")
                    }
                },
                description: instance.directory.display().to_string(),
                fixed: false,
                action: Some(UiAction::EditInstance),
            },
        ];
        for ComponentCard {
            id,
            icon,
            label,
            value,
            description,
            fixed,
            action,
        } in components
        {
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
                        .children(fixed.then(|| {
                            div()
                                .flex_shrink_0()
                                .debug_selector(move || format!("component-lock-{id}"))
                                .child(Icon::Lock.render(14., self.palette.muted))
                        })),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .min_w_0()
                        .max_w_full()
                        .truncate()
                        .child(value),
                )
                .child(note(description).min_w_0());
            if let Some(action) = action {
                let trigger = self.shell.surfaces.trigger_focus(id, cx);
                let restore_focus = trigger.clone();
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
                            restore_focus.focus(window, cx);
                            view.handle(action.clone(), window, cx);
                        }),
                    )
                    .track_focus(&trigger)
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
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(grid)
            .child(note(tr!(language, "games.components.fixed_note")))
    }

    fn linked_profiles(&self, instance: &Instance, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
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
                    .child(note(if profiles.len() == 1 {
                        tr!(language, "games.instance.used_by_one")
                    } else {
                        tr!(
                            language,
                            "games.instance.used_by_many",
                            count = profiles.len()
                        )
                    }))
                    .child(self.button(
                        "instance-new-profile",
                        tr!(language, "games.action.new_profile"),
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
        let language = self.language();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(note(tr!(language, "games.instance.game_instance")))
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
                .child(self.record_avatar(RowIcon::Instance, &instance.name, None, 40.))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(instance.name.clone())
                        .child(note(tr!(language, "games.instance.shared_files"))),
                )
                .child(Icon::ExternalLink.render(16., self.palette.muted)),
            )
    }

    fn launch_details(&self, profile: &Profile) -> gpui::Div {
        let language = self.language();
        let server = (!profile.server.address.is_empty()).then(|| profile.server.address.clone());
        let width = 0.48;
        let card = |icon: Icon, label: String, value: String, description: String| {
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
                        .child(note(label)),
                )
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(value))
                .child(note(description))
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr!(language, "games.launch.settings")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(card(
                        Icon::Server,
                        tr!(language, "games.launch.server"),
                        server.unwrap_or_else(|| tr!(language, "games.launch.choose_in_game")),
                        if profile.server.connect_on_launch {
                            tr!(language, "games.launch.connect_automatically")
                        } else {
                            tr!(language, "games.launch.no_automatic_connection")
                        },
                    ))
                    .child(card(
                        Icon::Cpu,
                        tr!(language, "games.launch.memory"),
                        components::memory_label(profile.memory_mib),
                        tr!(language, "games.launch.maximum_allocation"),
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::Terminal.render(14., self.palette.muted))
                    .child(note(tr!(language, "games.launch.jvm_arguments"))),
            )
            .child(
                div()
                    .rounded_lg()
                    .p_3()
                    .bg(rgb(self.palette.secondary))
                    .text_xs()
                    .child(if profile.jvm_args.is_empty() {
                        tr!(language, "games.launch.default_arguments")
                    } else {
                        profile.jvm_args.join(" ")
                    }),
            )
    }

    fn empty_browser_detail(&self, accounts: bool, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let language = self.language();
        let loading = matches!(self.collection().load, CollectionLoad::Loading);
        let failed = matches!(self.collection().load, CollectionLoad::Failed(_));
        let title = if loading {
            tr!(language, "games.empty_state.loading_title")
        } else if failed {
            tr!(language, "games.empty_state.failed_title")
        } else {
            tr!(language, "games.empty_state.title")
        };
        let description = if loading {
            tr!(language, "games.empty_state.loading_body")
        } else if failed {
            tr!(language, "games.empty_state.failed_body")
        } else if accounts {
            tr!(language, "games.empty_state.accounts_body")
        } else {
            tr!(language, "games.empty_state.games_body")
        };
        let mut content = patterns::empty_state(title, description, self.palette).child(
            if accounts { Icon::Account } else { Icon::Games }.render(32., self.palette.muted),
        );
        if !loading {
            content = content.child(self.browser_control(
                "welcome-create",
                if failed {
                    tr!(language, "games.action.try_again")
                } else if accounts {
                    tr!(language, "games.action.add_account")
                } else {
                    tr!(language, "games.action.create_game")
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
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(content)
            .child(patterns::management_footer(self.palette))
            .into_any_element()
    }

    pub(super) fn render_games_help(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let _ = cx;
        let language = self.language();
        let mode = if self.snapshot.profile_mode == ProfileMode::Bundled {
            "games.mode.bundled"
        } else {
            "games.mode.separated"
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(note(tr!(language, "games.help.intro")))
            .child(
                div()
                    .id("games-help-body")
                    .h(px(360.))
                    .min_h_0()
                    .w_full()
                    .child(
                        super::scroll_area::viewport(
                            div().id("games-help-scroll").flex().flex_col().gap_3(),
                            &self.shell.help_scroll,
                            "games.help",
                            self.palette,
                            false,
                        )
                        .child(
                            components::card(self.palette)
                                .p_3()
                                .child(tr!(language, "games.browser.instance"))
                                .child(note(tr!(language, "games.help.instance_body"))),
                        )
                        .child(
                            components::card(self.palette)
                                .p_3()
                                .child(tr!(language, "games.mode.bundled_profile"))
                                .child(note(tr!(language, "games.help.bundled_body"))),
                        )
                        .child(
                            components::card(self.palette)
                                .p_3()
                                .child(tr!(language, "games.mode.separated_profile"))
                                .child(note(tr!(language, "games.help.separated_body"))),
                        )
                        .child(note(tr!(
                            language,
                            "games.help.current_mode",
                            mode = tr!(language, mode)
                        ))),
                    ),
            )
            .into_any_element()
    }
}

fn loader_value(instance: &Instance, profile: Option<&Profile>, language: Language) -> String {
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
        || tr!(language, "games.instance.no_launch_target"),
        block_launcher::catalog::LaunchTarget::display_name,
    )
}
