//! Product navigation choices for the in-window Kit menu.
use super::{Block, Tab, UiAction, design::Icon, shell::Surface};
use gpui::{Bounds, Context, DismissEvent, Pixels, Window, point, px};
use gpui_component::menu::{PopupMenu, PopupMenuItem};

impl Block {
    fn app_menu_builder(
        &self,
        cx: &Context<'_, Self>,
    ) -> impl FnOnce(PopupMenu, &mut Window, &mut Context<'_, PopupMenu>) -> PopupMenu + use<> {
        let owner = cx.weak_entity();
        let current = self.router.tab();
        let language = self.language();
        move |mut menu, _, cx| {
            let id = cx.entity_id();
            for (tab, label, icon) in [
                (Tab::Home, "menu.home", Icon::Home),
                (Tab::Accounts, "menu.accounts", Icon::Account),
                (Tab::Instances, "menu.games", Icon::Games),
                (Tab::Settings, "menu.settings", Icon::Settings),
                (Tab::About, "menu.about", Icon::Info),
            ] {
                if current == Tab::Onboarding && !matches!(tab, Tab::Settings | Tab::About) {
                    continue;
                }
                if tab == Tab::About {
                    menu = menu.separator();
                }
                let owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(tr!(language, label))
                        .icon(gpui_component::Icon::default().path(icon.path()))
                        .checked(tab == current)
                        .on_click(move |_, window, cx| {
                            _ = owner.update(cx, |view, cx| {
                                if view.shell.surfaces.top().is_some_and(|layer| {
                                    matches!(&layer.kind, Surface::AppMenu { menu, .. } if menu.entity_id() == id)
                                }) {
                                    view.handle(UiAction::SelectValue(Box::new(UiAction::Tab(tab))), window, cx);
                                }
                            });
                        }),
                );
            }
            menu.min_w(px(192.))
                .max_w(px(192.))
                .check_side(gpui_component::Side::Right)
        }
    }

    pub(super) fn open_app_menu_at(
        &mut self,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .shell
            .surfaces
            .layers()
            .any(|layer| matches!(layer.kind, Surface::AppMenu { .. }))
        {
            return;
        }
        let builder = self.app_menu_builder(cx);
        let menu = PopupMenu::build(window, cx, builder);
        cx.subscribe_in(
            &menu,
            window,
            |view, dismissed, _: &DismissEvent, window, cx| {
                if view.shell.surfaces.top().is_some_and(|layer| {
                matches!(&layer.kind, Surface::AppMenu { menu, .. } if menu == dismissed)
            }) {
                view.handle(UiAction::Drawer(None), window, cx);
            }
            },
        )
        .detach();
        let anchor = anchor.map_or_else(
            || point(px(16.), px(56.)),
            |bounds| point(bounds.left(), bounds.bottom() + px(6.)),
        );
        self.shell
            .surfaces
            .open(Surface::AppMenu { menu, anchor }, window, cx);
    }

    pub(super) fn refresh_app_menu_language(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(menu) = self
            .shell
            .surfaces
            .layers()
            .find_map(|layer| match &layer.kind {
                Surface::AppMenu { menu, .. } => Some(menu),
                _ => None,
            })
        {
            let builder = self.app_menu_builder(cx);
            menu.update(cx, |menu, cx| menu.rebuild(window, cx, builder));
        }
    }
}
