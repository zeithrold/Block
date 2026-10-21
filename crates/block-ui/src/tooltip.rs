//! Use the upstream tooltip lifecycle and the unmodified Base tooltip surface.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Tooltips are internal product controls."
)]

use gpui::{
    AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, rgb,
};

use super::Block;

pub(super) fn trigger<E: gpui::StatefulInteractiveElement>(
    control: E,
    text: impl Into<SharedString>,
    _: &Context<'_, Block>,
) -> E {
    let text = text.into();
    control.tooltip(move |_, cx| cx.new(|_| Tip(text.clone())).into())
}

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let palette = cx
            .try_global::<super::Palette>()
            .copied()
            .unwrap_or(super::Palette::LIGHT);
        gpui_base::Tooltip::new("product-tooltip")
            .max_w(gpui::px(320.))
            .px_3()
            .py(gpui::px(6.))
            .rounded_md()
            .bg(rgb(palette.foreground))
            .text_color(rgb(palette.background))
            .text_xs()
            .child(self.0.clone())
    }
}
