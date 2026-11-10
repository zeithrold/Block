//! Typed destinations and the presentation identity shared by the page host.
use super::{
    Block, Form,
    authentication::AuthStep,
    game_wizard::{WizardPage, WizardStep},
    instance_editor::EditorPage,
    modpack_import::Phase as PackPhase,
    motion::MotionStyle,
    providers::ProviderKey,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Home,
    Onboarding,
    Instances,
    Accounts,
    Settings,
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    GameChoice,
    GameWizard,
    Authentication,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum RecordRef {
    Instance(String),
    Account(String),
}

impl RecordRef {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Instance(id) | Self::Account(id) => id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Screen {
    #[default]
    List,
    Record(RecordRef),
    InstanceEditor(String),
    Providers,
    Provider(uuid::Uuid),
    NewProvider,
    EditProvider(uuid::Uuid),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Destination {
    entry: u64,
    tab: Tab,
    task: Option<Task>,
    screen: Screen,
}

/// Navigation owns the destination and a task's return destination, not its draft.
#[derive(Clone, Debug)]
pub(super) struct Router {
    current: Destination,
    auth_return: Option<Destination>,
    parents: Vec<Destination>,
    next_entry: u64,
}

impl Default for Router {
    fn default() -> Self {
        Self::new(Tab::Home)
    }
}

impl Router {
    pub(super) fn new(tab: Tab) -> Self {
        let nested = !matches!(tab, Tab::Home | Tab::Onboarding);
        Self {
            current: Destination {
                entry: u64::from(nested),
                tab,
                task: None,
                screen: Screen::List,
            },
            auth_return: None,
            parents: if nested {
                vec![Destination::default()]
            } else {
                Vec::new()
            },
            next_entry: if nested { 2 } else { 1 },
        }
    }

    pub(super) const fn tab(&self) -> Tab {
        self.current.tab
    }

    pub(super) fn navigate(&mut self, tab: Tab) {
        if self.current.tab == tab
            && self.current.task.is_none()
            && self.current.screen == Screen::List
        {
            return;
        }
        self.parents.clear();
        if !matches!(tab, Tab::Home | Tab::Onboarding) {
            self.parents.push(Destination::default());
        }
        self.current = Destination {
            entry: self.allocate_entry(),
            tab,
            ..Destination::default()
        };
    }

    pub(super) fn screen(&self) -> &Screen {
        &self.current.screen
    }

    pub(super) const fn entry_id(&self) -> u64 {
        self.current.entry
    }

    pub(super) fn contains_entry(&self, entry: u64) -> bool {
        self.current.entry == entry
            || self.parents.iter().any(|parent| parent.entry == entry)
            || self
                .auth_return
                .as_ref()
                .is_some_and(|parent| parent.entry == entry)
    }

    fn allocate_entry(&mut self) -> u64 {
        let entry = self.next_entry;
        self.next_entry = self.next_entry.wrapping_add(1);
        entry
    }

    pub(super) fn open_screen(&mut self, tab: Tab, screen: Screen) {
        if self.current.tab == tab && self.current.task.is_none() && self.current.screen == screen {
            return;
        }
        let destination = Destination {
            entry: self.allocate_entry(),
            tab,
            task: None,
            screen,
        };
        self.parents.push(self.current.clone());
        self.current = destination;
    }

    pub(super) fn open_record(&mut self, tab: Tab, record: RecordRef) {
        self.open_screen(tab, Screen::Record(record));
    }

    pub(super) fn back(&mut self) -> bool {
        if let Some(parent) = self.parents.pop() {
            self.current = parent;
            true
        } else {
            false
        }
    }

    pub(super) fn enter_game_choice(&mut self) {
        if self.current.task.is_none() {
            self.parents.push(self.current.clone());
            self.current.entry = self.allocate_entry();
        }
        self.current.task = Some(Task::GameChoice);
    }

    pub(super) fn enter_game_wizard(&mut self) {
        if self.current.task.is_none() {
            self.parents.push(self.current.clone());
            self.current.entry = self.allocate_entry();
        }
        self.current.task = Some(Task::GameWizard);
    }

    pub(super) fn game_wizard(&self) -> bool {
        self.current.task == Some(Task::GameWizard)
    }

    pub(super) fn leave_task(&mut self) {
        if self.current.task.is_some() && !self.back() {
            self.current.task = None;
        }
    }

    pub(super) const fn allows(&self, tab: Tab, setup_incomplete: bool) -> bool {
        matches!((setup_incomplete, tab), (true, Tab::Onboarding))
            || !setup_incomplete && !matches!(tab, Tab::Onboarding)
    }

    pub(super) fn begin_auth(&mut self) {
        self.auth_return = Some(self.current.clone());
        self.current = Destination {
            entry: self.allocate_entry(),
            tab: Tab::Accounts,
            task: Some(Task::Authentication),
            screen: Screen::List,
        };
    }

    pub(super) const fn auth_return(&self) -> Option<Tab> {
        match &self.auth_return {
            Some(destination) => Some(destination.tab),
            None => None,
        }
    }

    /// The account page is a task over the destination that opened it, so
    /// leaving it — saved, cancelled or failed alike — returns to that
    /// destination and its own task.
    pub(super) fn end_auth(&mut self) {
        if let Some(destination) = self.auth_return.take() {
            self.current = destination;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Page {
    Home,
    FirstRun,
    Browser,
    Providers,
    Settings,
    About,
    Authentication,
    GameWizard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LayoutKind {
    Plain,
    SinglePane,
    Management,
    Wizard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PageKey {
    tab: Tab,
    auth: bool,
    task: bool,
    providers: bool,
    first_run: Option<super::first_run::Stage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DetailKey {
    Settings(Option<u8>),
    Provider(ProviderKey),
    Browser {
        tab: Tab,
        screen: Screen,
        inspected: Option<String>,
        editor_page: Option<EditorPage>,
        instance_form: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardKey {
    Authentication(AuthStep),
    Game(WizardStep, Option<WizardPage>),
    Pack(PackPhase),
}

/// Content selection and motion use the same page and region identity.
#[derive(Clone, Debug)]
pub(super) struct PageSpec {
    pub page: Page,
    pub key: PageKey,
    pub detail: DetailKey,
    pub wizard: Option<WizardKey>,
    pub layout: LayoutKind,
    pub preset: MotionStyle,
    pub task: bool,
}

impl PageSpec {
    pub(super) const fn page_label(&self) -> &'static str {
        match self.page {
            Page::Home => "home",
            Page::FirstRun => "first_run",
            Page::Browser => "browser",
            Page::Providers => "providers",
            Page::Settings => "settings",
            Page::About => "about",
            Page::Authentication => "authentication",
            Page::GameWizard => "game_wizard",
        }
    }

    pub(super) const fn tab_label(&self) -> &'static str {
        match self.key.tab {
            Tab::Home => "home",
            Tab::Onboarding => "onboarding",
            Tab::Instances => "instances",
            Tab::Accounts => "accounts",
            Tab::Settings => "settings",
            Tab::About => "about",
        }
    }

    pub(super) const fn frame_label(&self) -> &'static str {
        "fixed"
    }

    pub(super) const fn detail_label(&self) -> &'static str {
        match &self.detail {
            DetailKey::Settings(_) => "settings",
            DetailKey::Provider(ProviderKey::List) => "provider_list",
            DetailKey::Provider(ProviderKey::Detail(_)) => "provider_detail",
            DetailKey::Provider(ProviderKey::New) => "provider_new",
            DetailKey::Provider(ProviderKey::Existing(_)) => "provider_existing",
            DetailKey::Browser {
                instance_form: true,
                ..
            } => "instance_editor",
            DetailKey::Browser { .. } => "browser",
        }
    }

    pub(super) const fn wizard_label(&self) -> &'static str {
        match self.wizard {
            None => "none",
            Some(WizardKey::Authentication(AuthStep::Choose)) => "auth_choose",
            Some(WizardKey::Authentication(AuthStep::SignIn)) => "auth_sign_in",
            Some(WizardKey::Authentication(AuthStep::Verification)) => "auth_verification",
            Some(WizardKey::Authentication(AuthStep::Complete)) => "auth_complete",
            Some(WizardKey::Game(WizardStep::GetStarted, _)) => "game_get_started",
            Some(WizardKey::Game(WizardStep::Setup, _)) => "game_setup",
            Some(WizardKey::Pack(PackPhase::Choose)) => "pack_choose",
            Some(WizardKey::Pack(PackPhase::Inspecting)) => "pack_inspecting",
            Some(WizardKey::Pack(PackPhase::Review)) => "pack_review",
            Some(WizardKey::Pack(PackPhase::Installing)) => "pack_installing",
            Some(WizardKey::Pack(PackPhase::Blocked)) => "pack_blocked",
            Some(WizardKey::Pack(PackPhase::Stopped)) => "pack_stopped",
            Some(WizardKey::Pack(PackPhase::Complete)) => "pack_complete",
        }
    }
}

impl Block {
    pub(super) fn page_spec(&self) -> PageSpec {
        let tab = self.router.tab();
        let current = self.router.current.task;
        let auth = current == Some(Task::Authentication);
        let task = current.is_some();
        let page = match current {
            Some(Task::Authentication) => Page::Authentication,
            Some(Task::GameWizard) => Page::GameWizard,
            Some(Task::GameChoice) => Page::FirstRun,
            None => match tab {
                Tab::Home => Page::Home,
                Tab::Onboarding => Page::FirstRun,
                Tab::Accounts
                    if matches!(
                        self.router.screen(),
                        Screen::Providers
                            | Screen::Provider(_)
                            | Screen::NewProvider
                            | Screen::EditProvider(_)
                    ) =>
                {
                    Page::Providers
                }
                Tab::Instances | Tab::Accounts => Page::Browser,
                Tab::Settings => Page::Settings,
                Tab::About => Page::About,
            },
        };
        let layout = match page {
            Page::Browser | Page::Providers | Page::About => LayoutKind::Management,
            Page::Settings => LayoutKind::SinglePane,
            Page::Authentication | Page::GameWizard => LayoutKind::Wizard,
            Page::Home | Page::FirstRun => LayoutKind::Plain,
        };
        let detail = match page {
            Page::Settings => DetailKey::Settings(self.shell.settings_page),
            Page::Providers => DetailKey::Provider(match self.router.screen() {
                Screen::Provider(id) => ProviderKey::Detail(*id),
                Screen::NewProvider => ProviderKey::New,
                Screen::EditProvider(id) => ProviderKey::Existing(*id),
                _ => ProviderKey::List,
            }),
            _ => DetailKey::Browser {
                tab,
                screen: self.router.screen().clone(),
                inspected: match self.router.screen() {
                    Screen::Record(record) => Some(record.id().to_owned()),
                    _ => None,
                },
                editor_page: matches!(self.router.screen(), Screen::InstanceEditor(_))
                    .then_some(self.shell.editor.page)
                    .flatten(),
                instance_form: matches!(self.router.screen(), Screen::InstanceEditor(_))
                    && matches!(self.form, Some(Form::EditInstance(_))),
            },
        };
        let wizard = if page == Page::Authentication
            && let Some(auth) = &self.shell.auth
        {
            Some(WizardKey::Authentication(auth.motion_key()))
        } else if page == Page::GameWizard && self.pack_wizard_active() {
            Some(WizardKey::Pack(self.shell.pack_import.phase))
        } else if page == Page::GameWizard && matches!(self.form, Some(Form::Instance(_))) {
            Some(WizardKey::Game(
                self.shell.wizard.step,
                self.wizard_outer_page(),
            ))
        } else {
            None
        };
        PageSpec {
            page,
            key: PageKey {
                tab,
                auth,
                task,
                providers: page == Page::Providers,
                first_run: (page == Page::FirstRun).then_some(self.shell.first_run.stage),
            },
            detail,
            wizard,
            layout,
            preset: if task {
                MotionStyle::Wizard
            } else {
                MotionStyle::Navigation
            },
            task,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DetailKey, LayoutKind, Page, PageKey, PageSpec, Router, Tab, Task};

    #[test]
    fn linked_details_and_editors_return_to_the_exact_parent_entry() {
        let mut router = Router::new(Tab::Instances);
        let list = router.entry_id();
        router.open_record(
            Tab::Instances,
            super::RecordRef::Instance("instance-a".into()),
        );
        let instance = router.entry_id();
        router.open_record(
            Tab::Instances,
            super::RecordRef::Instance("profile-b".into()),
        );
        let profile = router.entry_id();
        router.open_screen(
            Tab::Instances,
            super::Screen::InstanceEditor("profile-b".into()),
        );
        assert!(router.back());
        assert_eq!(router.entry_id(), profile);
        assert!(
            matches!(router.screen(), super::Screen::Record(super::RecordRef::Instance(id)) if id == "profile-b")
        );
        assert!(router.back());
        assert_eq!(router.entry_id(), instance);
        assert!(
            matches!(router.screen(), super::Screen::Record(super::RecordRef::Instance(id)) if id == "instance-a")
        );
        assert!(router.back());
        assert_eq!(router.entry_id(), list);
        assert_eq!(router.screen(), &super::Screen::List);
        assert!(router.back());
        assert_eq!(router.tab(), Tab::Home);
        assert!(!router.back());
    }

    #[test]
    fn root_navigation_is_idempotent_and_has_home_as_parent() {
        let mut router = Router::new(Tab::Home);
        router.navigate(Tab::Accounts);
        let accounts = router.entry_id();
        router.navigate(Tab::Accounts);
        assert_eq!(router.entry_id(), accounts);
        assert!(router.back());
        assert_eq!(router.tab(), Tab::Home);
        assert!(!router.contains_entry(accounts));
    }

    #[test]
    fn authentication_restores_the_same_detail_and_its_parent_stack() {
        let mut router = Router::new(Tab::Instances);
        router.open_record(
            Tab::Instances,
            super::RecordRef::Instance("profile-a".into()),
        );
        let detail = router.entry_id();
        router.begin_auth();
        router.end_auth();
        assert_eq!(router.entry_id(), detail);
        assert!(router.back());
        assert_eq!(router.screen(), &super::Screen::List);
    }

    #[test]
    fn game_choice_replacement_keeps_the_invoking_destination() {
        let mut router = Router::new(Tab::Onboarding);
        let overview = router.entry_id();
        router.enter_game_choice();
        router.enter_game_wizard();
        router.leave_task();
        assert_eq!(router.entry_id(), overview);
        assert_eq!(router.current.task, None);
        assert!(!router.back());
    }

    #[test]
    fn setup_guards_regular_destinations_and_cannot_be_replayed_after_completion() {
        let router = Router::new(Tab::Onboarding);
        assert!(router.allows(Tab::Onboarding, true));
        for tab in [
            Tab::Home,
            Tab::Instances,
            Tab::Accounts,
            Tab::Settings,
            Tab::About,
        ] {
            assert!(!router.allows(tab, true));
        }
        assert!(!router.allows(Tab::Onboarding, false));
        assert!(router.allows(Tab::Settings, false));
    }

    #[test]
    fn leaving_auth_restores_the_task_destination_not_just_its_tab() {
        let mut router = Router::new(Tab::Instances);
        router.enter_game_wizard();
        router.begin_auth();
        assert_eq!(router.tab(), Tab::Accounts);
        assert_eq!(router.auth_return(), Some(Tab::Instances));
        router.end_auth();
        assert_eq!(router.tab(), Tab::Instances);
        assert_eq!(router.current.task, Some(Task::GameWizard));
        assert_eq!(router.auth_return(), None);
    }

    #[test]
    fn leaving_auth_returns_to_whatever_destination_opened_it() {
        for tab in [Tab::Home, Tab::Onboarding, Tab::Accounts, Tab::Instances] {
            let mut router = Router::new(tab);
            router.begin_auth();
            assert_eq!(router.tab(), Tab::Accounts);
            router.end_auth();
            assert_eq!(router.tab(), tab);
            assert_eq!(router.current.task, None);
        }
    }

    #[test]
    fn page_labels_never_include_detail_identity() {
        let mut spec = PageSpec {
            page: Page::Browser,
            key: PageKey {
                tab: Tab::Instances,
                auth: false,
                task: false,
                providers: false,
                first_run: None,
            },
            detail: DetailKey::Browser {
                tab: Tab::Instances,
                screen: super::Screen::InstanceEditor("private-instance".into()),
                inspected: Some("private-profile".into()),
                editor_page: None,
                instance_form: true,
            },
            wizard: None,
            layout: LayoutKind::Management,
            preset: super::MotionStyle::Navigation,
            task: false,
        };
        assert_eq!(spec.page_label(), "browser");
        assert_eq!(spec.tab_label(), "instances");
        assert_eq!(spec.detail_label(), "instance_editor");
        assert_eq!(spec.wizard_label(), "none");
        spec.detail = DetailKey::Provider(super::ProviderKey::Existing(uuid::Uuid::from_u128(42)));
        assert_eq!(spec.detail_label(), "provider_existing");
    }
}
