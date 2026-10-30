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
    App, AppContext as _, Context, FocusHandle, Focusable as _, IntoElement, ParentElement as _,
    Render, Styled as _, WeakEntity, Window, div, px,
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

    pub(super) fn discard(&mut self) -> Option<Surface> {
        self.entries.pop().map(|entry| entry.kind)
    }

    pub(super) fn open(
        &mut self,
        kind: Surface,
        window: &mut Window,
        cx: &mut Context<'_, Block>,
    ) -> bool {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.kind.purpose() == kind.purpose())
        {
            if self.entries[index].kind.overlay().is_some() {
                // A wizard section can switch pages while its one dialog stays
                // mounted; keep the logical layer and let the body re-read it.
                self.entries[index].kind = kind;
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
            return true;
        }
        let Some(presentation) = presentation else {
            return false;
        };
        self.entries.push(entry);
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
        self.entries.clear();
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
        let body = cx.new(|cx| {
            if let Some(owner) = observer.upgrade() {
                cx.observe(&owner, |_, _, cx| cx.notify()).detach();
            }
            SurfaceOverlayBody {
                owner: observer,
                revision,
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
            window.open_dialog(cx, move |dialog, _dialog_window, dialog_cx| {
                let Some(view) = build_owner.upgrade() else {
                    return dialog;
                };
                let section = matches!(kind, Surface::WizardSection(_));
                let title = view.read(dialog_cx).surface_dialog_title(&kind);
                let body = body.clone();
                let gate = build_owner.clone();
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
                                .text_xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(title),
                        )
                };
                dialog
                    .content(move |content, _window, _cx| content.child(body.clone()))
                    .on_cancel(move |_, _, cx| {
                        gate.upgrade().is_none_or(|view| {
                            let view = view.read(cx);
                            !(view.busy && !view.can_stop())
                        })
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
}

impl Render for SurfaceOverlayBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let revision = self.revision;
        self.owner
            .update(cx, |view, cx| {
                view.render_surface_dialog_body(revision, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
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
        let entry = self.shell.surfaces.entries.remove(index);
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
    pub(super) fn purpose(&self) -> String {
        match self {
            Self::Drawer(drawer) => format!("drawer:{drawer:?}"),
            Self::DownloadFiles(group) => format!("download-files:{group:?}"),
            _ => format!("{:?}", std::mem::discriminant(self)),
        }
    }

    pub(super) fn title(&self) -> &str {
        match self {
            Self::Drawer(Drawer::Menu) => "Block menu",
            Self::Drawer(Drawer::Accounts) => "Switch account",
            Self::Drawer(Drawer::Profiles) => "Switch game",
            Self::Drawer(Drawer::Downloads) => "Downloads",
            Self::Drawer(Drawer::ImportGames) => "Import games",
            Self::DownloadFiles(group) => group.label(),
            Self::DeleteProfile(_) => "Delete profile",
            Self::RemoveAccount(_) => "Remove account",
            Self::RemoveProvider(_) => "Remove provider",
            Self::SeparateProfiles => "Enable separated profiles",
            Self::GamesHelp => "Instances and Profiles",
            Self::WizardSection(page) => match page {
                super::WizardPage::Version => "Minecraft version",
                super::WizardPage::Loader => "Mod loader",
                super::WizardPage::OptiFine => "OptiFine",
                super::WizardPage::Location => "Game configuration",
            },
            Self::AuthProviderAdd => "Add authentication provider",
            Self::Select(state) => &state.title,
        }
    }

    /// The library overlay for this surface. Select keeps its anchored in-window
    /// popup because a centered modal would not match the prototype pickers.
    pub(super) fn overlay(&self) -> Option<OverlayPresentation> {
        match self {
            Self::Drawer(Drawer::Menu) | Self::Select(_) => None,
            Self::Drawer(
                Drawer::Accounts | Drawer::Profiles | Drawer::Downloads | Drawer::ImportGames,
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
