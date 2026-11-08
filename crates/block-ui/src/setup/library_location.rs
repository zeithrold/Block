//! A library choice belongs to its creation flow until final confirmation.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Location state is shared with the parent view and confirmation owners."
)]

use std::path::{Path, PathBuf};

use gpui::{
    Context, EntityId, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions,
    StatefulInteractiveElement as _, Styled as _, div, rgb,
};

use super::{
    Block, Form, UiAction,
    components::ButtonVariant,
    first_run,
    game_wizard::{CreationKind, WizardStep},
    import_games,
    shell::{Drawer, Surface},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Game(EntityId),
    Import(usize),
    Pack(EntityId, PathBuf),
}

#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub scope: Scope,
    pub parent: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct Picker {
    pub id: uuid::Uuid,
    pub scope: Scope,
    pub wizard_revision: usize,
    pub imports: Vec<String>,
    pub import_source: Option<block_launcher::game_import::SourceKind>,
}

impl Block {
    pub(super) fn setup_game_library_scope(&self) -> Option<Scope> {
        if !self.shell.first_run.enabled
            || !self.setup_incomplete()
            || self.shell.wizard.kind != CreationKind::NewInstance
            || self.shell.wizard.step != WizardStep::Setup
        {
            return None;
        }
        let Some(Form::Instance(form)) = &self.form else {
            return None;
        };
        Some(Scope::Game(form.directory.entity_id()))
    }

    pub(super) fn setup_import_library_scope(&self) -> Option<Scope> {
        if !self.shell.first_run.enabled || !self.setup_incomplete() {
            return None;
        }
        self.shell.surfaces.top().and_then(|surface| {
            matches!(surface.kind, Surface::Drawer(Drawer::ImportGames))
                .then_some(Scope::Import(surface.revision))
        })
    }

    pub(super) fn setup_pack_library_scope(&self) -> Option<Scope> {
        let state = &self.shell.pack_import;
        if !self.shell.first_run.enabled || !self.setup_incomplete() {
            return None;
        }
        let scope = Scope::Pack(state.name.as_ref()?.entity_id(), state.source.clone()?);
        if state.instance.is_some() && self.shell.first_run.library_saved.as_ref() != Some(&scope) {
            return None;
        }
        Some(scope)
    }

    pub(super) fn setup_library_parent(&self, scope: &Scope) -> PathBuf {
        self.shell
            .first_run
            .library_draft
            .as_ref()
            .filter(|draft| {
                &draft.scope == scope
                    && !(matches!(scope, Scope::Pack(..))
                        && self.shell.pack_import.instance.is_some())
            })
            .map_or_else(
                || self.first_run_game_parent(),
                |draft| draft.parent.clone(),
            )
    }

    fn library_scope_current(&self, scope: &Scope) -> bool {
        if self.busy {
            return false;
        }
        match scope {
            Scope::Game(_) => {
                self.setup_game_library_scope().as_ref() == Some(scope)
                    && self.shell.first_run.stage == first_run::Stage::GameWizard
                    && self.shell.wizard.page.is_none()
                    && self.shell.surfaces.top().is_none()
            }
            Scope::Import(_) => {
                self.setup_import_library_scope().as_ref() == Some(scope)
                    && !self.shell.import_games.running
                    && matches!(
                        self.shell.import_games.page,
                        import_games::Page::Sources | import_games::Page::List
                    )
            }
            Scope::Pack(..) => {
                self.setup_pack_library_scope().as_ref() == Some(scope)
                    && self.shell.wizard.kind == CreationKind::ImportPack
                    && self.shell.pack_import.request.is_none()
                    && self.shell.pack_import.instance.is_none()
                    && self.shell.surfaces.top().is_none()
            }
        }
    }

    pub(super) fn browse_setup_library(&mut self, scope: Scope, cx: &mut Context<'_, Self>) {
        if !self.library_scope_current(&scope) {
            return;
        }
        let id = uuid::Uuid::new_v4();
        let import_source =
            matches!(scope, Scope::Import(_)).then_some(self.shell.import_games.source);
        self.shell.first_run.library_picker = Some(Picker {
            id,
            scope,
            wizard_revision: self.shell.wizard.revision,
            imports: self.shell.import_games.selected.iter().cloned().collect(),
            import_source,
        });
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr!(self.language(), "setup.library.choose").into()),
        });
        cx.spawn(async move |view, cx| {
            let selection = match selection.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(()),
            };
            let _ = view.update(cx, |view, cx| {
                view.finish_setup_library_browse(id, selection, cx);
            });
        })
        .detach();
    }

    pub(super) fn finish_setup_library_browse(
        &mut self,
        id: uuid::Uuid,
        selection: Result<Option<PathBuf>, ()>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(picker) = self
            .shell
            .first_run
            .library_picker
            .as_ref()
            .filter(|picker| picker.id == id)
            .cloned()
        else {
            return;
        };
        self.shell.first_run.library_picker = None;
        if !self.library_scope_current(&picker.scope)
            || matches!(picker.scope, Scope::Game(_))
                && picker.wizard_revision != self.shell.wizard.revision
            || matches!(picker.scope, Scope::Import(_))
                && (picker.import_source != Some(self.shell.import_games.source)
                    || picker.imports
                        != self
                            .shell
                            .import_games
                            .selected
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>())
        {
            return;
        }
        match selection {
            Ok(Some(parent)) => {
                let old = self.setup_library_parent(&picker.scope);
                let accepted = self.first_run_game_parent();
                let parent = if super::setup_storage::comparable_path(&parent)
                    == super::setup_storage::comparable_path(&accepted)
                {
                    accepted.clone()
                } else {
                    parent
                };
                self.update_generated_game_directory(&picker.scope, &old, &parent, cx);
                self.shell.first_run.library_error = None;
                if parent != accepted {
                    self.shell.first_run.library_saved = None;
                }
                self.shell.first_run.library_draft = (parent != accepted).then_some(Draft {
                    scope: picker.scope,
                    parent,
                });
            }
            Ok(None) => return,
            Err(()) => {
                self.library_location_error(&picker.scope, msg!("setup.library.choose_failed"))
            }
        }
        cx.notify();
    }

    fn update_generated_game_directory(
        &mut self,
        scope: &Scope,
        old: &Path,
        parent: &Path,
        cx: &mut Context<'_, Self>,
    ) {
        let Scope::Game(field) = scope else { return };
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        if form.directory.entity_id() != *field {
            return;
        }
        let directory = PathBuf::from(form.directory.read(cx).value().trim());
        if self
            .shell
            .first_run
            .generated_game
            .as_ref()
            .is_some_and(|(id, generated)| id == field && generated == &directory)
            && directory.parent() == Some(old)
            && let Some(leaf) = directory.file_name()
        {
            let value = parent.join(leaf).display().to_string();
            self.shell.first_run.generated_game = Some((*field, PathBuf::from(&value)));
            form.directory.update(cx, |field, cx| {
                field.set_value(&value, cx);
            });
        }
    }

    pub(super) fn library_location_error(&mut self, scope: &Scope, error: crate::i18n::UiText) {
        self.shell.first_run.library_error = Some((scope.clone(), error));
    }

    /// Only an eligible final confirmation accepts the pending default.
    pub(super) fn commit_setup_library(
        &mut self,
        scope: &Scope,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(draft) = self
            .shell
            .first_run
            .library_draft
            .as_ref()
            .filter(|draft| &draft.scope == scope)
            .cloned()
        else {
            if self
                .shell
                .first_run
                .library_error
                .as_ref()
                .is_some_and(|(owner, _)| owner == scope)
            {
                self.shell.first_run.library_error = None;
            }
            return true;
        };
        if !self.library_scope_current(scope) {
            return false;
        }
        let result = self
            .engine
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Launcher storage is unavailable"))
            .and_then(|engine| super::setup_storage::save_game_parent(&engine.root, &draft.parent));
        match result {
            Ok(parent) => {
                self.update_generated_game_directory(scope, &draft.parent, &parent, cx);
                self.shell.first_run.game_parent = Some(parent);
                self.shell.first_run.library_draft = None;
                self.shell.first_run.library_error = None;
                self.shell.first_run.library_saved = Some(scope.clone());
                true
            }
            Err(error) => {
                self.library_location_error(
                    scope,
                    msg!("setup.library.save_failed", detail = format!("{error:#}")),
                );
                cx.notify();
                false
            }
        }
    }

    pub(super) fn setup_library_location(
        &self,
        scope: Scope,
        interactive: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let parent = self.setup_library_parent(&scope);
        let actual_game = match (&scope, &self.form) {
            (Scope::Game(_), Some(Form::Instance(form))) => {
                let directory = PathBuf::from(form.directory.read(cx).value().trim());
                directory
                    .parent()
                    .filter(|directory_parent| {
                        super::setup_storage::comparable_path(directory_parent)
                            != super::setup_storage::comparable_path(&parent)
                    })
                    .map(|_| {
                        tr!(
                            self.language(),
                            "setup.library.actual_game",
                            path = directory.display()
                        )
                    })
            }
            _ => None,
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(super::note(tr!(self.language(), "setup.library.label")))
                    .children(
                        (interactive
                            && !matches!(scope, Scope::Import(_))
                            && self.library_scope_current(&scope))
                        .then(|| {
                            self.button_with_variant(
                                "setup-library-change",
                                tr!(self.language(), "setup.library.change"),
                                UiAction::FirstRun(first_run::Action::ChooseLibrary(scope.clone())),
                                ButtonVariant::Ghost,
                                cx,
                            )
                        }),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_sm()
                    .whitespace_normal()
                    .child(parent.display().to_string()),
            )
            .child(super::note(tr!(self.language(), "setup.library.scope")))
            .children(actual_game.map(|actual| super::note(actual).w_full().whitespace_normal()))
            .children(
                self.shell
                    .first_run
                    .library_error
                    .as_ref()
                    .filter(|(owner, _)| interactive && owner == &scope)
                    .map(|(_, error)| {
                        super::note(error.text(self.language()))
                            .id("setup-library-error")
                            .role(gpui::Role::Alert)
                            .text_color(rgb(self.palette.destructive))
                            .w_full()
                            .whitespace_normal()
                    }),
            )
            .children(
                self.shell
                    .first_run
                    .library_saved
                    .as_ref()
                    .filter(|owner| interactive && *owner == &scope)
                    .map(|_| super::note(tr!(self.language(), "setup.library.saved"))),
            )
            .children(
                self.shell
                    .first_run
                    .library_draft
                    .as_ref()
                    .filter(|draft| draft.scope == scope)
                    .filter(|_| {
                        !(matches!(scope, Scope::Pack(..))
                            && self.shell.pack_import.instance.is_some())
                    })
                    .map(|_| super::note(tr!(self.language(), "setup.library.pending"))),
            )
            .into_any_element()
    }
}
