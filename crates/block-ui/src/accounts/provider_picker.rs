//! The Authlib sign-in form's server selector.

use block_auth::AuthlibProvider;
use gpui::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div, px, rgb,
};
use gpui_component::{
    IndexPath, Size,
    searchable_list::{SearchableListItem, SearchableVec},
    select::{Caret, Select, SelectEvent, SelectState},
};
use uuid::Uuid;

use super::{AuthWizard, Block};
use crate::{Palette, components, design};

const PROVIDER_CONTROL_HEIGHT: f32 = 56.;
// Select measures the first server row and reserves that height for every row.
const PROVIDER_ROW_CONTENT_HEIGHT: f32 = 42.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderValue {
    Server(Uuid),
    Add,
}

#[derive(Clone)]
pub(super) enum ProviderChoice {
    Server(ServerChoice),
    Add(SharedString),
}

#[derive(Clone)]
pub(super) struct ServerChoice {
    value: ProviderValue,
    name: SharedString,
    url: SharedString,
    label: SharedString,
    icon: Option<components::ProviderBadge>,
}

#[derive(IntoElement)]
struct ProviderChoiceContent(ServerChoice);

impl RenderOnce for ProviderChoiceContent {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let palette = cx
            .try_global::<Palette>()
            .copied()
            .unwrap_or(Palette::LIGHT);
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .w_full()
            .min_h(px(PROVIDER_ROW_CONTENT_HEIGHT))
            .child(components::provider_icon(
                &self.0.name,
                self.0.icon.as_ref(),
                palette,
                26.,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(self.0.name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(self.0.url),
                    ),
            )
    }
}

impl SearchableListItem for ProviderChoice {
    type Value = ProviderValue;

    fn title(&self) -> SharedString {
        match self {
            Self::Server(server) => server.label.clone(),
            Self::Add(label) => label.clone(),
        }
    }

    fn display_title(&self) -> Option<gpui::AnyElement> {
        match self {
            Self::Server(server) => Some(ProviderChoiceContent(server.clone()).into_any_element()),
            Self::Add(_) => None,
        }
    }

    fn render(&self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        match self {
            Self::Server(server) => ProviderChoiceContent(server.clone()).into_any_element(),
            Self::Add(label) => {
                let palette = cx
                    .try_global::<Palette>()
                    .copied()
                    .unwrap_or(Palette::LIGHT);
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_h(px(PROVIDER_ROW_CONTENT_HEIGHT))
                    .child(
                        div()
                            .w(px(26.))
                            .flex()
                            .justify_center()
                            .child(design::Icon::Plus.render(16., palette.foreground)),
                    )
                    .child(label.clone())
                    .into_any_element()
            }
        }
    }

    fn value(&self) -> &ProviderValue {
        match self {
            Self::Server(server) => &server.value,
            Self::Add(_) => &ProviderValue::Add,
        }
    }
}

pub(super) type ProviderPicker = Entity<SelectState<SearchableVec<ProviderChoice>>>;

impl Block {
    fn auth_provider_choice(&self, provider: &AuthlibProvider) -> ServerChoice {
        ServerChoice {
            value: ProviderValue::Server(provider.id),
            name: provider.name.clone().into(),
            url: provider.api_url.clone().into(),
            label: format!("{} · {}", provider.name, provider.api_url).into(),
            icon: self.provider_icons.get(&provider.id.to_string()).cloned(),
        }
    }

    fn auth_provider_choices(&self) -> Vec<ProviderChoice> {
        let mut choices: Vec<_> = self
            .shell
            .providers
            .records
            .iter()
            .map(|provider| ProviderChoice::Server(self.auth_provider_choice(provider)))
            .collect();
        if !choices.is_empty() {
            choices.push(ProviderChoice::Add(
                tr!(self.language(), "accounts.auth.provider.picker.add").into(),
            ));
        }
        choices
    }

    pub(super) fn new_auth_provider_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ProviderPicker {
        let items = self.auth_provider_choices();
        let selected = (!items.is_empty()).then(|| IndexPath::new(0));
        let picker = cx.new(|cx| {
            SelectState::new(SearchableVec::from(items), selected, window, cx).searchable(true)
        });
        cx.subscribe_in(
            &picker,
            window,
            |view, picker, event: &SelectEvent<SearchableVec<ProviderChoice>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event
                    && let Some(state) = view.shell.auth.as_ref().filter(|state| {
                        state.provider_picker.as_ref() == Some(picker)
                            && state.provider_draft.is_none()
                    })
                {
                    match *value {
                        ProviderValue::Server(id) => view.choose_external_provider(id, window, cx),
                        ProviderValue::Add => {
                            // The Select confirms every row; restore its server value before opening the drawer.
                            state.sync_auth_provider_selection(window, cx);
                            view.open_auth_provider_add(window, cx);
                        }
                    }
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
                        let items = SearchableVec::from(view.auth_provider_choices());
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
    ) -> impl IntoElement + use<> {
        let language = self.language();
        let palette = self.palette;
        let label = tr!(language, "select.auth_provider");
        let placeholder = tr!(language, "accounts.auth.provider.picker.empty");
        let disabled = self.busy || state.provider_draft.is_some();
        div()
            .w_full()
            .min_w_0()
            .h(px(PROVIDER_CONTROL_HEIGHT))
            .child(if let Some(picker) = &state.provider_picker {
                Select::new(picker)
                    .id("auth-provider-picker-select")
                    .accessibility_label(label)
                    .placeholder(placeholder)
                    .disabled(disabled)
                    .w_full()
                    .h(px(PROVIDER_CONTROL_HEIGHT))
                    .py_2()
                    .rounded(px(design::CONTROL_RADIUS))
                    .border_color(rgb(palette.input))
                    .bg(rgb(palette.background))
                    .text_color(rgb(palette.foreground))
                    .text_size(px(design::BODY_FONT_SIZE))
                    .into_any_element()
            } else {
                div()
                    .size_full()
                    .px(Size::default().input_px())
                    .py_2()
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
                    .child(div().flex_1().min_w_0().child(provider.map_or_else(
                        || placeholder.into_any_element(),
                        |provider| {
                            ProviderChoiceContent(self.auth_provider_choice(provider))
                                .into_any_element()
                        },
                    )))
                    .child(Caret::new(Size::default()).text_color(rgb(palette.muted).into()))
                    .into_any_element()
            })
            .debug_selector(|| "auth-provider-picker".into())
    }
}

impl AuthWizard {
    pub(crate) fn sync_auth_provider_selection(&self, window: &mut Window, cx: &mut gpui::App) {
        if let Some(picker) = &self.provider_picker
            && picker.read(cx).selected_value().copied()
                != self.external_provider.map(ProviderValue::Server)
        {
            picker.update(cx, |picker, cx| match self.external_provider {
                Some(id) => picker.set_selected_value(&ProviderValue::Server(id), window, cx),
                None => picker.set_selected_index(None, window, cx),
            });
        }
    }
}
