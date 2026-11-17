use super::{Typography, monospace_text, ui_text};
use crate::Language;
use gpui::{FontFeatures, FontStyle, FontWeight, Refineable as _, Styled as _, TextStyle, div};

#[test]
fn sans_fallbacks_use_only_available_families_in_the_views_language_order() {
    for (names, chinese, japanese) in [
        (
            vec!["Noto Sans CJK JP", "Noto Sans CJK SC", "Noto Serif CJK SC"],
            vec!["Noto Sans CJK SC", "Noto Sans CJK JP"],
            vec!["Noto Sans CJK JP", "Noto Sans CJK SC"],
        ),
        (
            vec!["Noto Sans CJK SC"],
            vec!["Noto Sans CJK SC"],
            vec!["Noto Sans CJK SC"],
        ),
        (
            vec!["Noto Sans CJK JP"],
            vec!["Noto Sans CJK JP"],
            vec!["Noto Sans CJK JP"],
        ),
        (
            vec!["Noto Serif CJK SC", "Noto Sans Mono CJK JP"],
            vec![],
            vec![],
        ),
        (vec![], vec![], vec![]),
    ] {
        let names = names.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let typography = Typography::from_names(&names);
        for (language, expected) in [
            (Language::English, &chinese),
            (Language::SimplifiedChinese, &chinese),
            (Language::Japanese, &japanese),
        ] {
            let fallback = typography.fallbacks(language);
            let expected = expected
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                fallback.as_ref().map(|fonts| fonts.fallback_list()),
                (!expected.is_empty()).then_some(expected.as_slice()),
                "{names:?}: {language:?}"
            );
        }
    }
}

#[gpui::test]
fn ui_typography_preserves_text_traits_and_monospace_can_override_its_chain(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        cx.set_global(Typography::from_names(&["Noto Sans CJK SC".into()]));
        let features = FontFeatures::disable_ligatures();
        let mut element = ui_text(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .italic()
                .font_features(features.clone()),
            Language::SimplifiedChinese,
            cx,
        );
        let mut inherited = TextStyle::default();
        inherited.refine(element.text_style());
        assert_eq!(inherited.font_family.as_ref(), "Inter");
        assert_eq!(inherited.font_weight, FontWeight::SEMIBOLD);
        assert_eq!(inherited.font_style, FontStyle::Italic);
        assert_eq!(inherited.font_features, features);
        assert_eq!(
            inherited
                .font_fallbacks
                .as_ref()
                .map(|fonts| fonts.fallback_list()),
            Some(["Noto Sans CJK SC".to_owned()].as_slice())
        );
        let mut mono = monospace_text(div(), "Diagnostic Mono".into());
        inherited.refine(mono.text_style());
        assert_eq!(inherited.font_family.as_ref(), "Diagnostic Mono");
        assert_eq!(inherited.font_weight, FontWeight::SEMIBOLD);
        assert_eq!(inherited.font_style, FontStyle::Italic);
        assert_eq!(inherited.font_features, features);
        assert!(
            inherited
                .font_fallbacks
                .as_ref()
                .is_some_and(|fonts| fonts.fallback_list().is_empty())
        );
    });
}
