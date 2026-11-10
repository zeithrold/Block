//! Unicode 15 normal opportunities with cluster-safe emergency word wrapping.
//!
//! Emergency wrapping is a deliberate tailoring, not dictionary segmentation.
//! Protected punctuation/glue units may overflow rather than break illegally.

use unicode_linebreak::{BreakClass, break_property, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BreakKind {
    Prohibited,
    Normal,
    Emergency,
}

pub(super) struct LineBreaks {
    boundaries: Vec<(usize, BreakKind)>,
}

impl LineBreaks {
    pub(super) fn new(text: &str) -> Self {
        let mut normal = linebreaks(text).peekable();
        let mut previous = None;
        let mut previous_non_space = None;
        let mut has_content = false;
        let mut boundaries = Vec::new();
        for (index, grapheme) in text.grapheme_indices(true) {
            while normal.peek().is_some_and(|(offset, _)| *offset < index) {
                normal.next();
            }
            let Some(first) = grapheme.chars().next() else {
                continue;
            };
            let right = break_property(u32::from(first));
            let protected = previous.is_some_and(glue)
                || glue(right)
                || previous_non_space.is_some_and(|class| {
                    matches!(
                        class,
                        BreakClass::OpenPunctuation
                            | BreakClass::Quotation
                            | BreakClass::WordJoiner
                    )
                });
            let kind = if index == 0 || protected {
                BreakKind::Prohibited
            } else if has_content && normal.peek().is_some_and(|(offset, _)| *offset == index) {
                BreakKind::Normal
            } else if matches!(
                right,
                BreakClass::ClosePunctuation
                    | BreakClass::CloseParenthesis
                    | BreakClass::Exclamation
                    | BreakClass::Inseparable
                    | BreakClass::NonStarter
                    | BreakClass::ConditionalJapaneseStarter
                    | BreakClass::Quotation
                    | BreakClass::InfixSeparator
                    | BreakClass::Symbol
                    | BreakClass::Postfix
                    | BreakClass::Hyphen
                    | BreakClass::CombiningMark
                    | BreakClass::EmojiModifier
            ) || previous.is_some_and(|class| {
                matches!(class, BreakClass::Prefix | BreakClass::ZeroWidthJoiner)
            }) {
                BreakKind::Prohibited
            } else {
                BreakKind::Emergency
            };
            boundaries.push((index, kind));
            for ch in grapheme.chars() {
                let class = break_property(u32::from(ch));
                if class != BreakClass::CombiningMark || previous.is_none() {
                    previous = Some(class);
                }
                if class != BreakClass::Space && class != BreakClass::CombiningMark {
                    previous_non_space = Some(class);
                }
                has_content |= class != BreakClass::Space;
            }
        }
        boundaries.push((text.len(), BreakKind::Normal));
        Self { boundaries }
    }

    pub(super) fn at(&self, index: usize) -> BreakKind {
        self.boundaries
            .binary_search_by_key(&index, |(offset, _)| *offset)
            .map_or(BreakKind::Prohibited, |ix| self.boundaries[ix].1)
    }
}

fn glue(class: BreakClass) -> bool {
    matches!(
        class,
        BreakClass::WordJoiner | BreakClass::NonBreakingGlue | BreakClass::ZeroWidthJoiner
    )
}
