//! `build_from_config` pipeline — the main entry point for building a font
//! from a `.hiero` configuration.
//!
//! Flow:
//! 1. Parse glyph_text into codepoints.
//! 2. Load primary + fallback fonts.
//! 3. For each codepoint, resolve which font provides it (first-hit-wins).
//! 4. Rasterize each outline glyph into a high-res mask.
//! 5. Apply DistanceFieldEffect to produce final glyph bitmaps.
//! 6. Pack all glyphs into atlas pages.
//! 7. Export as text BMFont (.fnt) + PNG pages.

use crate::effect::distance_field::{generate_distance_field, DistanceFieldConfig};
use crate::export::bmfont_text::{encode_atlas_png, glyphs_to_fnt};
use crate::model::{
    BuildResult, BuildSpec, EffectSpec, FontbakeError, GlyphRecord, SourceKind,
};
use crate::pack::hiero_rows::pack_glyphs;
use crate::raster::java_shape::{advance_width_px, rasterize_glyph, rasterize_glyph_in_layout};
use crate::raster::hinted_bounds::HintedFont;
use crate::source::outline::{resolve_codepoint, OutlineFont};

/// Named font data for the pipeline — bytes + identifier.
pub struct FontAsset<'a> {
    pub data: &'a [u8],
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedGlyphPlan {
    codepoint: char,
    font_idx: usize,
    glyph_id: ttf_parser::GlyphId,
    missing: bool,
}

fn resolve_glyph_plan<'a>(fonts: &[&OutlineFont<'a>], codepoint: char) -> ResolvedGlyphPlan {
    debug_assert!(!fonts.is_empty(), "font chain must contain a primary font");
    if let Some((font_idx, glyph_id)) = resolve_codepoint(fonts, codepoint) {
        ResolvedGlyphPlan {
            codepoint,
            font_idx,
            glyph_id,
            missing: false,
        }
    } else {
        // Match Hiero's missing-glyph behaviour by falling back to the primary
        // font's .notdef glyph instead of emitting a fake empty record.
        ResolvedGlyphPlan {
            codepoint,
            font_idx: 0,
            glyph_id: ttf_parser::GlyphId(0),
            missing: true,
        }
    }
}

fn should_emit_kerning(left: ResolvedGlyphPlan, right: ResolvedGlyphPlan) -> bool {
    !left.missing && !right.missing && left.font_idx == right.font_idx
}

/// Build a font from a `BuildSpec` and raw font data.
///
/// `primary_font_data` and `fallback_font_data` are the raw TTF/OTF bytes.
/// The caller (CLI or WASM) is responsible for reading files / fetching URLs.
pub fn build_from_config(
    spec: &BuildSpec,
    primary_font_data: &[u8],
    fallback_font_data: &[(&[u8], String)],
) -> Result<BuildResult, FontbakeError> {
    // --- Validate ---
    let (df_color, df_scale, df_spread) = match spec.effects.first() {
        Some(EffectSpec::DistanceField {
            color,
            scale,
            spread,
        }) => {
            let c = DistanceFieldConfig::parse_color(color)?;
            (*c.first().unwrap(), *c.get(1).unwrap(), *c.get(2).unwrap());
            (
                DistanceFieldConfig::parse_color(color)?,
                *scale,
                *spread,
            )
        }
        None => {
            return Err(FontbakeError::Config(
                "no effects specified; v1 requires DistanceFieldEffect".into(),
            ));
        }
    };

    // --- Load fonts ---
    let primary = OutlineFont::load(primary_font_data, spec.font_name.clone())?;
    let mut fallbacks: Vec<OutlineFont> = Vec::new();
    for (data, name) in fallback_font_data {
        fallbacks.push(OutlineFont::load(data, name.clone())?);
    }

    let font_chain: Vec<&OutlineFont> = std::iter::once(&primary)
        .chain(fallbacks.iter())
        .collect();

    // --- Resolve codepoints ---
    let size_px = spec.font_size as f32;
    let codepoints: Vec<char> = spec.glyph_text.chars().collect();

    // Java AWT FontMetrics: ascent = round(hhea_ascender * size / upem)
    // descent = round(abs(hhea_descender) * size / upem)
    // leading = round(hhea_lineGap * size / upem)
    let scale_1x = size_px / primary.units_per_em as f32;
    let base = (primary.ascender() as f32 * scale_1x).round() as u32;
    let descent = (primary.descender().unsigned_abs() as f32 * scale_1x).round() as u32;
    let leading = (primary.line_gap() as f32 * scale_1x).round() as u32;

    let resolved_glyphs: Vec<ResolvedGlyphPlan> = codepoints
        .iter()
        .copied()
        .map(|cp| resolve_glyph_plan(&font_chain, cp))
        .collect();

    // Load skrifa hinted fonts for each font in the chain.
    // Used for hinted vertical bounds that match Java AWT's getGlyphPixelBounds.
    let hinted_fonts: Vec<Option<HintedFont<'_>>> = {
        let mut fts = Vec::new();
        fts.push(HintedFont::load(primary_font_data, spec.font_size as u32).ok());
        for (data, _name) in fallback_font_data {
            fts.push(HintedFont::load(data, spec.font_size as u32).ok());
        }
        fts
    };

    let mut glyphs: Vec<GlyphRecord> = Vec::with_capacity(resolved_glyphs.len());

    for resolved in &resolved_glyphs {
        let cp = resolved.codepoint;
        let font = font_chain[resolved.font_idx];
        let source_id = font.source_id.clone();

        // --- Measure glyph at 1x for Java-compatible layout metrics ---
        // Unhinted bounds from tiny-skia rasterization (used for horizontal metrics).
        // Then overlay hinted vertical bounds from skrifa (matches Java AWT).
        let measure = rasterize_glyph(font, resolved.glyph_id, size_px, 1)?;
        let measure = {
            let hinted = hinted_fonts
                .get(resolved.font_idx)
                .and_then(|ft| ft.as_ref())
                .and_then(|ft| ft.hinted_vertical_bounds(cp as u32).ok().flatten());

            match (measure, hinted) {
                (Some(mut m), Some(hb)) => {
                    m.bearing_y = hb.y;
                    m.height = hb.height;
                    Some(m)
                }
                (m, _) => m,
            }
        };

        let mut rec = GlyphRecord::new(cp as u32, SourceKind::Outline, source_id);
        let raw_adv = advance_width_px(font, resolved.glyph_id, size_px);

        match measure {
            Some(measure) => {
                let pad_l = spec.padding.left.max(0) as u32;
                let pad_t = spec.padding.top.max(0) as u32;

                let scale_1x = size_px / font.units_per_em as f32;
                let advance_px =
                    font.advance_width(resolved.glyph_id).unwrap_or(0) as f32 * scale_1x;
                let lsb_px = font.left_side_bearing(resolved.glyph_id).unwrap_or(0) as f32 * scale_1x;
                let layout = compute_java_layout(
                    measure.bearing_x,
                    measure.bearing_y,
                    measure.width,
                    measure.height,
                    lsb_px,
                    advance_px,
                    base,
                    spec.padding.left,
                    spec.padding.right,
                    spec.padding.top,
                    spec.padding.bottom,
                );

                let layout_mask = rasterize_glyph_in_layout(
                    font,
                    resolved.glyph_id,
                    size_px,
                    df_scale,
                    layout.width,
                    layout.height,
                    measure.bearing_x,
                    measure.bearing_y,
                    pad_l,
                    pad_t,
                )?
                .ok_or_else(|| FontbakeError::FontLoad(format!(
                    "glyph U+{:04X} unexpectedly disappeared during layout rasterization",
                    cp as u32
                )))?;

                let mask_w = layout
                    .width
                    .checked_mul(df_scale)
                    .ok_or_else(|| FontbakeError::Pack("layout mask width overflow".into()))?;
                let mask_h = layout
                    .height
                    .checked_mul(df_scale)
                    .ok_or_else(|| FontbakeError::Pack("layout mask height overflow".into()))?;

                let config = DistanceFieldConfig {
                    scale: df_scale,
                    spread: df_spread,
                    color: df_color,
                };
                let (sdf_rgba, sdf_w, sdf_h) =
                    generate_distance_field(&layout_mask, mask_w, mask_h, &config)?;
                if sdf_w != layout.width || sdf_h != layout.height {
                    return Err(FontbakeError::Pack(format!(
                        "glyph U+{:04X} layout/SDF size mismatch: layout={}x{}, sdf={}x{}",
                        cp as u32, layout.width, layout.height, sdf_w, sdf_h
                    )));
                }

                rec.bitmap_rgba = sdf_rgba;
                rec.width = layout.width;
                rec.height = layout.height;
                rec.xoffset = layout.xoffset;
                rec.yoffset = layout.yoffset;
                rec.xadvance =
                    raw_adv + spec.advance_adjust.x + spec.padding.left + spec.padding.right;
            }
            None => {
                rec.width = 0;
                rec.height = 0;
                rec.xoffset = -spec.padding.left;
                rec.yoffset = 0;
                rec.xadvance =
                    raw_adv + spec.advance_adjust.x + spec.padding.left + spec.padding.right;
            }
        }

        // --- Kerning ---
        if !resolved.missing {
            for other in &resolved_glyphs {
                if other.codepoint == cp || !should_emit_kerning(*resolved, *other) {
                    continue;
                }
                let kern = font.kern(resolved.glyph_id, other.glyph_id);
                if kern != 0 {
                    let scale = size_px / font.units_per_em as f32;
                    let kern_px = (kern as f32 * scale).round() as i16;
                    if kern_px != 0 {
                        rec.kernings.push((other.codepoint as u32, kern_px));
                    }
                }
            }
        }

        glyphs.push(rec);
    }

    // --- Pack ---
    let pages = pack_glyphs(&mut glyphs, spec.page_width, spec.page_height)?;

    // --- Export ---
    // Java Hiero: lineHeight = descent + ascent + leading + padTop + padBottom + advY
    let lh = (descent as i32 + base as i32 + leading as i32
        + spec.padding.top + spec.padding.bottom
        + spec.advance_adjust.y) as u32;

    let page_filenames: Vec<String> = (0..pages.len())
        .map(|i| format!("{}{}.png", spec.font_name, if i == 0 { String::new() } else { format!("_{i}") }))
        .collect();

    let fnt_text = glyphs_to_fnt(
        &glyphs,
        &spec.font_name,
        spec.font_size as i32,
        lh,
        base,
        spec.page_width,
        spec.page_height,
        &page_filenames,
        [
            spec.padding.top,
            spec.padding.right,
            spec.padding.bottom,
            spec.padding.left,
        ],
        [spec.advance_adjust.x as i32, spec.advance_adjust.y as i32],
    );

    let page_pngs: Vec<Vec<u8>> = pages
        .iter()
        .map(|p| encode_atlas_png(p))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(BuildResult {
        fnt_text,
        page_pngs,
        glyphs,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JavaGlyphLayout {
    width: u32,
    height: u32,
    xoffset: i32,
    yoffset: i32,
}

fn compute_java_layout(
    bounds_x: i32,
    bounds_y: i32,
    bounds_w: u32,
    bounds_h: u32,
    left_side_bearing_px: f32,
    advance_px: f32,
    base: u32,
    pad_left: i32,
    pad_right: i32,
    pad_top: i32,
    pad_bottom: i32,
) -> JavaGlyphLayout {
    if bounds_w == 0 || bounds_h == 0 {
        return JavaGlyphLayout {
            width: 0,
            height: 0,
            xoffset: -pad_left,
            yoffset: 0,
        };
    }

    // Match Glyph.java: int lsb = (int)metrics.getLSB(); if (lsb > 0) lsb = 0;
    let mut lsb = left_side_bearing_px as i32;
    if lsb > 0 {
        lsb = 0;
    }

    // GlyphMetrics.getRSB() is effectively advance - lsb - pixelBounds.width.
    let mut rsb = (advance_px - left_side_bearing_px - bounds_w as f32) as i32;
    if rsb > 0 {
        rsb = 0;
    }

    let glyph_width = (bounds_w as i32 - lsb - rsb).max(0) as u32;
    let width = glyph_width + pad_left.max(0) as u32 + pad_right.max(0) as u32;
    let height = bounds_h + pad_top.max(0) as u32 + pad_bottom.max(0) as u32;

    JavaGlyphLayout {
        width,
        height,
        xoffset: bounds_x - pad_left,
        yoffset: base as i32 + bounds_y - pad_top,
    }
}
