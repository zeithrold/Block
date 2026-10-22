//! Product dialog windows share the launcher's entities and keep their own lifetime.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Window ownership is internal to the product."
)]

use super::{
    Block,
    shell::{Drawer, Surface},
};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, FocusHandle, Focusable as _,
    IntoElement, ParentElement as _, Render, ScrollHandle, Styled as _, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowDecorations, WindowId, WindowKind,
    WindowOptions, div, point, px, size,
};
use gpui_component::WindowExt as _;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

#[derive(Clone, Debug)]
pub(super) struct SurfaceWindow {
    pub kind: Surface,
    pub scroll: ScrollHandle,
    pub focus: FocusHandle,
    pub revision: usize,
    pub window: Option<AnyWindowHandle>,
    pub parent: AnyWindowHandle,
    return_focus: Option<FocusHandle>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SurfaceWindows {
    trigger_focus: RefCell<HashMap<&'static str, FocusHandle>>,
    entries: Vec<SurfaceWindow>,
    current_window: Cell<Option<WindowId>>,
    revision: usize,
}

impl SurfaceWindows {
    pub(super) fn enter_window(&self, window: &Window) -> Option<WindowId> {
        self.current_window
            .replace(Some(window.window_handle().window_id()))
    }
    pub(super) fn restore_window(&self, previous: Option<WindowId>) {
        self.current_window.set(previous);
    }
    pub(super) fn activate(&self, window: &Window) {
        self.current_window
            .set(Some(window.window_handle().window_id()));
    }
    pub(super) fn snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        for entry in &mut snapshot.entries {
            let scroll = ScrollHandle::new();
            scroll.set_offset(entry.scroll.offset());
            entry.scroll = scroll;
        }
        snapshot
    }
    pub(super) fn trigger_focus(&self, id: &'static str, cx: &App) -> FocusHandle {
        self.trigger_focus
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }
    fn current_index(&self) -> Option<usize> {
        let current = self.current_window.get()?;
        self.entries.iter().rposition(|entry| {
            entry
                .window
                .map_or(entry.parent.window_id() == current, |window| {
                    window.window_id() == current
                })
        })
    }
    pub(super) fn top(&self) -> Option<&SurfaceWindow> {
        self.current_index()
            .and_then(|index| self.entries.get(index))
    }
    pub(super) fn top_mut(&mut self) -> Option<&mut SurfaceWindow> {
        self.current_index()
            .and_then(|index| self.entries.get_mut(index))
    }
    pub(super) fn layers(&self) -> impl Iterator<Item = &SurfaceWindow> {
        self.entries.iter()
    }
    pub(super) fn is_open(&self) -> bool {
        self.top().is_some()
    }
    pub(super) fn discard(&mut self) -> Option<Surface> {
        let index = self.current_index()?;
        Some(self.entries.remove(index).kind)
    }
    pub(super) fn open(
        &mut self,
        kind: Surface,
        window: &mut Window,
        cx: &mut Context<'_, Block>,
    ) -> bool {
        let parent = window.window_handle();
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| entry.kind.purpose() == kind.purpose())
        {
            if let Some(handle) = existing.window {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                self.current_window.set(Some(handle.window_id()));
            }
            return false;
        }
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let focus = cx.focus_handle();
        let overlay = kind.is_overlay();
        let dialog_kind = kind.clone();
        let entry = SurfaceWindow {
            kind,
            scroll: ScrollHandle::new(),
            focus: focus.clone(),
            revision,
            window: None,
            parent,
            return_focus: window.focused(cx),
        };
        self.current_window.set(Some(parent.window_id()));
        if matches!(entry.kind, Surface::Select(_)) {
            focus.focus(window, cx);
            self.entries.push(entry);
            return true;
        }
        if overlay {
            self.entries.push(entry);
            Self::open_surface_dialog(dialog_kind, revision, window, cx);
            return true;
        }
        let title = entry.kind.title().to_owned();
        let preferred = entry.kind.window_size();
        let dimensions = window.display(cx).map_or(preferred, |display| {
            size(
                preferred
                    .width
                    .min((display.bounds().size.width - px(32.)).max(px(320.))),
                preferred
                    .height
                    .min((display.bounds().size.height - px(96.)).max(px(220.))),
            )
        });
        let origin = window.bounds().origin
            + point(
                (window.bounds().size.width - dimensions.width).max(px(0.)) / 2.,
                px(40.),
            );
        self.entries.push(entry);
        let owner = cx.weak_entity();
        // Opening may render synchronously. Return the shared Block entity to
        // GPUI before creating a window that renders through that entity.
        cx.defer(move |cx| {
            if !cx
                .windows()
                .iter()
                .any(|window| window.window_id() == parent.window_id())
            {
                return;
            }
            let pending = owner.upgrade().is_some_and(|owner| {
                owner
                    .read(cx)
                    .shell
                    .surfaces
                    .entries
                    .iter()
                    .any(|entry| entry.revision == revision)
            });
            if !pending {
                return;
            }
            let focus = owner
                .upgrade()
                .and_then(|owner| owner.read(cx).surface_input_focus(revision, cx))
                .unwrap_or(focus);
            let dialog_owner = owner.clone();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, dimensions))),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.clone().into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: Some(if cfg!(target_os = "macos") {
                    WindowDecorations::Client
                } else {
                    WindowDecorations::Server
                }),
                kind: WindowKind::Floating,
                is_resizable: false,
                is_minimizable: false,
                app_id: Some("one.ztd.block".into()),
                ..Default::default()
            };
            match cx.open_window(options, move |window, cx| {
                focus.focus(window, cx);
                let child = window.window_handle();
                let _ = dialog_owner.update(cx, |view, _| {
                    if let Some(entry) = view
                        .shell
                        .surfaces
                        .entries
                        .iter_mut()
                        .find(|entry| entry.revision == revision)
                    {
                        entry.window = Some(child);
                    }
                });
                let observer_owner = dialog_owner.clone();
                let parent_subscription = cx.on_window_closed(move |cx, closed| {
                    if closed == parent.window_id() {
                        let _ = child.update(cx, |_, window, _| window.remove_window());
                    } else if closed == child.window_id() {
                        let fallback = cx.windows().into_iter().find(|window| {
                            window.window_id() != child.window_id()
                                && window.window_id() != parent.window_id()
                        });
                        let target = if cx
                            .windows()
                            .iter()
                            .any(|window| window.window_id() == parent.window_id())
                        {
                            Some(parent)
                        } else {
                            fallback
                        };
                        if let Some(target) = target {
                            let _ = target.update(cx, |_, parent_window, cx| {
                                let _ = observer_owner.update(cx, |view, cx| {
                                    view.close_surface_window(revision, parent_window, cx)
                                });
                            });
                        }
                    }
                });
                let dialog = cx.new(|cx| {
                    if let Some(owner) = dialog_owner.upgrade() {
                        cx.observe(&owner, |_, _, cx| cx.notify()).detach();
                    }
                    DialogWindow {
                        owner: dialog_owner,
                        revision,
                        _parent_subscription: parent_subscription,
                    }
                });
                cx.new(|cx| gpui_base::Root::new(dialog, window, cx))
            }) {
                Ok(handle) => {
                    let handle: AnyWindowHandle = handle.into();
                    let initial_focus = owner
                        .update(cx, |view, cx| {
                            view.shell
                                .surfaces
                                .current_window
                                .set(Some(handle.window_id()));
                            let entry = view
                                .shell
                                .surfaces
                                .entries
                                .iter()
                                .find(|entry| entry.revision == revision)?;
                            let focus = match &entry.kind {
                                Surface::Drawer(Drawer::Accounts | Drawer::Profiles) => view
                                    .shell
                                    .selector
                                    .as_ref()
                                    .map(|state| state.query_focus(cx)),
                                Surface::OnboardingSection(super::WizardPage::Version)
                                | Surface::WizardSection(super::WizardPage::Version) => view
                                    .shell
                                    .wizard
                                    .version_search
                                    .as_ref()
                                    .map(|input| input.read(cx).focus_handle(cx)),
                                Surface::OnboardingSection(super::WizardPage::Loader)
                                | Surface::WizardSection(super::WizardPage::Loader) => view
                                    .shell
                                    .wizard
                                    .loader_search
                                    .as_ref()
                                    .map(|input| input.read(cx).focus_handle(cx)),
                                Surface::OnboardingSection(super::WizardPage::Location)
                                | Surface::WizardSection(super::WizardPage::Location) => {
                                    match &view.form {
                                        Some(super::Form::Instance(form)) => {
                                            Some(form.directory.read(cx).focus_handle(cx))
                                        }
                                        _ => None,
                                    }
                                }
                                _ => None,
                            }
                            .unwrap_or_else(|| entry.focus.clone());
                            cx.notify();
                            Some(focus)
                        })
                        .ok()
                        .flatten();
                    if let Some(focus) = initial_focus {
                        let _ = handle.update(cx, |_, window, cx| {
                            focus.focus(window, cx);
                            window.refresh();
                        });
                    }
                }
                Err(error) => {
                    let _ = owner.update(cx, |view, cx| {
                        view.shell
                            .surfaces
                            .entries
                            .retain(|entry| entry.revision != revision);
                        view.report(format!("Could not open {title}: {error:#}"));
                        cx.notify();
                    });
                }
            }
        });
        true
    }
    /// Present an overlay surface through the component library's dialog
    /// layer. The logical entry remains the stack owner; the library owns the
    /// scrim, Escape, focus trap, focus restore and motion.
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
        window.open_dialog(cx, move |dialog, dialog_window, dialog_cx| {
            let Some(view) = owner.upgrade() else {
                return dialog;
            };
            view.update(dialog_cx, |view, _| {
                view.shell.surfaces.activate(dialog_window);
            });
            let title = view.read(dialog_cx).surface_dialog_title(&kind);
            let body = body.clone();
            let gate = owner.clone();
            let dismiss = owner.clone();
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
                .content(move |content, _window, _cx| content.child(body.clone()))
                .on_cancel(move |_, _, cx| {
                    gate.upgrade().is_none_or(|view| {
                        let view = view.read(cx);
                        !(view.busy && !view.can_stop())
                    })
                })
                .on_close(move |_, _, cx| {
                    let _ =
                        dismiss.update(cx, |view, cx| view.dismiss_surface_overlay(revision, cx));
                })
        });
    }

    fn close_surface_dialog(target: AnyWindowHandle, window: &mut Window, cx: &mut App) {
        if target.window_id() == window.window_handle().window_id() {
            window.close_dialog(cx);
        } else {
            let _ = target.update(cx, |_, target_window, cx| target_window.close_dialog(cx));
        }
    }

    fn close_all_surface_dialogs(target: AnyWindowHandle, window: &mut Window, cx: &mut App) {
        if target.window_id() == window.window_handle().window_id() {
            window.close_all_dialogs(cx);
        } else {
            let _ = target.update(cx, |_, target_window, cx| {
                target_window.close_all_dialogs(cx)
            });
        }
    }

    pub(super) fn close(
        &mut self,
        window: &mut Window,
        fallback: &FocusHandle,
        cx: &mut App,
    ) -> Option<Surface> {
        let index = self.current_index()?;
        let entry = self.entries.remove(index);
        self.current_window.set(Some(entry.parent.window_id()));
        if entry.kind.is_overlay() {
            Self::close_surface_dialog(entry.parent, window, cx);
            return Some(entry.kind);
        }
        if let Some(handle) = entry.window {
            if handle.window_id() == window.window_handle().window_id() {
                window.remove_window();
            } else {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            let focus = entry.return_focus.clone();
            if entry.parent.window_id() == window.window_handle().window_id() {
                if let Some(focus) = focus {
                    focus.focus(window, cx);
                }
            } else {
                let _ = entry.parent.update(cx, move |_, parent, cx| {
                    if let Some(focus) = focus {
                        focus.focus(parent, cx);
                    }
                });
            }
        } else {
            entry
                .return_focus
                .as_ref()
                .unwrap_or(fallback)
                .focus(window, cx);
        }
        Some(entry.kind)
    }
    pub(super) fn clear(&mut self, window: &mut Window, fallback: &FocusHandle, cx: &mut App) {
        let mut dialog_parents: Vec<AnyWindowHandle> = Vec::new();
        for entry in self.entries.drain(..) {
            if entry.kind.is_overlay() {
                if !dialog_parents.contains(&entry.parent) {
                    dialog_parents.push(entry.parent);
                }
                continue;
            }
            if let Some(handle) = entry.window {
                if handle.window_id() == window.window_handle().window_id() {
                    window.remove_window();
                } else {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }
        }
        for parent in dialog_parents {
            Self::close_all_surface_dialogs(parent, window, cx);
        }
        fallback.focus(window, cx);
    }
}

/// A logical overlay surface rendered inside the component library's dialog
/// layer. It follows the shared Block entity, so its note and buttons always
/// describe the current catalog and operation state instead of a stale copy.
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

struct DialogWindow {
    owner: WeakEntity<Block>,
    revision: usize,
    _parent_subscription: Subscription,
}
impl Render for DialogWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let revision = self.revision;
        self.owner
            .update(cx, |view, cx| {
                let previous_window = view.shell.surfaces.enter_window(window);
                let Some(entry) = view
                    .shell
                    .surfaces
                    .entries
                    .iter()
                    .find(|entry| entry.revision == revision)
                    .cloned()
                else {
                    window.remove_window();
                    return div().into_any_element();
                };
                let content = view
                    .render_surface_window(&entry, window, cx)
                    .into_any_element();
                view.shell.surfaces.restore_window(previous_window);
                content
            })
            .unwrap_or_else(|_| {
                window.remove_window();
                div().into_any_element()
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
            Surface::OnboardingSection(super::WizardPage::Version)
            | Surface::WizardSection(super::WizardPage::Version) => self
                .shell
                .wizard
                .version_search
                .as_ref()
                .map(|input| input.read(cx).focus_handle(cx)),
            Surface::OnboardingSection(super::WizardPage::Loader)
            | Surface::WizardSection(super::WizardPage::Loader) => self
                .shell
                .wizard
                .loader_search
                .as_ref()
                .map(|input| input.read(cx).focus_handle(cx)),
            Surface::OnboardingSection(super::WizardPage::Location)
            | Surface::WizardSection(super::WizardPage::Location) => match &self.form {
                Some(super::Form::Instance(form)) => Some(form.directory.read(cx).focus_handle(cx)),
                _ => None,
            },
            _ => None,
        }
    }

    fn close_surface_window(
        &mut self,
        revision: usize,
        parent: &mut Window,
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
        self.shell
            .surfaces
            .current_window
            .set(Some(entry.parent.window_id()));
        match entry.kind {
            Surface::OnboardingSection(_) | Surface::WizardSection(_) => {
                self.cancel_component_draft(cx)
            }
            Surface::Auth => self.cancel_auth(parent, cx),
            Surface::AuthProviderAdd => self.cancel_auth_provider(cx),
            Surface::GuideInformation(super::Tab::Settings) => {
                self.preferences.clone_from(&self.saved_preferences);
                self.settings_form = None;
            }
            _ => {}
        }
        if let Some(focus) = entry.return_focus {
            focus.focus(parent, cx);
        }
        self.motion_dirty = true;
        cx.notify();
    }

    /// The library dismissed a dialog itself (Escape, backdrop or close
    /// button). Drop exactly that logical layer; a late dismissal for an older
    /// revision is a no-op and never closes a newer layer.
    fn dismiss_surface_overlay(&mut self, revision: usize, cx: &mut Context<'_, Self>) {
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
        self.shell
            .surfaces
            .current_window
            .set(Some(entry.parent.window_id()));
        self.motion_dirty = true;
        cx.notify();
    }
}

impl Surface {
    pub(super) fn purpose(&self) -> String {
        match self {
            Self::Drawer(drawer) => format!("drawer:{drawer:?}"),
            Self::GuideInformation(tab) => format!("information:{tab:?}"),
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
            Self::DownloadFiles(group) => group.label(),
            Self::DeleteProfile(_) => "Delete profile",
            Self::RemoveAccount(_) => "Remove account",
            Self::RemoveProvider(_) => "Remove provider",
            Self::SeparateProfiles => "Enable separated profiles",
            Self::StorageCleanup(_) => "Remove retained storage copy",
            Self::GamesHelp => "Instances and Profiles",
            Self::GuideInformation(super::Tab::Settings) => "Settings",
            Self::GuideInformation(_) => "About Block",
            Self::OnboardingSection(page) | Self::WizardSection(page) => match page {
                super::WizardPage::Version => "Minecraft version",
                super::WizardPage::Loader => "Mod loader",
                super::WizardPage::OptiFine => "OptiFine",
                super::WizardPage::Location => "Game configuration",
            },
            Self::Auth => "Add account",
            Self::AuthProviderAdd => "Add authentication provider",
            Self::Select(state) => &state.title,
        }
    }
    /// Surfaces that the component library renders as an in-window dialog
    /// instead of a separate OS window.
    pub(super) const fn is_overlay(&self) -> bool {
        matches!(
            self,
            Self::DeleteProfile(_)
                | Self::RemoveAccount(_)
                | Self::RemoveProvider(_)
                | Self::SeparateProfiles
                | Self::StorageCleanup(_)
        )
    }
    fn window_size(&self) -> gpui::Size<gpui::Pixels> {
        let (width, height) = match self {
            Self::GuideInformation(_) => (960., 600.),
            Self::OnboardingSection(_) | Self::WizardSection(_) => (760., 520.),
            Self::Auth => (960., 600.),
            Self::AuthProviderAdd => (460., 520.),
            Self::Drawer(Drawer::Downloads) | Self::DownloadFiles(_) => (600., 460.),
            Self::StorageCleanup(_) => (560., 460.),
            Self::Drawer(_) => (432., 600.),
            _ => (460., 300.),
        };
        size(px(width), px(height))
    }
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod overlay_tests;
