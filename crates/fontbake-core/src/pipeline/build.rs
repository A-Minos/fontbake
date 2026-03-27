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
use crate::raster::java_shape::{advance_width_px, ascender_px, line_height_px, rasterize_glyph};
use crate::source::outline::{resolve_codepoint, OutlineFont};

/// Named font data for the pipeline — bytes + identifier.
pub struct FontAsset<'a> {
    pub data: &'a [u8],
    pub name: String,
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

    let mut glyphs: Vec<GlyphRecord> = Vec::with_capacity(codepoints.len());

    for &cp in &codepoints {
        let (font_idx, glyph_id) = match resolve_codepoint(&font_chain, cp) {
            Some(r) => r,
            None => {
                // No font has this glyph — create an empty record
                let mut rec =
                    GlyphRecord::new(cp as u32, SourceKind::Outline, "missing".into());
                rec.xadvance = 0;
                glyphs.push(rec);
                continue;
            }
        };

        let font = font_chain[font_idx];
        let source_id = font.source_id.clone();

        // --- Rasterize ---
        let raster = rasterize_glyph(font, glyph_id, size_px, df_scale)?;

        let mut rec = GlyphRecord::new(cp as u32, SourceKind::Outline, source_id);

        match raster {
            Some(r) => {
                // --- Apply DistanceFieldEffect ---
                let config = DistanceFieldConfig {
                    scale: df_scale,
                    spread: df_spread,
                    color: df_color,
                };
                let (rgba, out_w, out_h) =
                    generate_distance_field(&r.mask, r.width, r.height, &config)?;

                // Compute offsets: bearing adjusted for downscale + padding
                let bearing_x = r.bearing_x / df_scale as i32;
                let bearing_y = r.bearing_y / df_scale as i32;

                rec.bitmap_rgba = rgba;
                rec.width = out_w;
                rec.height = out_h;
                rec.xoffset = bearing_x + spec.padding.left;
                rec.yoffset = bearing_y + spec.padding.top;
                rec.xadvance = advance_width_px(font, glyph_id, size_px)
                    + spec.advance_adjust.x;
            }
            None => {
                // Empty glyph (e.g. space)
                rec.xadvance = advance_width_px(font, glyph_id, size_px)
                    + spec.advance_adjust.x;
            }
        }

        // --- Kerning ---
        for &other_cp in &codepoints {
            if other_cp == cp {
                continue;
            }
            if let Some(other_gid) = font.glyph_id(other_cp) {
                let kern = font.kern(glyph_id, other_gid);
                if kern != 0 {
                    let scale = size_px / font.units_per_em as f32;
                    let kern_px = (kern as f32 * scale).round() as i16;
                    if kern_px != 0 {
                        rec.kernings.push((other_cp as u32, kern_px));
                    }
                }
            }
        }

        glyphs.push(rec);
    }

    // --- Pack ---
    let pages = pack_glyphs(&mut glyphs, spec.page_width, spec.page_height)?;

    // --- Export ---
    let lh = line_height_px(&primary, size_px);
    let base = ascender_px(&primary, size_px) as u32;

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
