use super::{LineLayout, WrapBoundary};
use crate::{Pixels, px};
use smallvec::SmallVec;

use super::super::line_breaks::{BreakKind, LineBreaks};

impl LineLayout {
    pub(super) fn compute_wrap_boundaries(
        &self,
        text: &str,
        wrap_width: Pixels,
        max_lines: Option<usize>,
    ) -> SmallVec<[WrapBoundary; 1]> {
        let breaks = LineBreaks::new(text);
        let glyphs = self
            .runs
            .iter()
            .enumerate()
            .flat_map(|(run_ix, run)| {
                run.glyphs
                    .iter()
                    .enumerate()
                    .map(move |(glyph_ix, glyph)| (WrapBoundary { run_ix, glyph_ix }, glyph))
            })
            .collect::<Vec<_>>();
        // A boundary must separate complete logical clusters, including repeats
        // in another fallback run. A visual bidi run may have descending indices.
        let mut suffix_min = vec![text.len(); glyphs.len() + 1];
        for (ix, (_, glyph)) in glyphs.iter().enumerate().rev() {
            suffix_min[ix] = suffix_min[ix + 1].min(glyph.index);
        }
        let mut prefix_max = None;
        let mut boundaries = SmallVec::new();
        let mut start_x = px(0.);
        let mut start_ix = 0;
        let mut normal = None;
        let mut emergency = None;
        for (ix, (boundary, glyph)) in glyphs.iter().enumerate() {
            let cluster_start =
                prefix_max.is_none_or(|index| glyph.index > index) && suffix_min[ix] == glyph.index;
            prefix_max =
                Some(prefix_max.map_or(glyph.index, |index: usize| index.max(glyph.index)));
            if cluster_start && glyph.index > start_ix {
                let candidate = (*boundary, glyph.index, glyph.position.x);
                match breaks.at(glyph.index) {
                    BreakKind::Normal => normal = Some(candidate),
                    BreakKind::Emergency => emergency = Some(candidate),
                    BreakKind::Prohibited => {}
                }
            }
            let next_x = glyphs
                .get(ix + 1)
                .map_or(self.width, |(_, next)| next.position.x);
            if next_x - start_x <= wrap_width {
                continue;
            }
            if boundaries.len() >= max_lines.unwrap_or(usize::MAX).saturating_sub(1) {
                break;
            }
            let Some((boundary, index, x)) = normal.or(emergency) else {
                // The first protected unit does not fit. Preserve it until a
                // permitted boundary exists, even if that requires overflow.
                continue;
            };
            boundaries.push(boundary);
            start_ix = index;
            start_x = x;
            normal = None;
            emergency = None;
        }
        boundaries
    }
}
