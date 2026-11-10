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
    catalog::{Instance, InstanceAppearance},
    recipe::RuntimeSelection,
};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    Window, accesskit::Role, div, px, rgb,
};

use super::{
    Block, Language, Tab, TextInput, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState},
    design::Icon,
    input,
    navigation::{RecordRef, Screen},
    note, patterns,
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
    /// List highlight only. The Router owns the record shown on a detail page.
    pub inspected: Option<String>,
    pub scroll: ScrollHandle,
    pub detail_scroll: ScrollHandle,
    detail_entries: HashMap<u64, (RecordRef, ScrollHandle)>,
    detail_offsets: HashMap<RecordRef, gpui::Point<gpui::Pixels>>,
    active_detail: Option<(u64, RecordRef)>,
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
            detail_entries: HashMap::new(),
            detail_offsets: HashMap::new(),
            active_detail: None,
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
    Instances,
}

impl BrowserKind {
    /// Semantic keys, so every locale keeps its own wording for a kind.
    const fn title_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.title.accounts",
            Self::Instances => "games.browser.title.instances",
        }
    }
    const fn search_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.search.accounts",
            Self::Instances => "games.browser.search.instances",
        }
    }
    const fn empty_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.empty.accounts",
            Self::Instances => "games.browser.empty.instances",
        }
    }
    const fn no_match_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.no_match.accounts",
            Self::Instances => "games.browser.no_match.instances",
        }
    }
    const fn count_key(self) -> &'static str {
        match self {
            Self::Accounts => "games.browser.count.accounts",
            Self::Instances => "games.browser.count.instances",
        }
    }
}

fn category_panel(kind: BrowserKind, rows: impl IntoElement) -> gpui::AnyElement {
    if !matches!(kind, BrowserKind::Instances) {
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
    Instance(InstanceAppearance),
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
    pub(super) fn reconcile_browser_scroll(&mut self) {
        let exists = |record: &RecordRef| match record {
            RecordRef::Instance(id) => self
                .snapshot
                .instances
                .iter()
                .any(|record| record.id == *id),
            RecordRef::Account(id) => self.accounts.iter().any(|record| account_id(record) == *id),
        };
        for (tab, state) in [
            (Tab::Instances, &mut self.shell.games),
            (Tab::Accounts, &mut self.shell.accounts),
        ] {
            if let Some((_, record)) = &state.active_detail
                && exists(record)
            {
                state
                    .detail_offsets
                    .insert(record.clone(), state.detail_scroll.offset());
            }
            state.detail_offsets.retain(|record, _| exists(record));
            state
                .detail_entries
                .retain(|entry, (record, _)| self.router.contains_entry(*entry) && exists(record));
            let active = match self.router.screen() {
                Screen::Record(record) if self.router.tab() == tab && exists(record) => {
                    Some((self.router.entry_id(), record.clone()))
                }
                _ => None,
            };
            if active != state.active_detail
                && let Some((entry, record)) = &active
            {
                state.detail_scroll = state
                    .detail_entries
                    .entry(*entry)
                    .or_insert_with(|| {
                        let scroll = ScrollHandle::new();
                        if let Some(offset) = state.detail_offsets.get(record) {
                            scroll.set_offset(*offset);
                        }
                        (record.clone(), scroll)
                    })
                    .1
                    .clone();
            }
            state.active_detail = active;
        }
    }

    pub(super) fn initialize_browsers(&mut self, cx: &mut Context<'_, Self>) {
        let language = self.language();
        let games = BrowserKind::Instances;
        for (state, kind) in [
            (&mut self.shell.games, games),
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
        let id = match self.router.screen() {
            Screen::Record(RecordRef::Instance(id)) | Screen::InstanceEditor(id) => id.as_str(),
            Screen::List
            | Screen::Record(RecordRef::Account(_))
            | Screen::Providers
            | Screen::Provider(_)
            | Screen::NewProvider
            | Screen::EditProvider(_) => return None,
        };
        self.snapshot
            .instances
            .iter()
            .find(|instance| instance.id == id)
    }

    pub(super) const fn collection(&self) -> &CollectionState {
        match self.router.tab() {
            Tab::Accounts => &self.shell.accounts,
            _ => &self.shell.games,
        }
    }
    pub(super) const fn collection_mut(&mut self) -> &mut CollectionState {
        match self.router.tab() {
            Tab::Accounts => &mut self.shell.accounts,
            _ => &mut self.shell.games,
        }
    }

    fn browser_kind(&self) -> BrowserKind {
        if self.router.tab() == Tab::Accounts {
            BrowserKind::Accounts
        } else {
            BrowserKind::Instances
        }
    }

    fn instance_badges(&self, instance: &Instance) -> Vec<(String, BadgeVariant)> {
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
                loader_value(instance, self.language()),
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
                        loader_value(instance, language)
                    )
                    .to_lowercase()
                    .contains(&query)
                })
                .map(|instance| Record {
                    id: instance.id.clone(),
                    selector: format!("instance-{}", instance.id),
                    title: instance.name.clone(),
                    icon: RowIcon::Instance(instance.appearance),
                    badges: self.instance_badges(instance),
                    current: self.selected_instance.as_ref() == Some(&instance.id),
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
        let current = self.collection().inspected.clone();
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
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn render_browser_detail(
        &self,
        cx: &Context<'_, Self>,
    ) -> super::page_host::ContentSlots {
        match self.router.screen() {
            Screen::List => {}
            Screen::InstanceEditor(id) if matches!(&self.form, Some(super::Form::EditInstance(form)) if &form.original.id==id) =>
            {
                return self.render_instance_editor(cx);
            }
            Screen::Record(_) => {
                return match self.detail_sections(cx) {
                    Some((body, footer)) => super::page_host::ContentSlots {
                        body: body.into_any_element(),
                        footer: footer
                            .id("detail-footer")
                            .debug_selector(|| "detail-footer".into())
                            .into_any_element(),
                    },
                    None => self.unavailable_browser_detail(cx),
                };
            }
            Screen::InstanceEditor(_)
            | Screen::Providers
            | Screen::Provider(_)
            | Screen::NewProvider
            | Screen::EditProvider(_) => return self.unavailable_browser_detail(cx),
        }
        let kind = self.browser_kind();
        let records = self.browser_records(cx);
        let inspected = &self.collection().inspected;
        let roving = records
            .iter()
            .find(|record| Some(&record.id) == inspected.as_ref())
            .or_else(|| records.first())
            .map(|record| record.id.as_str());
        let count = match kind {
            BrowserKind::Accounts => self.accounts.len(),
            BrowserKind::Instances => self.snapshot.instances.len(),
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
            .child(rows);
        super::page_host::ContentSlots {
            body: browser.into_any_element(),
            footer: footer
                .id("browser-footer")
                .debug_selector(|| "browser-footer".into())
                .into_any_element(),
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
        let create = if kind == BrowserKind::Accounts {
            UiAction::SignIn
        } else {
            UiAction::CreateInstance
        };
        let create_label = tr!(
            language,
            if kind == BrowserKind::Accounts {
                "games.action.add_account"
            } else {
                "games.action.create_game_setup"
            }
        );
        heading = heading.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
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
                    .w_full()
                    .child(Icon::Plus.render(16., self.palette.foreground)),
                )
                .children(
                    state
                        .query
                        .as_ref()
                        .map(|field| div().w_full().min_w_0().child(field.clone())),
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
        let mut footer = patterns::management_footer(self.palette).px_4();
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
                .w_full()
                .child(Icon::Server.render(14., self.palette.muted)),
            );
        } else {
            footer = footer.child(note(tr!(language, kind.count_key(), count = count)));
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
        components::button(
            id,
            label,
            self.palette,
            variant,
            ControlState {
                disabled,
                ..Default::default()
            },
            cx.listener(move |view, (): &(), window, cx| view.handle(action.clone(), window, cx)),
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
        let inspected = self.collection().inspected.as_deref() == Some(record.id.as_str());
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
        .aria_label(record.title.clone())
        .track_focus(&focus)
        .w_full()
        .h_auto()
        .min_h(px(72.))
        .rounded_none()
        .border_0()
        .border_l_2()
        .border_color(rgb(if inspected {
            self.palette.foreground
        } else {
            self.palette.sidebar
        }))
        .px_4()
        .py_3()
        .flex()
        .items_center()
        .gap_3()
        .bg(rgb(if inspected {
            self.palette.inspected
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
        row.child(Icon::ChevronRight.render(16., self.palette.muted))
    }

    fn record_avatar(
        &self,
        icon: RowIcon,
        name: &str,
        avatar: Option<&components::AccountAvatar>,
        size: f32,
    ) -> gpui::Div {
        match icon {
            RowIcon::Instance(appearance) => {
                components::instance_avatar(appearance, self.palette, size)
            }
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
            .gap_5()
            .p_4()
    }

    fn detail_footer(&self) -> gpui::Div {
        patterns::management_footer(self.palette)
            .flex_col()
            .items_stretch()
            .gap_2()
    }

    fn account_sections(
        &self,
        body: DetailBody,
        cx: &Context<'_, Self>,
    ) -> Option<(DetailBody, gpui::Div)> {
        let language = self.language();
        let Screen::Record(RecordRef::Account(record)) = self.router.screen() else {
            return None;
        };
        let account = self
            .accounts
            .iter()
            .find(|account| account_id(account) == *record)?;
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
        let language = self.language();
        let current = self.selected_instance.as_ref() == Some(&instance.id);
        let mut body = body
            .child(self.detail_identity(
                RowIcon::Instance(instance.appearance),
                &instance.name,
                None,
                self.instance_badges(instance),
                current,
            ))
            .child(
                self.browser_control(
                    "edit-instance",
                    tr!(language, "games.action.edit_launch_settings"),
                    UiAction::EditInstance,
                    ButtonVariant::Outline,
                    self.busy,
                    cx,
                )
                .track_focus(&self.instance_editor_trigger_focus(cx)),
            )
            .child(self.browser_control(
                "repair-instance",
                tr!(language, "games.action.repair"),
                UiAction::RepairInstance(instance.id.clone()),
                ButtonVariant::Outline,
                self.busy,
                cx,
            ))
            .child(self.instance_components(instance, cx))
            .child(self.launch_details(instance));
        if let Some(notice) = self.installation_notice(instance) {
            body = body.child(notice);
        }
        let footer = self
            .detail_footer()
            .child(self.browser_control(
                "delete-instance",
                tr!(language, "action.delete_instance"),
                UiAction::DeleteInstanceId(instance.id.clone()),
                ButtonVariant::Outline,
                self.busy,
                cx,
            ))
            .child(self.browser_control(
                "use-instance",
                tr!(
                    language,
                    if current {
                        "games.action.current_game"
                    } else {
                        "games.action.use_game"
                    }
                ),
                UiAction::SelectInstance(instance.id.clone()),
                ButtonVariant::Primary,
                current || self.busy,
                cx,
            ));
        Some((body, footer))
    }

    /// Keep account inspection on a surviving row after an account refresh.
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

    fn installation_notice(&self, instance: &Instance) -> Option<gpui::Div> {
        let targets: Vec<_> = instance
            .targets
            .iter()
            .filter(|target| instance.target_id == target.id)
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
                            tr!(language, "games.install.failed_repair", message = message),
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
    pub(super) fn instance_editor_trigger_focus(&self, cx: &Context<'_, Self>) -> FocusHandle {
        self.collection()
            .focus("instance-editor-trigger", cx)
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
                    56.
                } else {
                    40.
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
                            .text_xl()
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
                    .flex_col()
                    .gap_y_6()
                    .children(values.iter().map(|(label, value)| {
                        div()
                            .w_full()
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

    fn instance_components(&self, instance: &Instance, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let mut grid = div().flex().flex_col().gap_3();
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
                value: loader_value(instance, language),
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
                    .find(|target| instance.target_id == target.id)
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
                value: match &instance.runtime {
                    RuntimeSelection::Managed => tr!(language, "games.java.automatic"),
                    RuntimeSelection::Local { .. } => tr!(language, "games.java.custom"),
                },
                description: instance
                    .game_directory
                    .display()
                    .to_string()
                    .chars()
                    .flat_map(|character| [character, '\u{200b}'])
                    .collect(),
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
                .items_center()
                .gap_3()
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .flex_shrink_0()
                        .child(icon.render(18., self.palette.muted)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(note(label.clone()))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .min_w_0()
                                .max_w_full()
                                .whitespace_normal()
                                .child(value),
                        )
                        .child(note(description).min_w_0().w_full().whitespace_normal()),
                )
                .children(action.as_ref().map(|_| {
                    div()
                        .flex_shrink_0()
                        .child(Icon::ChevronRight.render(16., self.palette.muted))
                }))
                .children(fixed.then(|| {
                    div()
                        .flex_shrink_0()
                        .debug_selector(move || format!("component-lock-{id}"))
                        .child(Icon::Lock.render(14., self.palette.muted))
                }));
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
                    .aria_label(label)
                    .track_focus(&trigger)
                    .w_full()
                    .h_auto()
                    .min_h(px(80.))
                    .p_3()
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .rounded(px(super::design::CARD_RADIUS))
                    .child(contents),
                );
            } else {
                grid = grid.child(
                    components::card(self.palette)
                        .id(id)
                        .w_full()
                        .min_h(px(80.))
                        .p_3()
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

    fn launch_details(&self, profile: &Instance) -> gpui::Div {
        let language = self.language();
        let server = (!profile.server.address.is_empty()).then(|| profile.server.address.clone());
        let card = |icon: Icon, label: String, value: String, description: String| {
            div()
                .w_full()
                .min_w_0()
                .rounded(px(super::design::CARD_RADIUS))
                .p_5()
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
                    .flex_col()
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

    fn unavailable_browser_detail(&self, cx: &Context<'_, Self>) -> super::page_host::ContentSlots {
        let language = self.language();
        super::page_host::ContentSlots {
            body: self
                .detail_body(cx)
                .child(patterns::empty_state(
                    tr!(language, "games.browser.unavailable_title"),
                    tr!(language, "games.browser.unavailable_body"),
                    self.palette,
                ))
                .into_any_element(),
            footer: self
                .detail_footer()
                .child(self.browser_control(
                    "record-back",
                    tr!(language, "common.back"),
                    UiAction::NavigationBack,
                    ButtonVariant::Outline,
                    false,
                    cx,
                ))
                .into_any_element(),
        }
    }

    pub(super) fn render_games_help(&self, _cx: &Context<'_, Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(note(tr!(self.language(), "games.help.instance_body")))
            .child(note(tr!(self.language(), "import.policy.files_remain")))
            .into_any_element()
    }
}

fn loader_value(instance: &Instance, language: Language) -> String {
    instance
        .targets
        .iter()
        .find(|target| target.id == instance.target_id)
        .map_or_else(
            || tr!(language, "games.instance.no_launch_target"),
            block_launcher::catalog::LaunchTarget::display_name,
        )
}
