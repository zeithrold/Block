//! Typed destinations and the presentation identity shared by the page host.
use super::{
    Block, Form,
    authentication::AuthStep,
    first_run::Stage,
    game_wizard::{WizardPage, WizardStep},
    motion::MotionStyle,
    profile_editor::EditorPage,
    providers::ProviderKey,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Home,
    Onboarding,
    Instances,
    Profiles,
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

#[derive(Clone, Copy, Debug, Default)]
struct Destination {
    tab: Tab,
    task: Option<Task>,
}

/// Navigation owns the destination and a task's return destination, not its draft.
#[derive(Clone, Debug, Default)]
pub(super) struct Router {
    current: Destination,
    auth_return: Option<Destination>,
}

impl Router {
    pub(super) const fn new(tab: Tab) -> Self {
        Self {
            current: Destination { tab, task: None },
            auth_return: None,
        }
    }

    pub(super) const fn tab(&self) -> Tab {
        self.current.tab
    }

    pub(super) fn navigate(&mut self, tab: Tab) {
        self.current = Destination { tab, task: None };
    }

    pub(super) fn enter_game_choice(&mut self) {
        self.current.task = Some(Task::GameChoice);
    }

    pub(super) fn enter_game_wizard(&mut self) {
        self.current.task = Some(Task::GameWizard);
    }

    pub(super) fn leave_task(&mut self) {
        self.current.task = None;
    }

    pub(super) const fn allows(&self, tab: Tab, setup_incomplete: bool) -> bool {
        matches!((setup_incomplete, tab), (true, Tab::Onboarding))
            || !setup_incomplete && !matches!(tab, Tab::Onboarding)
    }

    pub(super) fn begin_auth(&mut self) {
        self.auth_return = Some(self.current);
        self.current = Destination {
            tab: Tab::Accounts,
            task: Some(Task::Authentication),
        };
    }

    pub(super) const fn auth_return(&self) -> Option<Tab> {
        match self.auth_return {
            Some(destination) => Some(destination.tab),
            None => None,
        }
    }

    pub(super) fn end_auth(&mut self, restore: bool) -> Option<Tab> {
        let destination = self.auth_return.take()?;
        if restore || destination.tab == Tab::Onboarding {
            self.current = destination;
        } else {
            self.current.task = None;
        }
        Some(destination.tab)
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
pub(super) enum FrameClass {
    Home,
    Management,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LayoutKind {
    Plain,
    Management,
    Wizard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PageKey {
    tab: Tab,
    auth: bool,
    task: bool,
    providers: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DetailKey {
    Settings(u8),
    Provider(ProviderKey),
    Browser {
        tab: Tab,
        inspected: Option<String>,
        editor_page: Option<EditorPage>,
        profile_form: bool,
        instance_editor: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardKey {
    Authentication(AuthStep),
    Game(WizardStep, Option<WizardPage>),
}

/// Content selection and motion use the same page and region identity.
#[derive(Clone, Debug)]
pub(super) struct PageSpec {
    pub page: Page,
    pub key: PageKey,
    pub detail: DetailKey,
    pub wizard: Option<WizardKey>,
    pub frame: FrameClass,
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
            Tab::Profiles => "profiles",
            Tab::Accounts => "accounts",
            Tab::Settings => "settings",
            Tab::About => "about",
        }
    }

    pub(super) const fn frame_label(&self) -> &'static str {
        match self.frame {
            FrameClass::Home => "home",
            FrameClass::Management => "management",
        }
    }

    pub(super) const fn detail_label(&self) -> &'static str {
        match &self.detail {
            DetailKey::Settings(_) => "settings",
            DetailKey::Provider(ProviderKey::List) => "provider_list",
            DetailKey::Provider(ProviderKey::New) => "provider_new",
            DetailKey::Provider(ProviderKey::Existing(_)) => "provider_existing",
            DetailKey::Browser {
                instance_editor: Some(_),
                ..
            } => "instance_editor",
            DetailKey::Browser {
                profile_form: true, ..
            } => "profile_editor",
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
            Some(WizardKey::Game(WizardStep::Profile, _)) => "game_profile",
            Some(WizardKey::Game(WizardStep::Review, _)) => "game_review",
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
                Tab::Accounts if self.shell.providers.visible => Page::Providers,
                Tab::Instances | Tab::Profiles | Tab::Accounts => Page::Browser,
                Tab::Settings => Page::Settings,
                Tab::About => Page::About,
            },
        };
        let frame = if page == Page::Home && self.form.is_none()
            || page == Page::FirstRun && self.shell.first_run.stage == Stage::Welcome
        {
            FrameClass::Home
        } else {
            FrameClass::Management
        };
        let layout = match page {
            Page::Browser | Page::Providers | Page::Settings | Page::About => {
                LayoutKind::Management
            }
            Page::Authentication | Page::GameWizard => LayoutKind::Wizard,
            Page::Home | Page::FirstRun => LayoutKind::Plain,
        };
        let detail = match page {
            Page::Settings => DetailKey::Settings(self.shell.settings_page),
            Page::Providers => DetailKey::Provider(self.shell.providers.motion_key()),
            _ => DetailKey::Browser {
                tab,
                inspected: self.collection().inspected.clone(),
                editor_page: self.shell.editor.page,
                profile_form: matches!(self.form, Some(Form::Profile(_))),
                instance_editor: self
                    .shell
                    .instance_editor
                    .as_ref()
                    .map(|editor| editor.identity().to_owned()),
            },
        };
        let wizard = if let Some(auth) = &self.shell.auth {
            Some(WizardKey::Authentication(auth.motion_key()))
        } else if matches!(self.form, Some(Form::Instance(_))) {
            Some(WizardKey::Game(
                self.shell.wizard.step,
                self.shell.wizard.page,
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
                providers: self.shell.providers.visible,
            },
            detail,
            wizard,
            frame,
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
    use super::{DetailKey, FrameClass, LayoutKind, Page, PageKey, PageSpec, Router, Tab, Task};

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
    fn cancelling_auth_restores_the_task_destination_not_just_its_tab() {
        let mut router = Router::new(Tab::Instances);
        router.enter_game_wizard();
        router.begin_auth();
        assert_eq!(router.tab(), Tab::Accounts);
        assert_eq!(router.auth_return(), Some(Tab::Instances));
        assert_eq!(router.end_auth(true), Some(Tab::Instances));
        assert_eq!(router.tab(), Tab::Instances);
        assert_eq!(router.current.task, Some(Task::GameWizard));
        assert_eq!(router.auth_return(), None);
    }

    #[test]
    fn page_labels_never_include_detail_identity() {
        let mut spec = PageSpec {
            page: Page::Browser,
            key: PageKey {
                tab: Tab::Profiles,
                auth: false,
                task: false,
                providers: false,
            },
            detail: DetailKey::Browser {
                tab: Tab::Profiles,
                inspected: Some("private-profile".into()),
                editor_page: None,
                profile_form: false,
                instance_editor: Some("private-instance".into()),
            },
            wizard: None,
            frame: FrameClass::Management,
            layout: LayoutKind::Management,
            preset: super::MotionStyle::Navigation,
            task: false,
        };
        assert_eq!(spec.page_label(), "browser");
        assert_eq!(spec.tab_label(), "profiles");
        assert_eq!(spec.detail_label(), "instance_editor");
        assert_eq!(spec.wizard_label(), "none");
        spec.detail = DetailKey::Provider(super::ProviderKey::Existing(uuid::Uuid::from_u128(42)));
        assert_eq!(spec.detail_label(), "provider_existing");
    }
}
