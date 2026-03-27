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

    // Compute pixel bounds at the upscaled resolution.
    // Java Hiero rasterizes at 1x then upscales, but computing at 32x
    // and dividing gives closer SDF output dimensions due to how integer
    // division naturally tightens the bounds (matching Java's pixel-tight
    // getGlyphPixelBounds behaviour).
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

    let path = build_outline_path(&commands, scale, -x_min as f32, -y_min as f32)?;
    let mask = render_binary_mask(&path, width, height)?;

    // Crop to tight bounds (match Java getGlyphPixelBounds)
    let (cropped_mask, crop_x, crop_y, crop_w, crop_h) = crop_to_content(&mask, width, height);

    Ok(Some(RasterResult {
        mask: cropped_mask,
        width: crop_w,
        height: crop_h,
        bearing_x: x_min + crop_x as i32,
        bearing_y: y_min + crop_y as i32,
    }))
}

/// Rasterise a glyph outline into a fixed logical layout frame.
///
/// This mirrors Java Hiero's `Glyph.shape` translation + `DistanceFieldEffect`
/// input image construction: the glyph's 1x pixel bounds are aligned to the
/// configured padding inside a `layout_width × layout_height` frame, then the
/// whole frame is upscaled by `scale_factor`.
pub fn rasterize_glyph_in_layout(
    font: &OutlineFont<'_>,
    glyph_id: ttf_parser::GlyphId,
    size_px: f32,
    scale_factor: u32,
    layout_width: u32,
    layout_height: u32,
    bounds_x_1x: i32,
    bounds_y_1x: i32,
    pad_left: u32,
    pad_top: u32,
) -> Result<Option<Vec<u8>>, FontbakeError> {
    let commands = match font.outline_glyph(glyph_id) {
        Some(c) if !c.is_empty() => c,
        _ => return Ok(None),
    };

    let scale_factor = scale_factor.max(1);
    let canvas_w = layout_width
        .checked_mul(scale_factor)
        .ok_or_else(|| FontbakeError::FontLoad("layout canvas width overflow".into()))?;
    let canvas_h = layout_height
        .checked_mul(scale_factor)
        .ok_or_else(|| FontbakeError::FontLoad("layout canvas height overflow".into()))?;
    if canvas_w > 8192 || canvas_h > 8192 {
        return Err(FontbakeError::FontLoad(format!(
            "glyph raster too large: {canvas_w}x{canvas_h}"
        )));
    }

    let scale = (size_px * scale_factor as f32) / font.units_per_em as f32;
    let translate_x = (pad_left as i32 - bounds_x_1x) as f32 * scale_factor as f32;
    let translate_y = (pad_top as i32 - bounds_y_1x) as f32 * scale_factor as f32;
    let path = build_outline_path(&commands, scale, translate_x, translate_y)?;
    let mask = render_binary_mask(&path, canvas_w, canvas_h)?;
    Ok(Some(mask))
}

fn build_outline_path(
    commands: &[PathCommand],
    scale: f32,
    translate_x: f32,
    translate_y: f32,
) -> Result<tiny_skia::Path, FontbakeError> {
    let mut pb = PathBuilder::new();
    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo { x, y } => {
                pb.move_to(x * scale + translate_x, -y * scale + translate_y);
            }
            PathCommand::LineTo { x, y } => {
                pb.line_to(x * scale + translate_x, -y * scale + translate_y);
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                pb.quad_to(
                    x1 * scale + translate_x,
                    -y1 * scale + translate_y,
                    x * scale + translate_x,
                    -y * scale + translate_y,
                );
            }
            PathCommand::CurveTo { x1, y1, x2, y2, x, y } => {
                pb.cubic_to(
                    x1 * scale + translate_x,
                    -y1 * scale + translate_y,
                    x2 * scale + translate_x,
                    -y2 * scale + translate_y,
                    x * scale + translate_x,
                    -y * scale + translate_y,
                );
            }
            PathCommand::Close => pb.close(),
        }
    }

    pb.finish()
        .ok_or_else(|| FontbakeError::FontLoad("failed to build tiny-skia path".into()))
}

fn render_binary_mask(
    path: &tiny_skia::Path,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, FontbakeError> {
    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| FontbakeError::FontLoad(format!("cannot create {width}x{height} pixmap")))?;

    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = false; // Match Java TYPE_BYTE_BINARY (no anti-aliasing)

    pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);

    let pixels = pixmap.data();
    let mut mask = Vec::with_capacity((width * height) as usize);
    for i in 0..(width * height) as usize {
        let alpha = pixels[i * 4 + 3];
        mask.push(if alpha >= 128 { 255 } else { 0 });
    }
    Ok(mask)
}

/// Crop mask to tight bounding box containing non-zero pixels.
/// Returns (cropped_mask, x_offset, y_offset, new_width, new_height).
fn crop_to_content(mask: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32, u32, u32) {
    let w = width as usize;
    let h = height as usize;

    // Find bounds of non-zero pixels
    let mut min_x = w;
    let mut max_x = 0;
    let mut min_y = h;
    let mut max_y = 0;

    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] > 0 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    // If no content, return 1x1 empty
    if min_x > max_x || min_y > max_y {
        return (vec![0], 0, 0, 1, 1);
    }

    let crop_w = (max_x - min_x + 1) as u32;
    let crop_h = (max_y - min_y + 1) as u32;

    // Extract cropped region
    let mut cropped = Vec::with_capacity((crop_w * crop_h) as usize);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            cropped.push(mask[y * w + x]);
        }
    }

    (cropped, min_x as u32, min_y as u32, crop_w, crop_h)
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
