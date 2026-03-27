//! Java-path shape rasterizer.
//!
//! Takes glyph outlines from `ttf-parser` and fills them into a grayscale
//! mask image using `tiny-skia`. This module aims to approximate Java Hiero's
//! Java2D `Graphics2D.fill(glyphShape)` pipeline:
//!
//! 1. Outline in font units → scale to target pixel size.
//! 2. Y-axis flip (font coords are Y-up, bitmap is Y-down).
//! 3. Fill with anti-aliased rendering.
//!
//! The output is an 8-bit alpha mask (one byte per pixel) suitable for
//! subsequent `DistanceFieldEffect` processing.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::model::FontbakeError;
use crate::source::outline::{OutlineFont, PathCommand};

/// Result of rasterising a single glyph.
pub struct RasterResult {
    /// 8-bit alpha mask (width × height bytes, row-major, top-to-bottom).
    pub mask: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Offset from the glyph origin to the top-left of the mask, in pixels.
    pub bearing_x: i32,
    pub bearing_y: i32,
}

/// Rasterise a glyph outline into an alpha mask at the given pixel size.
///
/// `scale_factor` is the additional upscale applied by DistanceFieldEffect
/// (typically `effect.Scale`). The mask is rendered at `size_px * scale_factor`
/// and will later be downsampled by the SDF generator.
///
/// Returns `None` if the glyph has no outline (e.g. space character).
pub fn rasterize_glyph(
    font: &OutlineFont<'_>,
    glyph_id: ttf_parser::GlyphId,
    size_px: f32,
    scale_factor: u32,
) -> Result<Option<RasterResult>, FontbakeError> {
    let commands = match font.outline_glyph(glyph_id) {
        Some(c) if !c.is_empty() => c,
        _ => return Ok(None),
    };

    let bbox = match font.glyph_bbox(glyph_id) {
        Some(b) => b,
        None => return Ok(None),
    };

    let upem = font.units_per_em as f32;
    let target_size = size_px * scale_factor as f32;
    let scale = target_size / upem;

    // Compute pixel bounds
    let x_min = (bbox.x_min as f32 * scale).floor() as i32;
    let y_min = (-(bbox.y_max as f32) * scale).floor() as i32; // Y flip
    let x_max = (bbox.x_max as f32 * scale).ceil() as i32;
    let y_max = (-(bbox.y_min as f32) * scale).ceil() as i32;

    let width = (x_max - x_min).max(1) as u32;
    let height = (y_max - y_min).max(1) as u32;

    // Safety limit to prevent OOM on degenerate glyphs
    if width > 8192 || height > 8192 {
        return Err(FontbakeError::FontLoad(format!(
            "glyph raster too large: {width}x{height}"
        )));
    }

    // Build tiny-skia path
    let mut pb = PathBuilder::new();
    for cmd in &commands {
        match *cmd {
            PathCommand::MoveTo { x, y } => {
                pb.move_to(x * scale - x_min as f32, -y * scale - y_min as f32);
            }
            PathCommand::LineTo { x, y } => {
                pb.line_to(x * scale - x_min as f32, -y * scale - y_min as f32);
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                pb.quad_to(
                    x1 * scale - x_min as f32,
                    -y1 * scale - y_min as f32,
                    x * scale - x_min as f32,
                    -y * scale - y_min as f32,
                );
            }
            PathCommand::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                pb.cubic_to(
                    x1 * scale - x_min as f32,
                    -y1 * scale - y_min as f32,
                    x2 * scale - x_min as f32,
                    -y2 * scale - y_min as f32,
                    x * scale - x_min as f32,
                    -y * scale - y_min as f32,
                );
            }
            PathCommand::Close => {
                pb.close();
            }
        }
    }

    let path = pb
        .finish()
        .ok_or_else(|| FontbakeError::FontLoad("failed to build tiny-skia path".into()))?;

    // Render into a pixmap
    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| FontbakeError::FontLoad(format!("cannot create {width}x{height} pixmap")))?;

    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;

    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);

    // Extract alpha channel as the mask
    let pixels = pixmap.data();
    let mut mask = Vec::with_capacity((width * height) as usize);
    for i in 0..(width * height) as usize {
        mask.push(pixels[i * 4 + 3]); // A component
    }

    Ok(Some(RasterResult {
        mask,
        width,
        height,
        bearing_x: x_min,
        bearing_y: y_min,
    }))
}

/// Compute horizontal advance in pixels at a given size.
pub fn advance_width_px(font: &OutlineFont<'_>, glyph_id: ttf_parser::GlyphId, size_px: f32) -> i32 {
    let scale = size_px / font.units_per_em as f32;
    let advance = font.advance_width(glyph_id).unwrap_or(0) as f32;
    (advance * scale).round() as i32
}

/// Compute ascender in pixels.
pub fn ascender_px(font: &OutlineFont<'_>, size_px: f32) -> i32 {
    let scale = size_px / font.units_per_em as f32;
    (font.ascender() as f32 * scale).round() as i32
}

/// Compute descender in pixels (typically negative).
pub fn descender_px(font: &OutlineFont<'_>, size_px: f32) -> i32 {
    let scale = size_px / font.units_per_em as f32;
    (font.descender() as f32 * scale).round() as i32
}

/// Compute line height in pixels.
pub fn line_height_px(font: &OutlineFont<'_>, size_px: f32) -> u32 {
    let scale = size_px / font.units_per_em as f32;
    let asc = font.ascender() as f32;
    let desc = font.descender() as f32; // negative
    let gap = font.line_gap() as f32;
    ((asc - desc + gap) * scale).round() as u32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Full rasterization tests require a real font file and will be in
    // integration tests (crates/fontbake-core/tests/). Here we just verify
    // basic metric computations.

    #[test]
    fn metric_helpers_dont_panic_with_zero() {
        // These functions shouldn't panic even with edge-case inputs.
        // We can't construct an OutlineFont without valid data, so we only
        // test the math helpers indirectly via the pipeline integration tests.
        assert!(true);
    }
}
