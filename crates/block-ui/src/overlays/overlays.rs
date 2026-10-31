//! Product overlays: one logical layer stack presented through the component
//! library's modal primitives inside the product window.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Overlay ownership is internal to the product."
)]

use super::{
    Block,
    shell::{Drawer, OverlayPresentation, Surface},
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, WeakEntity, Window, div, px,
};
use gpui_component::WindowExt as _;
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug)]
pub(super) struct SurfaceLayer {
    pub kind: Surface,
    pub focus: FocusHandle,
    pub revision: usize,
    return_focus: Option<FocusHandle>,
}

/// The single product overlay stack. The library owns modal input/focus, while
/// DrawerLayer retains product content through its exit. This list decides what
/// is open, in which order, and how a dismissal settles product state.
#[derive(Clone, Debug, Default)]
pub(super) struct Overlays {
    trigger_focus: RefCell<HashMap<&'static str, FocusHandle>>,
    entries: Vec<SurfaceLayer>,
    revision: usize,
    view_id: Option<u64>,
}

impl Overlays {
    pub(super) fn snapshot(&self) -> Self {
        self.clone()
    }

    pub(super) fn trigger_focus(&self, id: &'static str, cx: &App) -> FocusHandle {
        self.trigger_focus
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    pub(super) fn top(&self) -> Option<&SurfaceLayer> {
        self.entries.last()
    }

    pub(super) fn top_mut(&mut self) -> Option<&mut SurfaceLayer> {
        self.entries.last_mut()
    }

    pub(super) fn layers(&self) -> impl Iterator<Item = &SurfaceLayer> {
        self.entries.iter()
    }

    pub(super) fn is_open(&self) -> bool {
        self.top().is_some()
    }

    pub(super) fn contains(&self, revision: usize) -> bool {
        self.entries.iter().any(|entry| entry.revision == revision)
    }

    /// Whether this sheet directly owns the current anchored selector.
    pub(super) fn select_above(&self, revision: usize) -> bool {
        matches!(
            self.entries.as_slice(),
            [.., parent, top]
                if parent.revision == revision && matches!(top.kind, Surface::Select(_))
        )
    }

    pub(super) fn discard(&mut self) -> Option<Surface> {
        let entry = self.entries.pop()?;
        self.trace_change(&entry, "closed");
        Some(entry.kind)
    }

    fn trace_change(&self, entry: &SurfaceLayer, state: &'static str) {
        if let Some(view_id) = self.view_id {
            tracing::debug!(
                event = "ui.overlay.changed",
                view_id,
                overlay = entry.kind.log_label(),
                overlay_revision = entry.revision,
                depth = self.entries.len(),
                state,
            );
        }
    }

    pub(super) fn open(
        &mut self,
        kind: Surface,
        window: &mut Window,
        cx: &mut Context<'_, Block>,
    ) -> bool {
        self.view_id = Some(cx.entity_id().as_u64());
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.kind.purpose() == kind.purpose())
        {
            if self.entries[index].kind.overlay().is_some() {
                // A wizard section can switch pages while its one dialog stays
                // mounted; keep the logical layer and let the body re-read it.
                let changed = self.entries[index].kind.log_label() != kind.log_label();
                self.entries[index].kind = kind;
                if changed {
                    self.trace_change(&self.entries[index], "changed");
                }
                cx.notify();
            }
            return false;
        }
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let focus = cx.focus_handle();
        let presentation = kind.overlay();
        let overlay_kind = kind.clone();
        let entry = SurfaceLayer {
            kind,
            focus: focus.clone(),
            revision,
            return_focus: super::drawer::DrawerLayer::closing_return_focus(window, cx)
                .or_else(|| window.focused(cx)),
        };
        if matches!(entry.kind, Surface::Select(_)) {
            focus.focus(window, cx);
            self.entries.push(entry);
            if let Some(entry) = self.entries.last() {
                self.trace_change(entry, "opened");
            }
            return true;
        }
        let Some(presentation) = presentation else {
            return false;
        };
        self.entries.push(entry);
        if let Some(entry) = self.entries.last() {
            self.trace_change(entry, "opened");
        }
        match presentation {
            OverlayPresentation::Dialog => {
                Self::open_surface_dialog(overlay_kind, revision, window, cx);
            }
            OverlayPresentation::Sheet => {
                Self::open_surface_sheet(overlay_kind, self.entries.last().cloned(), window, cx);
            }
        }
        true
    }

    pub(super) fn close(
        &mut self,
        window: &mut Window,
        fallback: &FocusHandle,
        cx: &mut App,
    ) -> Option<Surface> {
        let entry = self.entries.pop()?;
        self.trace_change(&entry, "closed");
        match entry.kind.overlay() {
            Some(OverlayPresentation::Dialog) => window.close_dialog(cx),
            Some(OverlayPresentation::Sheet)
                if !super::drawer::DrawerLayer::close(entry.revision, None, window, cx) =>
            {
                entry
                    .return_focus
                    .as_ref()
                    .unwrap_or(fallback)
                    .focus(window, cx);
            }
            Some(OverlayPresentation::Sheet) | None => {}
        }
        if entry.kind.overlay() != Some(OverlayPresentation::Sheet) {
            entry
                .return_focus
                .as_ref()
                .unwrap_or(fallback)
                .focus(window, cx);
        }
        Some(entry.kind)
    }

    pub(super) fn clear(&mut self, window: &mut Window, fallback: &FocusHandle, cx: &mut App) {
        let had_dialog = self
            .entries
            .iter()
            .any(|entry| entry.kind.overlay() == Some(OverlayPresentation::Dialog));
        let sheet = self
            .entries
            .iter()
            .find(|entry| entry.kind.overlay() == Some(OverlayPresentation::Sheet))
            .map(|entry| entry.revision);
        while let Some(entry) = self.entries.pop() {
            self.trace_change(&entry, "closed");
        }
        if had_dialog {
            window.close_all_dialogs(cx);
        }
        if let Some(revision) = sheet {
            if !super::drawer::DrawerLayer::close(revision, Some(fallback.clone()), window, cx) {
                fallback.focus(window, cx);
            }
        } else {
            fallback.focus(window, cx);
        }
    }

    /// Present an overlay surface through the component library's dialog layer.
    /// The logical entry remains the stack owner; the library owns the scrim,
    /// Escape, focus trap, focus restore and motion.
    fn open_surface_dialog(
        kind: Surface,
        revision: usize,
        window: &mut Window,
        cx: &mut Context<'_, Block>,
    ) {
        let owner = cx.weak_entity();
        let observer = owner.clone();
        let close_owner = owner.clone();
        let close = cx.new(|cx| {
            if let Some(owner) = close_owner.upgrade() {
                cx.observe(&owner, |_, _, cx| cx.notify()).detach();
            }
            SurfaceDialogClose { owner: close_owner }
        });
        let body_close = matches!(kind, Surface::WizardSection(_) | Surface::AuthProviderAdd)
            .then(|| close.clone());
        let body = cx.new(|cx| {
            if let Some(owner) = observer.upgrade() {
                cx.observe(&owner, |_, _, cx| cx.notify()).detach();
            }
            SurfaceOverlayBody {
                owner: observer,
                revision,
                close: body_close,
            }
        });
        // The window root and neighbouring surface state are still settling
        // while a surface opens, so mount the overlay on the next effect pass.
        window.defer(cx, move |window, cx| {
            let Some(view) = owner.upgrade() else {
                return;
            };
            if !view.read(cx).shell.surfaces.contains(revision) {
                return;
            }
            let build_owner = owner.clone();
            let body = body.clone();
            let close = close.clone();
            window.open_dialog(cx, move |dialog, _dialog_window, dialog_cx| {
                let Some(view) = build_owner.upgrade() else {
                    return dialog;
                };
                let section = matches!(kind, Surface::WizardSection(_));
                let title = view.read(dialog_cx).surface_dialog_title(&kind);
                let body = body.clone();
                let gate = build_owner.clone();
                let confirm = build_owner.clone();
                let dismiss = build_owner.clone();
                let dialog = if section {
                    dialog
                        .font_family(super::design::FONT_FAMILY)
                        .w(px(760.))
                        .px(px(0.))
                        .pt(px(0.))
                        .pb(px(0.))
                } else if matches!(kind, Surface::AuthProviderAdd) {
                    dialog
                        .font_family(super::design::FONT_FAMILY)
                        .w(px(460.))
                        .px(px(0.))
                        .pt(px(24.))
                        .pb(px(0.))
                } else {
                    dialog
                        .font_family(super::design::FONT_FAMILY)
                        .w(px(520.))
                        .px(px(0.))
                        .pt(px(24.))
                        .pb(px(0.))
                        .title(
                            div()
                                .px(px(24.))
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xl()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(close.clone()),
                        )
                };
                dialog
                    .close_button(false)
                    .content(move |content, _window, _cx| content.child(body.clone()))
                    .on_cancel(move |_, _, cx| {
                        gate.upgrade().is_none_or(|view| {
                            let view = view.read(cx);
                            !(view.busy && !view.can_stop())
                        })
                    })
                    .on_ok(move |_, window, cx| {
                        let allowed = confirm.upgrade().is_none_or(|view| {
                            let view = view.read(cx);
                            !(view.busy && !view.can_stop())
                        });
                        if allowed {
                            window.close_dialog(cx);
                            let _ = confirm.update(cx, |view, cx| {
                                view.dismiss_surface_overlay(revision, window, cx)
                            });
                        }
                        // A permitted dismissal is complete; do not schedule
                        // a second, delayed focus restoration in the library.
                        false
                    })
                    .on_close(move |_, window, cx| {
                        let _ = dismiss.update(cx, |view, cx| {
                            view.dismiss_surface_overlay(revision, window, cx)
                        });
                    })
            });
            if let Some(focus) = view.read(cx).surface_input_focus(revision, cx) {
                focus.focus(window, cx);
            }
        });
    }

    /// Present an overlay surface as a bottom sheet inside the current window.
    fn open_surface_sheet(
        kind: Surface,
        entry: Option<SurfaceLayer>,
        window: &mut Window,
        cx: &mut Context<'_, Block>,
    ) {
        let owner = cx.weak_entity();
        let Some(entry) = entry else {
            return;
        };
        let revision = entry.revision;
        window.defer(cx, move |window, cx| {
            let Some(view) = owner.upgrade() else {
                return;
            };
            if !view.read(cx).shell.surfaces.contains(revision) {
                return;
            }
            super::drawer::DrawerLayer::open(
                owner,
                kind,
                revision,
                entry.focus.clone(),
                entry.return_focus,
                window,
                cx,
            );
            let focus = view
                .read(cx)
                .surface_input_focus(revision, cx)
                .unwrap_or(entry.focus);
            focus.focus(window, cx);
        });
    }
}

/// A logical overlay surface's content. It follows the shared Block entity, so
/// its text and controls always describe current state instead of a snapshot.
struct SurfaceOverlayBody {
    owner: WeakEntity<Block>,
    revision: usize,
    close: Option<Entity<SurfaceDialogClose>>,
}

impl Render for SurfaceOverlayBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let revision = self.revision;
        let body = self
            .owner
            .update(cx, |view, cx| {
                view.render_surface_dialog_body(revision, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element());
        div()
            .relative()
            .w_full()
            .child(body)
            .children(self.close.as_ref().map(|close| {
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .child(close.clone())
            }))
    }
}

/// Product-localized trigger using Base's existing dialog-cancel action.
struct SurfaceDialogClose {
    owner: WeakEntity<Block>,
}

impl Render for SurfaceDialogClose {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let (language, palette) = self.owner.upgrade().map_or(
            (crate::Language::English, super::Palette::LIGHT),
            |owner| {
                let view = owner.read(cx);
                (view.language(), view.palette)
            },
        );
        let label = tr!(language, "common.close");
        gpui_base::DialogClose::new().trigger(move |button| {
            button
                .accessibility_label(label)
                .debug_selector(|| "surface-dialog-close".into())
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .hover(move |style| style.bg(gpui::rgb(palette.accent)))
                .child(super::design::Icon::Close.render(16., palette.foreground))
        })
    }
}

impl Block {
    fn surface_input_focus(&self, revision: usize, cx: &App) -> Option<FocusHandle> {
        let entry = self
            .shell
            .surfaces
            .entries
            .iter()
            .find(|entry| entry.revision == revision)?;
        match &entry.kind {
            Surface::Drawer(Drawer::Accounts | Drawer::Profiles) => self
                .shell
                .selector
                .as_ref()
                .map(|state| state.query_focus(cx)),
            Surface::WizardSection(super::WizardPage::Version) => self
                .shell
                .wizard
                .version_search
                .as_ref()
                .map(|input| input.read(cx).focus_handle(cx)),
            Surface::WizardSection(super::WizardPage::Loader) => self
                .shell
                .wizard
                .loader_search
                .as_ref()
                .map(|input| input.read(cx).focus_handle(cx)),
            Surface::WizardSection(super::WizardPage::Location) => match &self.form {
                Some(super::Form::Instance(form)) => Some(form.directory.read(cx).focus_handle(cx)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The library dismissed a layer itself (Escape, backdrop or close button).
    /// Drop exactly that logical layer; a late dismissal for an older revision
    /// is a no-op and never closes a newer layer.
    pub(super) fn dismiss_surface_overlay(
        &mut self,
        revision: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self
            .shell
            .surfaces
            .entries
            .iter()
            .position(|entry| entry.revision == revision)
        else {
            return;
        };
        if self.shell.surfaces.select_above(revision) {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
        let entry = self.shell.surfaces.entries.remove(index);
        self.shell.surfaces.trace_change(&entry, "closed");
        match entry.kind {
            Surface::Drawer(Drawer::ImportGames) => self.import_games_closed(window, cx),
            Surface::WizardSection(_) => {
                self.cancel_component_draft(cx);
            }
            Surface::AuthProviderAdd => self.cancel_auth_provider(cx),
            _ => {}
        }
        if entry.kind.overlay() == Some(OverlayPresentation::Sheet) {
            if !super::drawer::DrawerLayer::close(revision, None, window, cx) {
                entry
                    .return_focus
                    .as_ref()
                    .unwrap_or(&self.focus)
                    .focus(window, cx);
            }
        } else if let Some(focus) = entry.return_focus {
            focus.focus(window, cx);
        }
        self.motion_dirty = true;
        cx.notify();
    }
}

impl Surface {
    const fn log_label(&self) -> &'static str {
        match self {
            Self::Drawer(Drawer::ImportGames) => "import_games",
            Self::Drawer(Drawer::Language) => "language",
            Self::Drawer(Drawer::Menu) => "menu",
            Self::Drawer(Drawer::Accounts) => "accounts",
            Self::Drawer(Drawer::Profiles) => "profiles",
            Self::Drawer(Drawer::Downloads) => "downloads",
            Self::DownloadFiles(_) => "download_files",
            Self::DeleteProfile(_) => "delete_profile",
            Self::RemoveAccount(_) => "remove_account",
            Self::RemoveProvider(_) => "remove_provider",
            Self::SeparateProfiles => "separate_profiles",
            Self::GamesHelp => "games_help",
            Self::WizardSection(super::WizardPage::Version) => "wizard_version",
            Self::WizardSection(super::WizardPage::Loader) => "wizard_loader",
            Self::WizardSection(super::WizardPage::OptiFine) => "wizard_optifine",
            Self::WizardSection(super::WizardPage::Location) => "wizard_location",
            Self::AuthProviderAdd => "auth_provider_add",
            Self::Select(_) => "select",
        }
    }

    pub(super) fn purpose(&self) -> String {
        match self {
            Self::Drawer(drawer) => format!("drawer:{drawer:?}"),
            Self::DownloadFiles(group) => format!("download-files:{group:?}"),
            _ => format!("{:?}", std::mem::discriminant(self)),
        }
    }

    /// The surface's display name in the given language. Every name resolves
    /// from a semantic key, so a retained layer follows the painting language.
    pub(super) fn title(&self, language: super::Language) -> String {
        match self {
            Self::Drawer(Drawer::Menu) => tr!(language, "surface.menu"),
            Self::Drawer(Drawer::Language) => tr!(language, "setup.language.label"),
            Self::Drawer(Drawer::Accounts) => tr!(language, "action.switch_account"),
            Self::Drawer(Drawer::Profiles) => tr!(language, "action.switch_game"),
            Self::Drawer(Drawer::Downloads) => tr!(language, "surface.downloads"),
            Self::Drawer(Drawer::ImportGames) => tr!(language, "surface.import_games"),
            Self::DownloadFiles(group) => group.label(language),
            Self::DeleteProfile(_) => tr!(language, "action.delete_profile"),
            Self::RemoveAccount(_) => tr!(language, "action.remove_account"),
            Self::RemoveProvider(_) => tr!(language, "action.remove_provider"),
            Self::SeparateProfiles => tr!(language, "action.enable_separated"),
            Self::GamesHelp => tr!(language, "surface.games_help"),
            Self::WizardSection(page) => match page {
                super::WizardPage::Version => tr!(language, "surface.wizard_version"),
                super::WizardPage::Loader => tr!(language, "surface.wizard_loader"),
                super::WizardPage::OptiFine => "OptiFine".to_owned(),
                super::WizardPage::Location => tr!(language, "surface.wizard_location"),
            },
            Self::AuthProviderAdd => tr!(language, "surface.auth_provider_add"),
            Self::Select(state) => state.title(language),
        }
    }

    /// The library overlay for this surface. Select keeps its anchored in-window
    /// popup because a centered modal would not match the prototype pickers.
    pub(super) fn overlay(&self) -> Option<OverlayPresentation> {
        match self {
            Self::Drawer(Drawer::Menu) | Self::Select(_) => None,
            Self::Drawer(
                Drawer::Accounts
                | Drawer::Profiles
                | Drawer::Downloads
                | Drawer::ImportGames
                | Drawer::Language,
            ) => Some(OverlayPresentation::Sheet),
            Self::DownloadFiles(_)
            | Self::GamesHelp
            | Self::AuthProviderAdd
            | Self::DeleteProfile(_)
            | Self::RemoveAccount(_)
            | Self::RemoveProvider(_)
            | Self::SeparateProfiles
            | Self::WizardSection(_) => Some(OverlayPresentation::Dialog),
        }
    }
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod overlay_tests;
