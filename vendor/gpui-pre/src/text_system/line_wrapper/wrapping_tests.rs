use super::{tests::build_wrapper, tests::generate_test_runs, *};

#[test]
fn approximate_wrap_preserves_cjk_punctuation_across_fragments() {
    let mut wrapper = build_wrapper();
    for (parts, expected) in [
        (vec!["甲乙。"], vec![3]),
        (vec!["甲", "乙", "、", "丙"], vec![3, 9]),
        (vec!["甲", "「乙」", "丙"], vec![3, 12]),
    ] {
        let fragments = parts
            .iter()
            .map(|text| LineFragment::text(text))
            .collect::<Vec<_>>();
        let width = wrapper.width_for_char('甲') * 2.;
        let boundaries = wrapper
            .wrap_line(&fragments, width, IndentAdjustment::NoIndent)
            .map(|boundary| boundary.ix)
            .collect::<Vec<_>>();
        assert_eq!(boundaries, expected, "{parts:?}");
    }
}

#[test]
fn approximate_emergency_wrap_keeps_protected_units_intact() {
    let mut wrapper = build_wrapper();
    for text in [
        "乙。",
        "「乙」",
        "a\u{a0}b",
        "a\u{2060}b",
        "a\u{301}",
        "👩‍💻",
        "🇯🇵",
    ] {
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(text)],
                    px(1.),
                    IndentAdjustment::NoIndent
                )
                .collect::<Vec<_>>(),
            Vec::<Boundary>::new(),
            "{text}"
        );
    }
}

#[test]
fn wrapped_truncation_accounts_for_punctuation_protected_rows() {
    let mut wrapper = build_wrapper();
    let text = "甲乙。丙丁";
    let runs = generate_test_runs(&[text.len()]);
    let width = wrapper.width_for_char('甲') * 2.;
    let (truncated, runs) =
        wrapper.truncate_wrapped_line(text.into(), width, 2, "", &runs, TruncateFrom::End);
    assert_eq!(truncated.as_ref(), "甲乙。");
    assert_eq!(
        runs.iter().map(|run| run.len).sum::<usize>(),
        truncated.len()
    );
}

#[test]
fn wrapped_truncation_never_returns_half_a_grapheme() {
    let mut wrapper = build_wrapper();
    let text = "甲乙丙a\u{301}丁";
    let runs = generate_test_runs(&[text.len()]);
    let width = wrapper.width_for_char('甲') * 2.;
    let (truncated, _) =
        wrapper.truncate_wrapped_line(text.into(), width, 2, "", &runs, TruncateFrom::End);
    assert_eq!(truncated.as_ref(), "甲乙丙");
}

#[test]
fn single_line_truncation_preserves_emoji_clusters_in_each_direction() {
    let mut wrapper = build_wrapper();
    let text = "ab👩‍💻cd";
    let runs = generate_test_runs(&[text.len()]);
    for (direction, expected) in [
        (TruncateFrom::Start, "…cd"),
        (TruncateFrom::End, "ab…"),
        (TruncateFrom::Middle, "ab…d"),
    ] {
        let (truncated, runs) = wrapper.truncate_line(text.into(), px(48.), "…", &runs, direction);
        assert_eq!(truncated.as_ref(), expected);
        assert_eq!(
            runs.iter().map(|run| run.len).sum::<usize>(),
            truncated.len()
        );
    }
}
