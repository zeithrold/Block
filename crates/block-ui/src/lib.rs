//! Root view and presentation for the Block desktop application.

use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, rgb};

/// The application's root view.
#[derive(Debug)]
pub struct Block;

impl Render for Block {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(rgb(0x0018_181b))
            .text_color(rgb(0x00f4_f4f5))
            .child(div().text_3xl().child("Hello, GPUI!"))
            .child(
                div()
                    .text_base()
                    .text_color(rgb(0x00a1_a1aa))
                    .child("Block · Built with Rust and GPUI"),
            )
    }
}
