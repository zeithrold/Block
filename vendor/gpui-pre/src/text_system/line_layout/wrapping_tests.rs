use super::{tests::glyph_at, tests::make_layout, *};

fn layout(text: &str) -> LineLayout {
    let mut x = 0.;
    let glyphs = text
        .char_indices()
        .map(|(index, _)| {
            let glyph = glyph_at(x, index);
            x += 10.;
            glyph
        })
        .collect();
    LineLayout {
        width: px(x),
        len: text.len(),
        ..make_layout(glyphs)
    }
}

fn offsets(layout: &LineLayout, text: &str, width: f32) -> Vec<usize> {
    layout
        .compute_wrap_boundaries(text, px(width), None)
        .iter()
        .map(|boundary| layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index)
        .collect()
}

#[test]
fn cjk_closing_punctuation_stays_with_its_predecessor() {
    for (text, expected) in [
        ("甲乙。", vec![3]),
        ("甲乙、丙", vec![3, 9]),
        ("甲「乙」丙", vec![3, 12]),
        ("甲乙ぁ丙", vec![3, 9]),
        ("甲乙!丙", vec![3, 7]),
    ] {
        assert_eq!(offsets(&layout(text), text, 20.), expected, "{text}");
    }
}

#[test]
fn protected_units_overflow_instead_of_splitting() {
    for text in [
        "乙。",
        "「乙」",
        "「 乙」",
        "a\u{a0}b",
        "a\u{a0}\u{301}b",
        "a\u{2060}b",
        "a\u{301}",
        "👩‍💻",
        "🇯🇵",
    ] {
        assert_eq!(
            offsets(&layout(text), text, 5.),
            Vec::<usize>::new(),
            "{text}"
        );
    }
}

#[test]
fn emergency_wrap_keeps_long_words_and_paths_usable() {
    for text in [
        "abcdefgh",
        "/abcdefgh/ijklmnop",
        "https://example.com/abcdefgh",
    ] {
        let layout = layout(text);
        let boundaries = offsets(&layout, text, 40.);
        assert!(!boundaries.is_empty(), "{text}");
        for (&start, &end) in std::iter::once(&0)
            .chain(boundaries.iter())
            .zip(boundaries.iter().chain(std::iter::once(&text.len())))
        {
            assert!(
                layout.x_for_index(end) - layout.x_for_index(start) <= px(40.),
                "{text}: {start}..{end}"
            );
        }
    }
}

#[test]
fn fallback_runs_and_duplicate_cluster_indices_do_not_add_breaks() {
    let text = "甲乙。丙";
    let layout = LineLayout {
        width: px(40.),
        len: text.len(),
        runs: vec![
            ShapedRun {
                font_id: FontId(0),
                glyphs: vec![glyph_at(0., 0), glyph_at(10., 3)],
            },
            ShapedRun {
                font_id: FontId(1),
                glyphs: vec![glyph_at(14., 3), glyph_at(20., 6), glyph_at(30., 9)],
            },
        ],
        ..LineLayout::default()
    };
    assert_eq!(offsets(&layout, text, 20.), vec![3, 9]);
    assert_eq!(
        layout
            .compute_wrap_boundaries(text, px(20.), Some(2))
            .as_slice(),
        &[WrapBoundary {
            run_ix: 0,
            glyph_ix: 1
        }]
    );
    assert!(
        layout
            .compute_wrap_boundaries(text, px(20.), Some(1))
            .is_empty()
    );
}

#[test]
fn product_notes_preserve_bytes_and_punctuation_through_width_sweep() {
    for text in [
        "不会移动或复制文件。原有存档、模组和配置仍保留在原处。",
        "ファイルの移動やコピーは行いません。既存のワールド、MOD、設定は元の場所に残ります。",
    ] {
        for width in [10., 19., 20., 21., 30., 50., 100., 316.] {
            let layout = layout(text);
            let boundaries = offsets(&layout, text, width);
            let rows = std::iter::once(0)
                .chain(boundaries.iter().copied())
                .zip(
                    boundaries
                        .iter()
                        .copied()
                        .chain(std::iter::once(text.len())),
                )
                .map(|(start, end)| &text[start..end])
                .collect::<Vec<_>>();
            assert_eq!(rows.concat(), text);
            assert!(
                rows.iter()
                    .all(|row| !row.starts_with(['。', '、', 'ァ', 'ィ', 'ー'])),
                "{width}: {rows:?}"
            );
        }
    }
}
