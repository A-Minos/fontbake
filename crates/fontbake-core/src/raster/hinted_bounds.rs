//! Hinted glyph bounds via skrifa (pure Rust, WASM-compatible).
//!
//! Uses skrifa's TrueType hinting engine to obtain pixel-snapped vertical
//! glyph bounds that match Java AWT's `getGlyphPixelBounds`.

use crate::model::FontbakeError;
use skrifa::instance::Size;
use skrifa::outline::{
    DrawSettings, HintingInstance, HintingOptions, OutlineGlyphCollection, OutlinePen,
};
use skrifa::prelude::*;

/// Hinted vertical bounds for a single glyph.
#[derive(Debug, Clone, Copy)]
pub struct HintedVerticalBounds {
    /// Top edge of the hinted glyph, relative to baseline (negative = above).
    pub y: i32,
    /// Pixel height of the hinted glyph.
    pub height: u32,
}

/// A loaded font with skrifa hinting for metrics queries.
pub struct HintedFont<'a> {
    font: FontRef<'a>,
    hinting: HintingInstance,
}

impl<'a> HintedFont<'a> {
    /// Load a font from raw bytes at the given pixel size.
    pub fn load(data: &'a [u8], size_px: u32) -> Result<Self, FontbakeError> {
        let font = FontRef::new(data)
            .map_err(|e| FontbakeError::FontLoad(format!("skrifa load failed: {e}")))?;

        let size = Size::new(size_px as f32);
        let outlines = font.outline_glyphs();

        let hinting = HintingInstance::new(
            &outlines,
            size,
            LocationRef::default(),
            HintingOptions::default(),
        )
        .map_err(|e| FontbakeError::FontLoad(format!("skrifa hinting init failed: {e}")))?;

        Ok(Self { font, hinting })
    }

    /// Get hinted vertical bounds for a glyph by its codepoint.
    ///
    /// Returns `None` for empty glyphs (e.g. space).
    pub fn hinted_vertical_bounds(
        &self,
        codepoint: u32,
    ) -> Result<Option<HintedVerticalBounds>, FontbakeError> {
        let glyph_id = match self.font.charmap().map(codepoint) {
            Some(id) => id,
            None => return Ok(None),
        };

        let outlines: OutlineGlyphCollection<'_> = self.font.outline_glyphs();
        let glyph = match outlines.get(glyph_id) {
            Some(g) => g,
            None => return Ok(None),
        };

        let settings = DrawSettings::hinted(&self.hinting, false);
        let mut pen = YBoundsPen::new();

        glyph
            .draw(settings, &mut pen)
            .map_err(|e| FontbakeError::FontLoad(format!("skrifa draw U+{codepoint:04X}: {e}")))?;

        if !pen.has_data {
            return Ok(None);
        }

        // Convert Y-up float bounds to Y-down integer pixel bounds (Java convention).
        let y_top = pen.min_y.floor() as i32;
        let y_bottom = pen.max_y.ceil() as i32;
        let h = (y_bottom - y_top) as u32;

        if h == 0 {
            return Ok(None);
        }

        Ok(Some(HintedVerticalBounds {
            y: -y_bottom, // Y-up → Y-down
            height: h,
        }))
    }
}

/// Pen that tracks only Y-axis bounds, including Bezier curve extrema.
struct YBoundsPen {
    min_y: f32,
    max_y: f32,
    cur_y: f32,
    has_data: bool,
}

impl YBoundsPen {
    fn new() -> Self {
        Self {
            min_y: f32::MAX,
            max_y: f32::MIN,
            cur_y: 0.0,
            has_data: false,
        }
    }

    fn track(&mut self, y: f32) {
        self.has_data = true;
        if y < self.min_y {
            self.min_y = y;
        }
        if y > self.max_y {
            self.max_y = y;
        }
    }
}

impl OutlinePen for YBoundsPen {
    fn move_to(&mut self, _x: f32, y: f32) {
        self.track(y);
        self.cur_y = y;
    }

    fn line_to(&mut self, _x: f32, y: f32) {
        self.track(y);
        self.cur_y = y;
    }

    fn quad_to(&mut self, _cx0: f32, cy0: f32, _x: f32, y: f32) {
        // Quadratic Bezier: B(t) = (1-t)²p0 + 2t(1-t)c0 + t²p1
        // Extremum: t = (p0 - c0) / (p0 - 2c0 + p1)
        let p0 = self.cur_y;
        let denom = p0 - 2.0 * cy0 + y;
        if denom.abs() > 1e-6 {
            let t = (p0 - cy0) / denom;
            if (0.0..=1.0).contains(&t) {
                let v = (1.0 - t) * (1.0 - t) * p0 + 2.0 * t * (1.0 - t) * cy0 + t * t * y;
                self.track(v);
            }
        }
        self.track(y);
        self.cur_y = y;
    }

    fn curve_to(&mut self, _cx0: f32, cy0: f32, _cx1: f32, cy1: f32, _x: f32, y: f32) {
        // Cubic Bezier: B(t) = (1-t)³p0 + 3t(1-t)²c0 + 3t²(1-t)c1 + t³p1
        // B'(t) = 3[(c0-p0)(1-t)² + 2(c1-c0)t(1-t) + (p1-c1)t²]
        // Solve B'(t) = 0: at² + bt + c = 0
        let p0 = self.cur_y;
        let a = -p0 + 3.0 * cy0 - 3.0 * cy1 + y;
        let b = 2.0 * (p0 - 2.0 * cy0 + cy1);
        let c = cy0 - p0;

        if a.abs() > 1e-6 {
            let disc = b * b - 4.0 * a * c;
            if disc >= 0.0 {
                let sqrt_disc = disc.sqrt();
                for t in [(-b + sqrt_disc) / (2.0 * a), (-b - sqrt_disc) / (2.0 * a)] {
                    if (0.0..=1.0).contains(&t) {
                        let mt = 1.0 - t;
                        let v = mt * mt * mt * p0
                            + 3.0 * mt * mt * t * cy0
                            + 3.0 * mt * t * t * cy1
                            + t * t * t * y;
                        self.track(v);
                    }
                }
            }
        } else if b.abs() > 1e-6 {
            // Linear case: bt + c = 0
            let t = -c / b;
            if (0.0..=1.0).contains(&t) {
                let mt = 1.0 - t;
                let v = mt * mt * mt * p0
                    + 3.0 * mt * mt * t * cy0
                    + 3.0 * mt * t * t * cy1
                    + t * t * t * y;
                self.track(v);
            }
        }

        self.track(y);
        self.cur_y = y;
    }

    fn close(&mut self) {}
}
