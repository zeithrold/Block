//! Use the upstream tooltip lifecycle and the unmodified Base tooltip surface.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Tooltips are internal product controls."
)]

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, rgb,
};

pub(super) fn trigger<E: gpui::StatefulInteractiveElement, T: 'static>(
    control: E,
    text: impl Into<SharedString>,
    language: fn(&T) -> crate::Language,
    cx: &Context<'_, T>,
) -> E {
    let text = text.into();
    let owner = cx.entity();
    control.tooltip(move |_, cx| {
        cx.new(|_| Tip {
            text: text.clone(),
            owner: owner.clone(),
            language,
        })
        .into()
    })
}

struct Tip<T> {
    text: SharedString,
    owner: Entity<T>,
    language: fn(&T) -> crate::Language,
}

impl<T: 'static> Render for Tip<T> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let palette = cx
            .try_global::<super::Palette>()
            .copied()
            .unwrap_or(super::Palette::LIGHT);
        let language = (self.language)(self.owner.read(cx));
        super::design::ui_text(gpui_base::Tooltip::new("product-tooltip"), language, cx)
            .max_w(gpui::px(320.))
            .px_3()
            .py(gpui::px(6.))
            .rounded_md()
            .bg(rgb(palette.foreground))
            .text_color(rgb(palette.background))
            .text_xs()
            .child(self.text.clone())
    }
}
