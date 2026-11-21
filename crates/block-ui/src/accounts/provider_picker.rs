//! The Authlib sign-in form's server selector.

use block_auth::AuthlibProvider;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Window, div, px, rgb,
};
use gpui_component::{
    IndexPath, Size,
    searchable_list::SearchableListItem,
    select::{Caret, Select, SelectEvent, SelectState},
};
use uuid::Uuid;

use super::{AuthWizard, Block};
use crate::design;

#[derive(Clone)]
pub(super) struct ProviderChoice {
    id: Uuid,
    name: SharedString,
    label: SharedString,
}

impl SearchableListItem for ProviderChoice {
    type Value = Uuid;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn display_title(&self) -> Option<gpui::AnyElement> {
        Some(self.name.clone().into_any_element())
    }

    fn value(&self) -> &Uuid {
        &self.id
    }
}

pub(super) type ProviderPicker = Entity<SelectState<Vec<ProviderChoice>>>;

fn choices(records: &[AuthlibProvider]) -> Vec<ProviderChoice> {
    records
        .iter()
        .map(|provider| ProviderChoice {
            id: provider.id,
            name: provider.name.clone().into(),
            label: format!("{} · {}", provider.name, provider.api_url).into(),
        })
        .collect()
}

impl Block {
    pub(super) fn new_auth_provider_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ProviderPicker {
        let items = choices(&self.shell.providers.records);
        let selected = (!items.is_empty()).then(|| IndexPath::new(0));
        let picker = cx.new(|cx| SelectState::new(items, selected, window, cx));
        cx.subscribe_in(
            &picker,
            window,
            |view, picker, event: &SelectEvent<Vec<ProviderChoice>>, window, cx| {
                if let SelectEvent::Confirm(Some(id)) = event
                    && view.shell.auth.as_ref().is_some_and(|state| {
                        state.provider_picker.as_ref() == Some(picker)
                            && state.provider_draft.is_none()
                    })
                {
                    view.choose_external_provider(*id, window, cx);
                }
            },
        )
        .detach();
        picker
    }

    pub(crate) fn refresh_auth_provider_picker(&self, cx: &mut Context<'_, Self>) {
        let Some(window) = self
            .shell
            .auth
            .as_ref()
            .and_then(|state| state.return_window)
        else {
            return;
        };
        let owner = cx.weak_entity();
        cx.defer(move |cx| {
            _ = window.update(cx, |_, window, cx| {
                _ = owner.update(cx, |view, cx| {
                    if let Some(state) = &view.shell.auth
                        && let Some(picker) = &state.provider_picker
                    {
                        let items = choices(&view.shell.providers.records);
                        picker.update(cx, |picker, cx| picker.set_items(items, window, cx));
                        state.sync_auth_provider_selection(window, cx);
                    }
                });
            });
        });
    }

    pub(super) fn render_provider_picker(
        &self,
        state: &AuthWizard,
        provider: Option<&AuthlibProvider>,
    ) -> impl gpui::IntoElement + use<> {
        let language = self.language();
        let palette = self.palette;
        let label = tr!(language, "select.auth_provider");
        let placeholder = tr!(language, "accounts.auth.provider.picker.empty");
        let disabled =
            self.busy || state.provider_draft.is_some() || self.shell.providers.records.is_empty();
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child(label.clone()))
            .child(
                div()
                    .w_full()
                    .h(px(design::CONTROL_HEIGHT))
                    .child(if let Some(picker) = &state.provider_picker {
                        Select::new(picker)
                            .id("auth-provider-picker-select")
                            .accessibility_label(label)
                            .placeholder(placeholder)
                            .disabled(disabled)
                            .w_full()
                            .h(px(design::CONTROL_HEIGHT))
                            .rounded(px(design::CONTROL_RADIUS))
                            .border_color(rgb(palette.input))
                            .bg(rgb(palette.background))
                            .text_color(rgb(palette.foreground))
                            .text_size(px(design::BODY_FONT_SIZE))
                            .into_any_element()
                    } else {
                        div()
                            .size_full()
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .rounded(px(design::CONTROL_RADIUS))
                            .border_1()
                            .border_color(rgb(palette.input))
                            .bg(rgb(palette.background))
                            .text_color(rgb(palette.foreground))
                            .text_size(px(design::BODY_FONT_SIZE))
                            .child(div().min_w_0().truncate().child(
                                provider.map_or(placeholder, |provider| provider.name.clone()),
                            ))
                            .child(
                                Caret::new(Size::default()).text_color(rgb(palette.muted).into()),
                            )
                            .into_any_element()
                    })
                    .debug_selector(|| "auth-provider-picker".into()),
            )
    }
}

impl AuthWizard {
    pub(crate) fn sync_auth_provider_selection(&self, window: &mut Window, cx: &mut gpui::App) {
        if let Some(picker) = &self.provider_picker
            && picker.read(cx).selected_value() != self.external_provider.as_ref()
        {
            picker.update(cx, |picker, cx| match self.external_provider {
                Some(id) => picker.set_selected_value(&id, window, cx),
                None => picker.set_selected_index(None, window, cx),
            });
        }
    }
}
