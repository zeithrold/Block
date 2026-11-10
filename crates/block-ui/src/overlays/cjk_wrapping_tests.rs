//! Shape the unchanged policy note with the native font system, without a desktop window.

use super::{DrawerFrame, DrawerLayer};
use crate::{Language, Palette, shell::Drawer, shell::Surface};
use gpui::{
    AppContext as _, AvailableSpace, Edges, IntoElement as _, Refineable as _, Styled as _,
    TextRun, TextStyle, TextSystem, WindowTextSystem, div, point, px, size,
};
use std::{borrow::Cow, sync::Arc};
use unicode_segmentation::UnicodeSegmentation as _;

#[gpui::test]
#[expect(
    clippy::expect_used,
    reason = "Native shaping and bundled font loading must succeed for this regression."
)]
fn policy_notes_keep_cjk_punctuation_with_native_shaping(cx: &mut gpui::TestAppContext) {
    let platform = gpui_platform::current_platform(true);
    let text_system = Arc::new(TextSystem::new(platform.text_system()));
    text_system
        .add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../../assets/fonts/Inter-Regular.ttf"
        ))])
        .expect("the production regular face loads");
    let shaper = WindowTextSystem::new(text_system.clone());
    let (owner, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.draw(
        point(px(0.), px(0.)),
        size(px(360.), px(720.)),
        |window, cx| {
            let layer = cx.new(|cx| DrawerLayer::new(window, cx));
            for language in [Language::SimplifiedChinese, Language::Japanese] {
                let frame = DrawerFrame {
                    revision: 0,
                    kind: Surface::Drawer(Drawer::ImportGames),
                    owner: owner.downgrade(),
                    palette: Palette::LIGHT,
                    language,
                    title: String::new(),
                    description: String::new(),
                    focus: cx.focus_handle(),
                    insets: Edges::all(px(0.)),
                    width: px(360.),
                    height: px(720.),
                    panel_height: 652.,
                    translation: 0.,
                    scrim: 1.,
                    dragging: false,
                    closing: false,
                    #[cfg(debug_assertions)]
                    trace: None,
                };
                let text = tr!(language, "import.policy.files_remain");
                let mut note = crate::note(text.clone());
                let mut sheet = layer.update(cx, |layer, cx| {
                    layer.sheet(&frame, div().into_any_element(), Vec::new(), cx)
                });
                let mut style = TextStyle::default();
                style.refine(sheet.text_style());
                style.refine(note.text_style());
                let font_size = style.font_size.to_pixels(window.rem_size());
                let runs = [TextRun {
                    len: text.len(),
                    font: style.font(),
                    ..TextRun::default()
                }];
                let unwrapped = shaper.shape_line(text.clone().into(), font_size, &runs, None);
                // Measure the actual note element at the compact sheet's body width.
                let measured = note.into_any_element().layout_as_root(
                    size(
                        AvailableSpace::Definite(frame.width - px(40.)),
                        AvailableSpace::MinContent,
                    ),
                    window,
                    cx,
                );
                let mut widths = vec![measured.width];
                widths.extend((20_u16..=340).step_by(3).map(|width| px(f32::from(width))));
                for (index, ch) in text.char_indices() {
                    if matches!(ch, '。' | '、') {
                        let x = unwrapped.x_for_index(index);
                        widths.extend([x - px(0.5), x, x + px(0.5)]);
                    }
                }
                let graphemes = text
                    .grapheme_indices(true)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                for width in widths {
                    let shaped = shaper
                        .shape_text(text.clone().into(), font_size, &runs, Some(width), None)
                        .expect("native note shaping succeeds");
                    assert_eq!(shaped.len(), 1);
                    let line = &shaped[0];
                    assert_eq!(line.text.as_ref(), text.as_str());
                    let offsets = line
                        .wrap_boundaries
                        .iter()
                        .map(|boundary| {
                            line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix]
                                .index
                        })
                        .collect::<Vec<_>>();
                    let rows = std::iter::once(0)
                        .chain(offsets.iter().copied())
                        .zip(offsets.iter().copied().chain(std::iter::once(text.len())))
                        .map(|(start, end)| &text[start..end])
                        .collect::<Vec<_>>();
                    assert_eq!(rows.concat(), text);
                    assert!(offsets.iter().all(|index| graphemes.contains(index)));
                    assert!(
                        rows.iter()
                            .all(|row| !row.starts_with(['。', '、', 'ァ', 'ィ', 'ー'])),
                        "{language:?}, {width:?}: {rows:?}"
                    );
                }
                println!(
                    "NATIVE_CJK_NOTE {language:?}: font_size={font_size:?}, measured_width={:?}",
                    measured.width
                );
            }
            div()
        },
    );
}
