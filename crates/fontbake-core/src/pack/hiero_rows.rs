//! Hiero-style row (shelf) packer.
//!
//! Replicates Hiero's atlas packing strategy:
//! 1. Sort glyphs by height descending (stable sort preserves codepoint order
//!    for equal heights).
//! 2. Place glyphs left-to-right in rows (shelves).
//! 3. Each shelf's height is the height of its tallest glyph.
//! 4. When a glyph doesn't fit in the current row, start a new row.
//! 5. When a new row doesn't fit on the current page, start a new page.
//!
//! After packing, each `GlyphRecord` has its `page`, `x`, `y` fields set.

use crate::model::{AtlasPage, FontbakeError, GlyphRecord};

/// Pack glyphs into atlas pages using Hiero's row-based algorithm.
///
/// Mutates each glyph's `page`, `x`, `y` fields in place.
/// Returns the atlas pages with glyph bitmaps composited.
pub fn pack_glyphs(
    glyphs: &mut [GlyphRecord],
    page_width: u32,
    page_height: u32,
) -> Result<Vec<AtlasPage>, FontbakeError> {
    if page_width == 0 || page_height == 0 {
        return Err(FontbakeError::Pack("page dimensions must be > 0".into()));
    }

    // Build index array sorted by height descending, then by codepoint for stability
    let mut indices: Vec<usize> = (0..glyphs.len()).collect();
    indices.sort_by(|&a, &b| {
        glyphs[b]
            .height
            .cmp(&glyphs[a].height)
            .then_with(|| glyphs[a].codepoint.cmp(&glyphs[b].codepoint))
    });

    let mut pages: Vec<AtlasPage> = vec![AtlasPage::new(page_width, page_height)];
    let mut shelf_x: u32 = 0;
    let mut shelf_y: u32 = 0;
    let mut shelf_h: u32 = 0;
    let mut current_page: u32 = 0;

    for &idx in &indices {
        let gw = glyphs[idx].width;
        let gh = glyphs[idx].height;

        // Zero-size glyphs (e.g. space) get placed at (0,0) on current page
        if gw == 0 || gh == 0 {
            glyphs[idx].page = current_page;
            glyphs[idx].x = 0;
            glyphs[idx].y = 0;
            continue;
        }

        // Check if glyph is too large for any page
        if gw > page_width || gh > page_height {
            return Err(FontbakeError::Pack(format!(
                "glyph U+{:04X} ({}x{}) exceeds page size ({}x{})",
                glyphs[idx].codepoint, gw, gh, page_width, page_height
            )));
        }

        // Try to fit in current shelf
        if shelf_x + gw <= page_width && shelf_y + gh <= page_height {
            // Fits in current shelf
        } else if shelf_x + gw > page_width {
            // Start new shelf
            shelf_y += shelf_h;
            shelf_x = 0;
            shelf_h = 0;

            if shelf_y + gh > page_height {
                // New page
                current_page += 1;
                pages.push(AtlasPage::new(page_width, page_height));
                shelf_x = 0;
                shelf_y = 0;
                shelf_h = 0;
            }
        } else {
            // shelf_y + gh > page_height but shelf_x + gw <= page_width
            // Try next shelf first
            shelf_y += shelf_h;
            shelf_x = 0;
            shelf_h = 0;

            if shelf_y + gh > page_height {
                // New page
                current_page += 1;
                pages.push(AtlasPage::new(page_width, page_height));
                shelf_x = 0;
                shelf_y = 0;
                shelf_h = 0;
            }
        }

        // Place glyph
        glyphs[idx].page = current_page;
        glyphs[idx].x = shelf_x;
        glyphs[idx].y = shelf_y;

        // Blit bitmap onto atlas page
        blit_glyph(&mut pages[current_page as usize], &glyphs[idx]);

        shelf_x += gw;
        if gh > shelf_h {
            shelf_h = gh;
        }
    }

    Ok(pages)
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
        // 4 glyphs of 32x32 in a 64x64 page → all fit on one page (2x2 grid)
        let mut glyphs = vec![
            make_glyph(65, 32, 32),
            make_glyph(66, 32, 32),
            make_glyph(67, 32, 32),
            make_glyph(68, 32, 32),
        ];
        let pages = pack_glyphs(&mut glyphs, 64, 64).unwrap();
        assert_eq!(pages.len(), 1);

        // 5th glyph should overflow to page 1
        let mut glyphs = vec![
            make_glyph(65, 32, 32),
            make_glyph(66, 32, 32),
            make_glyph(67, 32, 32),
            make_glyph(68, 32, 32),
            make_glyph(69, 32, 32),
        ];
        let pages = pack_glyphs(&mut glyphs, 64, 64).unwrap();
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
