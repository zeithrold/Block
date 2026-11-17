//! Resolve the title's actual CJK glyph faces through Linux's native text system.

use super::{DrawerFrame, DrawerLayer};
use crate::{Language, Palette, design, shell::Drawer, shell::Surface};
use gpui::{
    AppContext as _, Edges, Font, FontWeight, IntoElement as _, Refineable as _, Styled as _,
    TextRun, TextStyle, TextSystem, WindowTextSystem, div, font, px,
};
use std::{borrow::Cow, sync::Arc};

#[gpui::test]
#[expect(
    clippy::expect_used,
    reason = "The fixture must load product fonts and the title must specify its weight."
)]
fn all_drawer_titles_keep_latin_and_use_available_sans_cjk(cx: &mut gpui::TestAppContext) {
    let platform = gpui_platform::current_platform(true);
    let text_system = Arc::new(TextSystem::new(platform.text_system()));
    text_system
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
            Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
            Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
        ])
        .expect("the production faces load in the native text system");
    let shaper = WindowTextSystem::new(text_system.clone());
    let (owner, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        cx.set_global(design::Typography::new(&text_system));
        let layer = cx.new(|cx| DrawerLayer::new(window, cx));
        let families = text_system.all_font_names();
        for drawer in [
            Drawer::ImportGames,
            Drawer::Language,
            Drawer::Instances,
            Drawer::Accounts,
            Drawer::Downloads,
        ] {
            for (language, candidates, sample) in [
                (
                    Language::English,
                    ["Noto Sans CJK SC", "Noto Sans CJK JP"],
                    "\u{8bed}\u{8a00}",
                ),
                (
                    Language::SimplifiedChinese,
                    ["Noto Sans CJK SC", "Noto Sans CJK JP"],
                    "\u{8bed}\u{8a00}",
                ),
                (
                    Language::Japanese,
                    ["Noto Sans CJK JP", "Noto Sans CJK SC"],
                    "\u{8a00}\u{8a9e}",
                ),
            ] {
                let frame = DrawerFrame {
                    revision: 0,
                    kind: Surface::Drawer(drawer),
                    owner: owner.downgrade(),
                    palette: Palette::LIGHT,
                    language,
                    title: format!("Block {sample}"),
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
                let mut title = frame.title_row();
                let mut sheet = layer.update(cx, |layer, cx| {
                    layer.sheet(&frame, div().into_any_element(), Vec::new(), cx)
                });
                let mut style = TextStyle::default();
                style.refine(sheet.text_style());
                style.refine(title.text_style());
                assert_eq!(style.font_weight, FontWeight::SEMIBOLD);
                assert_eq!(style.font_family.as_ref(), design::FONT_FAMILY);
                for weight in [
                    FontWeight::NORMAL,
                    FontWeight::MEDIUM,
                    FontWeight::SEMIBOLD,
                    FontWeight::BOLD,
                ] {
                let mut title_font = style.font();
                title_font.weight = weight;
                let primary = text_system.resolve_font(&title_font);
                let text = frame.title.clone();
                let shaped = shaper.shape_line(
                    text.clone().into(),
                    px(18.),
                    &[TextRun {
                        len: text.len(),
                        font: title_font.clone(),
                        ..TextRun::default()
                    }],
                    None,
                );
                assert!(
                    shaped
                        .runs
                        .iter()
                        .filter(|run| run.glyphs.iter().any(|glyph| glyph.index < 5))
                        .all(|run| run.font_id == primary),
                    "{language:?} Latin glyphs retain primary {primary:?}: {:?}",
                    shaped.runs
                );
                let available = candidates
                    .into_iter()
                    .find(|family| families.iter().any(|name| name == family));
                if let Some(family) = available {
                    let sans_ids = [
                        FontWeight::NORMAL,
                        FontWeight::MEDIUM,
                        FontWeight::SEMIBOLD,
                        FontWeight::BOLD,
                    ]
                    .map(|weight| {
                        text_system.resolve_font(&Font {
                            weight,
                            ..font(family)
                        })
                    });
                    let cjk = shaped
                        .runs
                        .iter()
                        .filter(|run| run.glyphs.iter().any(|glyph| glyph.index >= 6))
                        .collect::<Vec<_>>();
                    assert!(!cjk.is_empty(), "{language:?} title must shape CJK glyphs");
                    assert!(
                        cjk.iter().all(|run| {
                            sans_ids.contains(&run.font_id)
                                && run.glyphs.iter().all(|glyph| glyph.id.0 != 0)
                        }),
                        "{drawer:?} {language:?} title must use an available sans face, got {cjk:?}"
                    );
                    println!("NATIVE_TITLE_SANS {drawer:?} {language:?} {weight:?}: {family}");
                } else {
                    assert_eq!(
                        title_font.fallbacks, None,
                        "an unavailable family must not replace platform fallback"
                    );
                }
                }
            }
        }
    });
}
