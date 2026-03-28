//! FreeType-based hinted glyph bounds.
//!
//! Uses FreeType's TrueType bytecode interpreter to obtain pixel-snapped
//! glyph bounding boxes. Currently only the **vertical** bounds (y, height)
//! match Java AWT closely — FreeType and Java's Windows GDI rasterizer
//! agree on vertical hinting but differ on horizontal grid-fitting.

use crate::model::FontbakeError;

/// Hinted vertical bounds for a single glyph.
#[derive(Debug, Clone, Copy)]
pub struct HintedVerticalBounds {
    /// Top edge of the hinted glyph, relative to baseline (negative = above).
    pub y: i32,
    /// Pixel height of the hinted glyph.
    pub height: u32,
}

/// A loaded FreeType face for hinted metrics queries.
pub struct FreetypeFont {
    _library: freetype::Library,
    face: freetype::Face,
}

impl FreetypeFont {
    /// Load a font from raw bytes at the given pixel size.
    pub fn load(data: &[u8], size_px: u32) -> Result<Self, FontbakeError> {
        let library = freetype::Library::init()
            .map_err(|e| FontbakeError::FontLoad(format!("FreeType init failed: {e}")))?;

        let face = library
            .new_memory_face(data.to_vec(), 0)
            .map_err(|e| FontbakeError::FontLoad(format!("FreeType load failed: {e}")))?;

        face.set_pixel_sizes(0, size_px)
            .map_err(|e| FontbakeError::FontLoad(format!("FreeType set_pixel_sizes: {e}")))?;

        Ok(Self {
            _library: library,
            face,
        })
    }

    /// Get hinted vertical bounds for a glyph by its codepoint.
    ///
    /// Returns `None` for empty glyphs (e.g. space).
    pub fn hinted_vertical_bounds(
        &self,
        codepoint: u32,
    ) -> Result<Option<HintedVerticalBounds>, FontbakeError> {
        self.face
            .load_char(codepoint as usize, freetype::face::LoadFlag::DEFAULT)
            .map_err(|e| {
                FontbakeError::FontLoad(format!("FreeType load_char U+{codepoint:04X}: {e}"))
            })?;

        let metrics = self.face.glyph().metrics();

        let bearing_y_26_6 = metrics.horiBearingY;
        let height_26_6 = metrics.height;

        if height_26_6 == 0 {
            return Ok(None);
        }

        // Convert 26.6 fixed-point to integer pixel bounds.
        let y_top_up = floor_26_6(bearing_y_26_6 - height_26_6);
        let y_bottom_up = ceil_26_6(bearing_y_26_6);
        let h = (y_bottom_up - y_top_up) as u32;

        if h == 0 {
            return Ok(None);
        }

        Ok(Some(HintedVerticalBounds {
            y: -y_bottom_up, // Y-up → Y-down (Java convention)
            height: h,
        }))
    }
}

/// Floor a 26.6 fixed-point value to integer.
fn floor_26_6(v: i64) -> i32 {
    (v >> 6) as i32
}

/// Ceil a 26.6 fixed-point value to integer.
fn ceil_26_6(v: i64) -> i32 {
    ((v + 63) >> 6) as i32
}
