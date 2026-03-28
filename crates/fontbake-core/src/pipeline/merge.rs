//! `merge_fonts` pipeline — merge multiple sets of `GlyphRecord`s into a
//! single atlas output.
//!
//! This supports the workflow: import existing BMFont glyphs + build new
//! outline glyphs → merge into one combined font.
//!
//! Merge rules:
//! - Glyphs are merged by codepoint.
//! - When the same codepoint exists in multiple sources, the first source wins
//!   (caller controls priority by ordering the input slices).
//! - Imported bitmap glyphs are pixel-passthrough — never re-rasterized.
//! - After merge, all glyphs are re-packed into fresh atlas pages.

use crate::export::bmfont_text::{encode_atlas_png, glyphs_to_fnt};
use crate::model::{BuildResult, FontbakeError, GlyphRecord};
use crate::pack::hiero_rows::pack_glyphs;
use std::collections::HashSet;

/// Merge multiple glyph sets into a single `BuildResult`.
///
/// `glyph_sets` is ordered by priority — first set wins on codepoint conflicts.
pub fn merge_fonts(
    glyph_sets: &[&[GlyphRecord]],
    face: &str,
    font_size: i32,
    line_height: u32,
    base: u32,
    page_width: u32,
    page_height: u32,
    padding: [i32; 4],
    spacing: [i32; 2],
) -> Result<BuildResult, FontbakeError> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut merged: Vec<GlyphRecord> = Vec::new();

    for set in glyph_sets {
        for glyph in *set {
            if seen.insert(glyph.codepoint) {
                // Reset packing fields — they'll be reassigned
                let mut g = glyph.clone();
                g.page = 0;
                g.x = 0;
                g.y = 0;
                merged.push(g);
            }
        }
    }

    // Sort by codepoint for deterministic output
    merged.sort_by_key(|g| g.codepoint);

    // Pack
    let pages = pack_glyphs(&mut merged, page_width, page_height)?;

    // Export
    let page_filenames: Vec<String> = (0..pages.len())
        .map(|i| {
            format!(
                "{}{}.png",
                face,
                if i == 0 {
                    String::new()
                } else {
                    format!("_{i}")
                }
            )
        })
        .collect();

    let fnt_text = glyphs_to_fnt(
        &merged,
        face,
        font_size,
        line_height,
        base,
        page_width,
        page_height,
        &page_filenames,
        padding,
        spacing,
    );

    let page_pngs: Vec<Vec<u8>> = pages
        .iter()
        .map(|p| encode_atlas_png(p))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(BuildResult {
        fnt_text,
        page_pngs,
        glyphs: merged,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceKind;

    fn make_glyph(cp: u32, source: &str, w: u32, h: u32) -> GlyphRecord {
        let mut g = GlyphRecord::new(cp, SourceKind::Outline, source.into());
        g.width = w;
        g.height = h;
        g.xadvance = 10;
        if w > 0 && h > 0 {
            g.bitmap_rgba = vec![128u8; (w * h * 4) as usize];
        }
        g
    }

    #[test]
    fn merge_deduplicates_by_codepoint() {
        let set_a = vec![make_glyph(65, "a", 10, 10), make_glyph(66, "a", 10, 10)];
        let set_b = vec![make_glyph(65, "b", 8, 8), make_glyph(67, "b", 10, 10)];

        let result = merge_fonts(
            &[&set_a, &set_b],
            "test",
            12,
            14,
            10,
            128,
            128,
            [0, 0, 0, 0],
            [0, 0],
        )
        .unwrap();

        // Should have 3 glyphs: A from set_a, B from set_a, C from set_b
        assert_eq!(result.glyphs.len(), 3);
        let a = result.glyphs.iter().find(|g| g.codepoint == 65).unwrap();
        assert_eq!(a.source_id, "a"); // first source wins
        assert_eq!(a.width, 10); // from set_a, not set_b's 8
    }

    #[test]
    fn merge_empty_sets() {
        let result = merge_fonts(
            &[&[], &[]],
            "test",
            12,
            14,
            10,
            128,
            128,
            [0, 0, 0, 0],
            [0, 0],
        )
        .unwrap();
        assert!(result.glyphs.is_empty());
    }

    #[test]
    fn merge_preserves_bitmap_import_kind() {
        let mut g = make_glyph(65, "imported", 10, 10);
        g.source_kind = SourceKind::BitmapImport;
        let set = vec![g];

        let result =
            merge_fonts(&[&set], "test", 12, 14, 10, 128, 128, [0, 0, 0, 0], [0, 0]).unwrap();

        assert_eq!(result.glyphs[0].source_kind, SourceKind::BitmapImport);
    }
}
