//! Shape the unchanged policy note with the native font system, without a desktop window.

use super::{DrawerFrame, DrawerLayer};
use crate::{Language, Palette, design, shell::Drawer, shell::Surface};
use gpui::{
    AnyElement, AppContext as _, Context, Edges, IntoElement as _, ParentElement as _, Render,
    Styled as _, TextRun, TextSystem, Window, WindowTextSystem, canvas, div, point, px,
};
use std::{borrow::Cow, cell::Cell, rc::Rc, sync::Arc};
use unicode_segmentation::UnicodeSegmentation as _;

const FORBIDDEN_ROW_STARTS: [char; 5] = ['。', '、', 'ァ', 'ィ', 'ー'];

struct NoteLayout(Option<AnyElement>);

impl Render for NoteLayout {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl gpui::IntoElement {
        div().children(self.0.take())
    }
}

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
    let drawer_size = crate::window_geometry::preferred_management_size();
    for language in [Language::SimplifiedChinese, Language::Japanese] {
        let text = tr!(language, "import.policy.files_remain");
        let typography = Rc::new(Cell::new(None));
        let probe = typography.clone();
        cx.draw(point(px(0.), px(0.)), drawer_size, |window, cx| {
            cx.set_global(design::Typography::new(&text_system));
            let layer = cx.new(|cx| DrawerLayer::new(window, cx));
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
                width: drawer_size.width,
                height: drawer_size.height,
                panel_height: 652.,
                fit: None,
                translation: 0.,
                scrim: 1.,
                dragging: false,
                closing: false,
                #[cfg(debug_assertions)]
                trace: None,
            };
            // An out-of-flow probe reads the real panel's note width and inherited
            // typography without relying on the test window's stub text metrics.
            let note = crate::note(text.clone()).relative().child(
                canvas(
                    move |bounds, window, _| {
                        let style = window.text_style();
                        probe.set(Some((
                            bounds.size.width,
                            style.font_size.to_pixels(window.rem_size()),
                            style.font(),
                        )));
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            );
            let sheet = layer.update(cx, |layer, cx| {
                let panel = layer.panel(&frame, &owner, note.into_any_element(), cx);
                layer.sheet(&frame, panel, Vec::new(), cx)
            });
            // Interactive sheet elements require a GPUI view during prepaint.
            div().child(cx.new(|_| NoteLayout(Some(sheet.into_any_element()))))
        });
        let (product_width, font_size, font) =
            typography.take().expect("the policy note was laid out");
        let runs = [TextRun {
            len: text.len(),
            font,
            ..TextRun::default()
        }];
        let unwrapped = shaper.shape_line(text.clone().into(), font_size, &runs, None);
        let rows_at = |width| {
            let shaped = shaper
                .shape_text(text.clone().into(), font_size, &runs, Some(width), None)
                .map_err(|error| format!("{language:?}, {width:?}: {error}"))
                .expect("native shaping succeeds");
            assert_eq!(
                shaped.len(),
                1,
                "{language:?}, {width:?}: expected one logical line"
            );
            let line = &shaped[0];
            assert_eq!(line.text.as_ref(), text.as_str(), "{language:?}, {width:?}");
            let offsets = line.wrap_boundaries.iter().map(|boundary| {
                line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
            });
            let graphemes = text.grapheme_indices(true).map(|(index, _)| index);
            let mut rows = Vec::new();
            let mut start = 0;
            for end in offsets.chain(std::iter::once(text.len())) {
                assert!(
                    start < end,
                    "{language:?}, {width:?}: invalid row {start}..{end}"
                );
                assert!(
                    end == text.len() || graphemes.clone().any(|index| index == end),
                    "{language:?}, {width:?}: break at byte {end} splits a grapheme"
                );
                rows.push(&text[start..end]);
                start = end;
            }
            assert_eq!(rows.concat(), text, "{language:?}, {width:?}");
            rows
        };

        // Emergency breaks are allowed across the sweep, but must preserve text.
        let sweep = (20_u16..=340)
            .step_by(3)
            .chain([340])
            .map(|width| px(f32::from(width)));
        let punctuation_edges = text
            .char_indices()
            .filter(|&(_, ch)| FORBIDDEN_ROW_STARTS.contains(&ch))
            .map(|(index, _)| unwrapped.x_for_index(index))
            .flat_map(|x| [x - px(0.5), x, x + px(0.5)]);
        for width in sweep.chain(punctuation_edges) {
            drop(rows_at(width));
        }

        // Check a neighborhood around the measured compact drawer, not an
        // allowlist of widths where the upstream emergency breaker succeeds.
        for width in [
            product_width - px(3.),
            product_width,
            product_width + px(3.),
        ] {
            let rows = rows_at(width);
            assert!(
                rows.iter()
                    .all(|row| !row.starts_with(FORBIDDEN_ROW_STARTS)),
                "{language:?}, {width:?}: forbidden row start in {rows:?}"
            );
        }
        let product_rows = rows_at(product_width);
        assert_eq!(
            product_rows.len(),
            2,
            "{language:?}, {product_width:?}: {product_rows:?}"
        );
        println!("NATIVE_CJK_NOTE {language:?}: width={product_width:?}, rows={product_rows:?}");
    }
}
