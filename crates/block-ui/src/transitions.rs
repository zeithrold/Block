//! Product bindings for the shared motion lifecycle. Retained layers contain
//! frozen render data and no engine, credential URL, receivers, or file guards.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Transitions are private product plumbing."
)]
use super::{
    Block, Form, InstanceForm, ProfileForm, Tab, frozen_input,
    motion::{self, Direction, Layer, MotionStyle, Preferences, Scope, Transition},
    shell::ShellState,
};
use gpui::{
    AnyElement, Context, EntityId, Global, IntoElement as _, ParentElement as _, Styled as _,
    WeakEntity, Window, div,
};
use std::{collections::HashMap, rc::Rc};

type Scene = Transition<String, Rc<Block>>;

/// Base's motion policy belongs to the App, so every live product window must
/// contribute before applying it. Weak ownership permits a later render to
/// discard closed/released windows without retaining their preferences.
#[derive(Default)]
struct MotionPolicies(HashMap<EntityId, (WeakEntity<Block>, bool)>);
impl Global for MotionPolicies {}

#[derive(Debug)]
pub(super) struct MotionState {
    pub page: Scene,
    pub detail: Scene,
    pub wizard: Scene,
    pub direction: Direction,
}
impl Default for MotionState {
    fn default() -> Self {
        Self {
            page: Scene::new(Scope::Page),
            detail: Scene::new(Scope::Detail),
            wizard: Scene::new(Scope::Wizard),
            direction: Direction::Forward,
        }
    }
}
impl MotionState {
    #[expect(
        clippy::unused_self,
        reason = "The product motion entry synchronizes an App-wide Base policy, shared by live product windows."
    )]
    pub(super) fn apply_policy(&self, reduced: bool, cx: &mut Context<'_, Block>) {
        let id = cx.entity_id();
        let weak = cx.weak_entity();
        let combined = {
            let policies = cx.default_global::<MotionPolicies>();
            policies.0.retain(|_, (view, _)| view.upgrade().is_some());
            policies.0.insert(id, (weak, reduced));
            policies.0.values().any(|(_, reduced)| *reduced)
        };
        if cx.reduce_motion() != combined {
            cx.set_reduce_motion(combined);
        }
    }

    fn scene(&self, scope: Scope) -> &Scene {
        match scope {
            Scope::Page => &self.page,
            Scope::Detail => &self.detail,
            Scope::Wizard => &self.wizard,
        }
    }
}

impl Block {
    #[expect(
        clippy::too_many_lines,
        reason = "Reconcile all independent scopes from one frozen visual snapshot and advance them with one clock value."
    )]
    pub(super) fn prepare_motion(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.visual_only {
            return;
        }
        let now = cx.background_executor().now();
        let preferences = self.motion_preferences();
        if self.motion_dirty {
            let snapshot = Rc::new(self.visual_snapshot(cx));
            let page = format!(
                "{:?}:{}:{}:{}",
                self.tab,
                self.shell.auth.is_some(),
                matches!(self.form, Some(Form::Instance(_))) && self.tab != Tab::Onboarding,
                self.shell.providers.visible
            );
            let detail = match self.tab {
                Tab::Accounts if self.shell.providers.visible => {
                    format!("provider:{}", self.shell.providers.motion_key())
                }
                Tab::Settings => format!("settings:{}", self.shell.settings_page),
                _ => format!(
                    "{:?}:{:?}:{:?}:{}:{:?}",
                    self.tab,
                    self.collection().inspected,
                    self.shell.editor.page,
                    matches!(self.form, Some(Form::Profile(_))),
                    self.shell
                        .instance_editor
                        .as_ref()
                        .map(super::instance_editor::InstanceEditor::identity)
                ),
            };
            let wizard = if let Some(auth) = &self.shell.auth {
                Some(format!("auth:{}", auth.motion_key()))
            } else if self.tab == Tab::Onboarding {
                Some(format!("setup:{:?}", self.shell.onboarding.step))
            } else if matches!(self.form, Some(Form::Instance(_))) {
                Some(format!(
                    "game:{:?}:{:?}",
                    self.shell.wizard.step, self.shell.wizard.page
                ))
            } else {
                None
            };
            let page_changed = self
                .motion
                .page
                .current()
                .is_some_and(|current| current.key != page);
            let wizard_changed = self
                .motion
                .wizard
                .current()
                .is_some_and(|current| Some(&current.key) != wizard.as_ref());
            self.motion.page.reconcile(
                Some(Layer {
                    key: page,
                    _snapshot: snapshot.clone(),
                }),
                self.motion.direction,
                MotionStyle::Navigation,
                now,
                preferences,
            );
            self.motion.detail.reconcile(
                Some(Layer {
                    key: detail,
                    _snapshot: snapshot.clone(),
                }),
                self.motion.direction,
                MotionStyle::Navigation,
                now,
                preferences,
            );
            self.motion.wizard.reconcile(
                wizard.map(|key| Layer {
                    key,
                    _snapshot: snapshot.clone(),
                }),
                self.motion.direction,
                MotionStyle::Navigation,
                now,
                preferences,
            );
            if page_changed {
                self.motion.detail.settle();
                self.motion.wizard.settle();
            } else if wizard_changed {
                self.motion.detail.settle();
            }
            self.motion_dirty = false;
        }
        let mut running = false;
        for (index, scene) in [
            &mut self.motion.page,
            &mut self.motion.detail,
            &mut self.motion.wizard,
        ]
        .into_iter()
        .enumerate()
        {
            running |= scene
                .frame_with_base(("product-motion", index), preferences, window, cx)
                .running();
        }
        if running {
            window.request_animation_frame();
        }
    }

    pub(super) fn motion_preferences(&self) -> Preferences {
        Preferences {
            app_reduced: self.visual_only || self.preferences.reduce_motion,
            system_reduced: super::platform::reduce_motion(false),
        }
    }

    pub(super) fn transition_content(
        &self,
        scope: Scope,
        content: AnyElement,
        _window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        if self.visual_only {
            return content;
        }
        let scene = self.motion.scene(scope);
        let now = cx.background_executor().now();
        // prepare_motion has already settled elapsed transitions; the immutable
        // renderer samples the same frame without changing lifetime ownership.
        let frame = scene.sample(now, self.motion_preferences());
        let viewport = div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .relative()
            .overflow_hidden();
        viewport
            .child(motion::layer(
                ("motion-current", scope as usize),
                frame.current.unwrap_or_default(),
                content,
            ))
            .into_any_element()
    }

    fn visual_snapshot(&self, cx: &mut Context<'_, Self>) -> Self {
        let form = self.form.as_ref().map(|form| match form {
            Form::Instance(form) => Form::Instance(InstanceForm {
                name: frozen_input(&form.name, cx),
                version: frozen_input(&form.version, cx),
                directory: frozen_input(&form.directory, cx),
                loader: form.loader.clone(),
                loader_version: form.loader_version.clone(),
                optifine: form.optifine.clone(),
            }),
            Form::Profile(form) => Form::Profile(Box::new(ProfileForm {
                original: form.original.clone(),
                name: frozen_input(&form.name, cx),
                memory: frozen_input(&form.memory, cx),
                java: frozen_input(&form.java, cx),
                jvm: frozen_input(&form.jvm, cx),
                game: frozen_input(&form.game, cx),
                server: frozen_input(&form.server, cx),
                managed: form.managed,
            })),
        });
        let downloads = self.downloads.as_ref().map(|tracker| {
            let snapshot = tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            std::sync::Arc::new(std::sync::Mutex::new(snapshot))
        });
        Self {
            build_info: self.build_info,
            shell: self.shell.snapshot(cx),
            motion: MotionState::default(),
            motion_dirty: false,
            visual_only: true,
            pending_window_size: None,
            onboarding_progress: self.onboarding_progress,
            settings_form: self.settings_form.as_ref().map(|draft| draft.snapshot(cx)),
            preferences: self.preferences.clone(),
            saved_preferences: self.saved_preferences.clone(),
            palette: self.palette,
            snapshot: self.snapshot.clone(),
            accounts: self.accounts.clone(),
            versions: self.versions.clone(),
            version_load: self.version_load.clone(),
            version_catalog: self.version_catalog.clone(),
            selected_instance: self.selected_instance.clone(),
            selected_profile: self.selected_profile.clone(),
            pending_profile: None,
            selected_account: self.selected_account.clone(),
            tab: self.tab,
            form,
            engine: None,
            messages: None,
            focus: cx.focus_handle(),
            status: self.status.clone(),
            completed: self.completed,
            operation_kind: self.operation_kind,
            operation_revision: self.operation_revision,
            pending_creation: None,
            downloads,
            downloads_revision: self.downloads_revision,
            busy: self.busy,
            operation_cancellable: self.operation_cancellable,
            cancel: block_launcher::CancellationToken::new(),
            play_stop: block_launcher::CancellationToken::new(),
            active_run: self.active_run.clone(),
            active_started_at: self.active_started_at,
            session_revisions: self.session_revisions.clone(),
            pending_crashes: std::collections::VecDeque::default(),
            crash_help: None,
            crash_guard: None,
            logs: self.logs.clone(),
            show_output: self.show_output,
            command: self.command.clone(),
            show_command: self.show_command,
            recipe: self.recipe.clone(),
            recipe_load: super::RecipeLoadState::default(),
        }
    }
}

impl ShellState {
    fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        Self {
            home: self.home.clone(),
            connection: self.connection.clone(),
            storage: self.storage.clone(),
            surfaces: self.surfaces.snapshot(),
            home_scroll: frozen_scroll(&self.home_scroll),
            about_scroll: frozen_scroll(&self.about_scroll),
            output_scroll: frozen_scroll(&self.output_scroll),
            downloads: self.downloads.snapshot(),
            toast: self.toast.clone(),
            auth: self.auth.as_ref().map(|auth| auth.snapshot(cx)),
            editor: self.editor.snapshot(),
            instance_editor: self
                .instance_editor
                .as_ref()
                .map(|editor| editor.snapshot(cx)),
            games: self.games.snapshot(cx),
            profiles: self.profiles.snapshot(cx),
            accounts: self.accounts.snapshot(cx),
            providers: self.providers.snapshot(cx),
            selector: self.selector.as_ref().map(|selector| selector.snapshot(cx)),
            revision: self.revision,
            backwards: self.backwards,
            settings_page: self.settings_page,
            wizard: self.wizard.snapshot(cx),
            onboarding: self.onboarding.snapshot(cx),
            system_window_chrome: self.system_window_chrome,
        }
    }
}
fn frozen_scroll(source: &gpui::ScrollHandle) -> gpui::ScrollHandle {
    let scroll = gpui::ScrollHandle::new();
    scroll.set_offset(source.offset());
    scroll
}

#[cfg(test)]
mod policy_tests {
    use crate::{Block, Tab, UiAction, test_support};
    use gpui::{Entity, TestAppContext, VisualTestContext};

    fn window(cx: &mut TestAppContext) -> (Entity<Block>, VisualTestContext) {
        let (view, cx) = test_support::window(cx, crate::tests::test_view);
        (view, cx.clone())
    }

    fn preview(view: &Entity<Block>, cx: &mut VisualTestContext, reduced: bool) {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                if view.preferences.reduce_motion != reduced {
                    view.handle(UiAction::ReduceMotion, window, cx);
                }
            });
        });
        cx.run_until_parked();
    }

    fn cancel(view: &Entity<Block>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Home), window, cx);
            });
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn base_roots_share_motion_policy_across_real_preview_and_cancel(cx: &mut TestAppContext) {
        let system_reduced = crate::platform::reduce_motion(false);
        let (first, mut first_cx) = window(cx);
        let (second, mut second_cx) = window(cx);
        preview(&second, &mut second_cx, true);
        assert!(first_cx.update(|_, cx| cx.reduce_motion()));

        for _ in 0..3 {
            first.update(&mut first_cx, |_, cx| cx.notify());
            second.update(&mut second_cx, |_, cx| cx.notify());
            first_cx.run_until_parked();
            assert!(first_cx.update(|_, cx| cx.reduce_motion()));
        }
        preview(&first, &mut first_cx, true);
        cancel(&first, &mut first_cx);
        assert!(
            first_cx.update(|_, cx| cx.reduce_motion()),
            "canceling one preview must preserve the other live window's request"
        );
        assert!(second.read_with(&second_cx, |view, _| {
            view.preferences.reduce_motion && !view.saved_preferences.reduce_motion
        }));
        cancel(&second, &mut second_cx);
        assert_eq!(
            first_cx.update(|_, cx| cx.reduce_motion()),
            system_reduced,
            "both canceled previews restore the effective saved/system policy"
        );
    }

    #[gpui::test]
    fn released_base_root_does_not_keep_the_app_motion_policy_enabled(cx: &mut TestAppContext) {
        let system_reduced = crate::platform::reduce_motion(false);
        let (first, mut first_cx) = window(cx);
        let (second, mut second_cx) = window(cx);
        preview(&second, &mut second_cx, true);
        assert!(first_cx.update(|_, cx| cx.reduce_motion()));
        second_cx.update(|window, _| window.remove_window());
        drop(second);
        drop(second_cx);
        first_cx.run_until_parked();
        first.update(&mut first_cx, |_, cx| cx.notify());
        first_cx.run_until_parked();
        assert_eq!(
            first_cx.update(|_, cx| cx.reduce_motion()),
            system_reduced,
            "a subsequent live render discards the released window's weak entry"
        );
        preview(&first, &mut first_cx, true);
        assert!(first_cx.update(|_, cx| cx.reduce_motion()));
        cancel(&first, &mut first_cx);
        assert_eq!(first_cx.update(|_, cx| cx.reduce_motion()), system_reduced);
    }
}
