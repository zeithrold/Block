//! Persistent, inline status messages for product pages.

use gpui::{
    App, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
    rgb,
};

use super::{
    appearance::Palette,
    design::{CONTROL_RADIUS, Icon},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AlertVariant {
    Info,
    Success,
    Warning,
    Error,
}

impl AlertVariant {
    pub(super) fn render(
        self,
        id: &'static str,
        title: Option<SharedString>,
        message: impl Into<SharedString>,
        palette: Palette,
    ) -> Stateful<Div> {
        let (icon, foreground, background, role) = match self {
            Self::Info => (
                Icon::Info,
                palette.info,
                palette.info_background,
                Role::Status,
            ),
            Self::Success => (
                Icon::Check,
                palette.success,
                palette.success_background,
                Role::Status,
            ),
            Self::Warning => (
                Icon::Warning,
                palette.warning,
                palette.warning_background,
                Role::Alert,
            ),
            Self::Error => (
                Icon::Error,
                palette.on_destructive_background,
                palette.destructive_background,
                Role::Alert,
            ),
        };
        let message = message.into();
        let title = title.filter(|text| !text.is_empty());

        div()
            .id(id)
            .role(role)
            .w_full()
            .min_w_0()
            .flex()
            .items_start()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(CONTROL_RADIUS))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(13.))
            .line_height(px(22.))
            .child(AlertIcon {
                icon,
                foreground,
                first_text: title.as_ref().unwrap_or(&message).clone(),
                has_title: title.is_some(),
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .whitespace_normal()
                    .children(title.map(|text| div().font_weight(FontWeight::SEMIBOLD).child(text)))
                    .child(div().child(message)),
            )
    }
}

#[derive(IntoElement)]
struct AlertIcon {
    icon: Icon,
    foreground: u32,
    first_text: SharedString,
    has_title: bool,
}

impl RenderOnce for AlertIcon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        const SIZE: f32 = 16.;
        let mut style = window.text_style();
        if self.has_title {
            style.font_weight = FontWeight::SEMIBOLD;
        }
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height_in_pixels(window.rem_size());
        let text = self.first_text.lines().next().unwrap_or_default();
        let fonts = window.text_system();
        let layout = fonts.layout_line(text, font_size, &[style.to_run(text.len())], None);
        // Shaping includes fallback faces, which can move the baseline within a line.
        let baseline = (line_height + layout.ascent - layout.descent) / 2.;
        let cap = fonts.cap_height(fonts.resolve_font(&style.font()), font_size);
        let center = if cap > px(0.) && !layout.runs.is_empty() {
            baseline - cap / 2.
        } else {
            line_height / 2.
        };
        let top = (center - px(SIZE) / 2.)
            .round()
            .clamp(px(0.), (line_height - px(SIZE)).max(px(0.)));
        div()
            .flex_shrink_0()
            .pt(top)
            .child(self.icon.render(SIZE, self.foreground))
    }
}
