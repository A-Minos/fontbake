//! Hiero glyph-page packer.
//!
//! Mirrors the real libGDX Hiero packing flow more closely than a simple shelf
//! packer:
//! 1. Stable-sort queued glyphs by height descending.
//! 2. Fill one page by scanning the remaining glyph queue in order.
//! 3. For each glyph, try the best existing non-last row, then the last row,
//!    then a new row, using the same strict `<` / `>=` fit checks as Hiero.
//! 4. Glyphs that don't fit stay in the queue for the next page.
//!
//! After packing, each `GlyphRecord` has its `page`, `x`, `y` fields set.

use crate::model::{AtlasPage, FontbakeError, GlyphRecord};

#[derive(Debug, Clone, Copy)]
struct Row {
    x: u32,
    y: u32,
    height: u32,
}

/// Pack glyphs into atlas pages using Hiero's glyph-page algorithm.
pub fn pack_glyphs(
    glyphs: &mut [GlyphRecord],
    page_width: u32,
    page_height: u32,
) -> Result<Vec<AtlasPage>, FontbakeError> {
    if page_width == 0 || page_height == 0 {
        return Err(FontbakeError::Pack("page dimensions must be > 0".into()));
    }

    // Match UnicodeFont.heightComparator: stable sort by height only.
    let mut remaining: Vec<usize> = (0..glyphs.len()).collect();
    remaining.sort_by(|&a, &b| glyphs[b].height.cmp(&glyphs[a].height));

    // Zero-size glyphs don't participate in packing. Hiero attaches them to the
    // currently loading page; for offline export a canonical (0,0) on page 0 is
    // sufficient and keeps them out of the geometric algorithm.
    for &idx in &remaining {
        if glyphs[idx].width == 0 || glyphs[idx].height == 0 {
            glyphs[idx].page = 0;
            glyphs[idx].x = 0;
            glyphs[idx].y = 0;
        }
    }
    remaining.retain(|&idx| glyphs[idx].width > 0 && glyphs[idx].height > 0);

    for &idx in &remaining {
        let gw = glyphs[idx].width;
        let gh = glyphs[idx].height;
        if gw >= page_width || gh >= page_height {
            return Err(FontbakeError::Pack(format!(
                "glyph U+{:04X} ({}x{}) exceeds Hiero packable size for page ({}x{})",
                glyphs[idx].codepoint, gw, gh, page_width, page_height
            )));
        }
    }

    let mut pages = Vec::new();

    while !remaining.is_empty() {
        let page_index = pages.len() as u32;
        let mut page = AtlasPage::new(page_width, page_height);
        let mut rows = vec![Row {
            x: 0,
            y: 0,
            height: 0,
        }];
        let mut next_remaining = Vec::new();
        let mut wrote_visible = false;

        for idx in remaining.drain(..) {
            let gw = glyphs[idx].width;
            let gh = glyphs[idx].height;

            if let Some((x, y)) = try_place_in_page(&mut rows, page_width, page_height, gw, gh) {
                glyphs[idx].page = page_index;
                glyphs[idx].x = x;
                glyphs[idx].y = y;
                blit_glyph(&mut page, &glyphs[idx]);
                wrote_visible = true;
            } else {
                next_remaining.push(idx);
            }
        }

        if !wrote_visible {
            let idx = next_remaining[0];
            return Err(FontbakeError::Pack(format!(
                "glyph U+{:04X} ({}x{}) cannot fit any Hiero row on page ({}x{})",
                glyphs[idx].codepoint,
                glyphs[idx].width,
                glyphs[idx].height,
                page_width,
                page_height
            )));
        }

        pages.push(page);
        remaining = next_remaining;
    }

    if pages.is_empty() {
        pages.push(AtlasPage::new(page_width, page_height));
    }

    Ok(pages)
}

fn try_place_in_page(
    rows: &mut Vec<Row>,
    page_width: u32,
    page_height: u32,
    width: u32,
    height: u32,
) -> Option<(u32, u32)> {
    let mut best_row: Option<usize> = None;

    // Match GlyphPage.loadGlyphs: scan any row before the last and choose the
    // smallest-height row that can still contain the glyph.
    if rows.len() > 1 {
        for row_idx in 0..rows.len() - 1 {
            let row = rows[row_idx];
            if row.x + width >= page_width {
                continue;
            }
            if row.y + height >= page_height {
                continue;
            }
            if height > row.height {
                continue;
            }
            if best_row.is_none() || row.height < rows[best_row.unwrap()].height {
                best_row = Some(row_idx);
            }
        }
    }

    if let Some(row_idx) = best_row {
        let row = &mut rows[row_idx];
        let x = row.x;
        let y = row.y;
        row.x += width;
        return Some((x, y));
    }

    let last_idx = rows.len() - 1;
    let last = rows[last_idx];
    if last.y + height >= page_height {
        return None;
    }

    if last.x + width < page_width {
        let row = &mut rows[last_idx];
        let x = row.x;
        let y = row.y;
        row.height = row.height.max(height);
        row.x += width;
        return Some((x, y));
    }

    if last.y + last.height + height < page_height {
        let new_y = last.y + last.height;
        rows.push(Row {
            x: width,
            y: new_y,
            height,
        });
        return Some((0, new_y));
    }

    None
}

/// Blit a glyph's RGBA bitmap onto an atlas page at its assigned (x, y).
fn blit_glyph(page: &mut AtlasPage, glyph: &GlyphRecord) {
    if glyph.width == 0 || glyph.height == 0 || glyph.bitmap_rgba.is_empty() {
        return;
    }

    let pw = page.width as usize;
    let gw = glyph.width as usize;
    let gh = glyph.height as usize;
    let gx = glyph.x as usize;
    let gy = glyph.y as usize;

    for row in 0..gh {
        let src_start = row * gw * 4;
        let src_end = src_start + gw * 4;
        let dst_start = ((gy + row) * pw + gx) * 4;

        if src_end <= glyph.bitmap_rgba.len() && dst_start + gw * 4 <= page.rgba.len() {
            page.rgba[dst_start..dst_start + gw * 4]
                .copy_from_slice(&glyph.bitmap_rgba[src_start..src_end]);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceKind;

    fn make_glyph(cp: u32, w: u32, h: u32) -> GlyphRecord {
        let mut g = GlyphRecord::new(cp, SourceKind::Outline, "test".into());
        g.width = w;
        g.height = h;
        if w > 0 && h > 0 {
            g.bitmap_rgba = vec![255u8; (w * h * 4) as usize];
        }
        g
    }

    #[test]
    fn single_glyph_fits() {
        let mut glyphs = vec![make_glyph(65, 10, 10)];
        let pages = pack_glyphs(&mut glyphs, 64, 64).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(glyphs[0].page, 0);
        assert_eq!(glyphs[0].x, 0);
        assert_eq!(glyphs[0].y, 0);
    }

    #[test]
    fn row_wrapping() {
        // 3 glyphs of width 30 in a 64-wide page → first two fit in row 0,
        // third wraps to row 1
        let mut glyphs = vec![
            make_glyph(65, 30, 10),
            make_glyph(66, 30, 10),
            make_glyph(67, 30, 10),
        ];
        let pages = pack_glyphs(&mut glyphs, 64, 64).unwrap();
        assert_eq!(pages.len(), 1);
        // All three should be on page 0
        assert!(glyphs.iter().all(|g| g.page == 0));
    }

    #[test]
    fn page_overflow() {
        // Hiero uses strict `<` for fit checks (row.x + width >= pageWidth rejects),
        // so page must be at least 1px wider/taller than the total glyph extent.
        // 4 glyphs of 32x32 in a 65x65 page → all fit on one page (2x2 grid)
        let mut glyphs = vec![
            make_glyph(65, 32, 32),
            make_glyph(66, 32, 32),
            make_glyph(67, 32, 32),
            make_glyph(68, 32, 32),
        ];
        let pages = pack_glyphs(&mut glyphs, 65, 65).unwrap();
        assert_eq!(pages.len(), 1);

        // 5th glyph should overflow to page 2
        let mut glyphs = vec![
            make_glyph(65, 32, 32),
            make_glyph(66, 32, 32),
            make_glyph(67, 32, 32),
            make_glyph(68, 32, 32),
            make_glyph(69, 32, 32),
        ];
        let pages = pack_glyphs(&mut glyphs, 65, 65).unwrap();
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn zero_size_glyph_placed() {
        let mut glyphs = vec![make_glyph(32, 0, 0), make_glyph(65, 10, 10)];
        let pages = pack_glyphs(&mut glyphs, 64, 64).unwrap();
        assert_eq!(pages.len(), 1);
        // Space glyph should be placed at (0,0)
        assert_eq!(glyphs[0].x, 0);
        assert_eq!(glyphs[0].y, 0);
    }

    #[test]
    fn height_sort_descending() {
        // Tallest glyph should be placed first (gets row 0)
        let mut glyphs = vec![
            make_glyph(65, 10, 5),  // short
            make_glyph(66, 10, 20), // tall
            make_glyph(67, 10, 10), // medium
        ];
        let _pages = pack_glyphs(&mut glyphs, 64, 64).unwrap();
        // The tall glyph (66) should be at y=0
        assert_eq!(glyphs[1].y, 0);
    }

    #[test]
    fn glyph_too_large_errors() {
        let mut glyphs = vec![make_glyph(65, 100, 100)];
        let result = pack_glyphs(&mut glyphs, 64, 64);
        assert!(result.is_err());
    }

    #[test]
    fn zero_page_size_errors() {
        let mut glyphs = vec![make_glyph(65, 10, 10)];
        assert!(pack_glyphs(&mut glyphs, 0, 64).is_err());
    }
}
