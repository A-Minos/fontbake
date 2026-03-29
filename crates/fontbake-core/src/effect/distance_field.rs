//! Distance Field Effect — port of libGDX `DistanceFieldGenerator`.
//!
//! Takes a high-resolution binary/alpha mask and produces a signed distance
//! field image at the original (non-upscaled) resolution.
//!
//! Algorithm:
//! 1. For each pixel in the output, sample a neighbourhood of `spread` pixels
//!    in the high-res mask.
//! 2. Find the minimum distance to the nearest edge (inside→outside or
//!    outside→inside).
//! 3. Map the signed distance to [0, 255] where 128 = on the edge.
//!
//! Reference: libGDX `DistanceFieldGenerator.java`

use crate::model::FontbakeError;

struct RowIndex {
    inside_start: usize,
    inside_len: usize,
    outside_start: usize,
    outside_len: usize,
}

struct RowIndexTable {
    rows: Vec<RowIndex>,
    xs: Vec<u32>,
}

/// Configuration for the distance field generator.
pub struct DistanceFieldConfig {
    /// Upscale factor — the mask was rendered at `scale` × the output size.
    pub scale: u32,
    /// Spread radius in output pixels. Determines how far the distance field
    /// extends from the edge.
    pub spread: f32,
    /// Color to apply (RGB hex, e.g. "ffffff"). The output RGBA will use this
    /// color for RGB and the distance value for alpha.
    pub color: [u8; 3],
}

impl DistanceFieldConfig {
    /// Parse a hex color string like "ffffff" into [R, G, B].
    pub fn parse_color(hex: &str) -> Result<[u8; 3], FontbakeError> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Err(FontbakeError::Config(format!(
                "invalid color hex: {hex} (expected 6 hex digits)"
            )));
        }
        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| FontbakeError::Config(format!("invalid color hex: {hex}")))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| FontbakeError::Config(format!("invalid color hex: {hex}")))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| FontbakeError::Config(format!("invalid color hex: {hex}")))?;
        Ok([r, g, b])
    }
}

/// Generate a distance field from a high-resolution alpha mask.
///
/// # Arguments
/// - `mask` — 8-bit alpha values, row-major, `mask_w × mask_h`.
/// - `mask_w`, `mask_h` — dimensions of the high-res mask.
/// - `config` — distance field parameters.
///
/// # Returns
/// `(rgba, out_w, out_h)` — RGBA pixel data at 1/scale resolution.
pub fn generate_distance_field(
    mask: &[u8],
    mask_w: u32,
    mask_h: u32,
    config: &DistanceFieldConfig,
) -> Result<(Vec<u8>, u32, u32), FontbakeError> {
    let scale = config.scale.max(1);
    let out_w = mask_w / scale;
    let out_h = mask_h / scale;

    if out_w == 0 || out_h == 0 {
        return Ok((vec![], 0, 0));
    }

    let spread_scaled = config.spread * scale as f32;
    let [cr, cg, cb] = config.color;
    let row_index = build_row_index(mask, mask_w, mask_h);

    let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];

    for oy in 0..out_h {
        let cy = oy * scale + scale / 2;
        for ox in 0..out_w {
            let cx = ox * scale + scale / 2;
            let inside = sample_mask(mask, mask_w, mask_h, cx as f32, cy as f32) >= 128;
            let min_dist =
                find_min_edge_distance(&row_index, mask_w, mask_h, cx, cy, spread_scaled, inside);

            let signed_dist = if inside { min_dist } else { -min_dist };
            let normalised = 0.5 + 0.5 * (signed_dist / spread_scaled);
            let alpha = (normalised.clamp(0.0, 1.0) * 255.0) as u8;

            let idx = ((oy * out_w + ox) * 4) as usize;
            rgba[idx] = cr;
            rgba[idx + 1] = cg;
            rgba[idx + 2] = cb;
            rgba[idx + 3] = alpha;
        }
    }

    Ok((rgba, out_w, out_h))
}

/// Sample the mask at a floating-point position (nearest-neighbour).
fn sample_mask(mask: &[u8], w: u32, h: u32, x: f32, y: f32) -> u8 {
    let ix = (x as u32).min(w.saturating_sub(1));
    let iy = (y as u32).min(h.saturating_sub(1));
    mask[(iy * w + ix) as usize]
}

fn build_row_index(mask: &[u8], mask_w: u32, mask_h: u32) -> RowIndexTable {
    let mut rows = Vec::with_capacity(mask_h as usize);
    let mut xs = Vec::with_capacity((mask_w * mask_h) as usize);
    for y in 0..mask_h {
        let row_start = (y * mask_w) as usize;
        let row_end = row_start + mask_w as usize;

        let inside_start = xs.len();
        for (x, &val) in mask[row_start..row_end].iter().enumerate() {
            if val >= 128 {
                xs.push(x as u32);
            }
        }
        let inside_len = xs.len() - inside_start;

        let outside_start = xs.len();
        for (x, &val) in mask[row_start..row_end].iter().enumerate() {
            if val < 128 {
                xs.push(x as u32);
            }
        }
        let outside_len = xs.len() - outside_start;

        rows.push(RowIndex {
            inside_start,
            inside_len,
            outside_start,
            outside_len,
        });
    }
    RowIndexTable { rows, xs }
}

fn find_min_edge_distance(
    row_index: &RowIndexTable,
    w: u32,
    h: u32,
    cx: u32,
    cy: u32,
    spread: f32,
    inside: bool,
) -> f32 {
    let delta = spread.ceil() as u32;
    let mut min_dist_sq = (delta * delta) as f32;

    let x0 = cx.saturating_sub(delta);
    let y0 = cy.saturating_sub(delta);
    let x1 = (cx + delta).min(w.saturating_sub(1));
    let y1 = (cy + delta).min(h.saturating_sub(1));

    for sy in y0..=y1 {
        let dy = sy as f32 - cy as f32;
        let dy_sq = dy * dy;
        if dy_sq >= min_dist_sq {
            continue;
        }

        let row = &row_index.rows[sy as usize];
        let (xs_start, xs_len) = if inside {
            (row.outside_start, row.outside_len)
        } else {
            (row.inside_start, row.inside_len)
        };
        let xs = &row_index.xs[xs_start..xs_start + xs_len];
        if xs.is_empty() {
            continue;
        }

        let insert_at = xs.partition_point(|&sx| sx < cx);
        let try_candidate = |sx: u32, min_dist_sq: &mut f32| {
            if sx < x0 || sx > x1 {
                return;
            }
            let dx = sx as f32 - cx as f32;
            let dist_sq = dx * dx + dy_sq;
            if dist_sq < *min_dist_sq {
                *min_dist_sq = dist_sq;
            }
        };

        if let Some(&sx) = xs.get(insert_at) {
            try_candidate(sx, &mut min_dist_sq);
        }
        if insert_at > 0 {
            try_candidate(xs[insert_at - 1], &mut min_dist_sq);
        }
    }

    min_dist_sq.sqrt().min(spread)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn brute_force_find_min_edge_distance(
        mask: &[u8],
        w: u32,
        h: u32,
        cx: u32,
        cy: u32,
        spread: f32,
        inside: bool,
    ) -> f32 {
        let delta = spread.ceil();
        let mut min_dist_sq = delta * delta;
        let delta_i = delta as u32;
        let x0 = cx.saturating_sub(delta_i);
        let y0 = cy.saturating_sub(delta_i);
        let x1 = (cx + delta_i).min(w.saturating_sub(1));
        let y1 = (cy + delta_i).min(h.saturating_sub(1));

        for sy in y0..=y1 {
            for sx in x0..=x1 {
                let val = mask[(sy * w + sx) as usize];
                let sample_inside = val >= 128;
                if sample_inside != inside {
                    let dx = sx as f32 - cx as f32;
                    let dy = sy as f32 - cy as f32;
                    let dist_sq = dx * dx + dy * dy;
                    if dist_sq < min_dist_sq {
                        min_dist_sq = dist_sq;
                    }
                }
            }
        }

        min_dist_sq.sqrt().min(spread)
    }

    fn reference_generate_distance_field(
        mask: &[u8],
        mask_w: u32,
        mask_h: u32,
        config: &DistanceFieldConfig,
    ) -> (Vec<u8>, u32, u32) {
        let scale = config.scale.max(1);
        let out_w = mask_w / scale;
        let out_h = mask_h / scale;
        if out_w == 0 || out_h == 0 {
            return (vec![], 0, 0);
        }

        let spread_scaled = config.spread * scale as f32;
        let [cr, cg, cb] = config.color;
        let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];

        for oy in 0..out_h {
            let cy = oy * scale + scale / 2;
            for ox in 0..out_w {
                let cx = ox * scale + scale / 2;
                let inside = sample_mask(mask, mask_w, mask_h, cx as f32, cy as f32) >= 128;
                let min_dist = brute_force_find_min_edge_distance(
                    mask,
                    mask_w,
                    mask_h,
                    cx,
                    cy,
                    spread_scaled,
                    inside,
                );
                let signed_dist = if inside { min_dist } else { -min_dist };
                let normalised = 0.5 + 0.5 * (signed_dist / spread_scaled);
                let alpha = (normalised.clamp(0.0, 1.0) * 255.0) as u8;
                let idx = ((oy * out_w + ox) * 4) as usize;
                rgba[idx] = cr;
                rgba[idx + 1] = cg;
                rgba[idx + 2] = cb;
                rgba[idx + 3] = alpha;
            }
        }

        (rgba, out_w, out_h)
    }

    #[test]
    fn parse_color_white() {
        let c = DistanceFieldConfig::parse_color("ffffff").unwrap();
        assert_eq!(c, [255, 255, 255]);
    }

    #[test]
    fn parse_color_red() {
        let c = DistanceFieldConfig::parse_color("ff0000").unwrap();
        assert_eq!(c, [255, 0, 0]);
    }

    #[test]
    fn parse_color_with_hash() {
        let c = DistanceFieldConfig::parse_color("#00ff00").unwrap();
        assert_eq!(c, [0, 255, 0]);
    }

    #[test]
    fn parse_color_invalid() {
        assert!(DistanceFieldConfig::parse_color("fff").is_err());
    }

    #[test]
    fn empty_mask_returns_empty() {
        let config = DistanceFieldConfig {
            scale: 4,
            spread: 3.5,
            color: [255, 255, 255],
        };
        // 3x3 mask / scale=4 → 0x0 output
        let (rgba, w, h) = generate_distance_field(&[0; 9], 3, 3, &config).unwrap();
        assert_eq!(w, 0);
        assert_eq!(h, 0);
        assert!(rgba.is_empty());
    }

    #[test]
    fn solid_white_mask_produces_inside_values() {
        let scale = 2u32;
        let mask_w = 8u32;
        let mask_h = 8u32;
        let mask = vec![255u8; (mask_w * mask_h) as usize];
        let config = DistanceFieldConfig {
            scale,
            spread: 2.0,
            color: [255, 255, 255],
        };
        let (rgba, w, h) = generate_distance_field(&mask, mask_w, mask_h, &config).unwrap();
        assert_eq!(w, 4);
        assert_eq!(h, 4);
        // All pixels should be "inside" with high alpha (≥ 128)
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4 + 3) as usize;
                assert!(
                    rgba[idx] >= 128,
                    "pixel ({x},{y}) alpha={} < 128",
                    rgba[idx]
                );
            }
        }
    }

    #[test]
    fn solid_black_mask_produces_outside_values() {
        let scale = 2u32;
        let mask_w = 8u32;
        let mask_h = 8u32;
        let mask = vec![0u8; (mask_w * mask_h) as usize];
        let config = DistanceFieldConfig {
            scale,
            spread: 2.0,
            color: [255, 255, 255],
        };
        let (rgba, w, h) = generate_distance_field(&mask, mask_w, mask_h, &config).unwrap();
        assert_eq!(w, 4);
        assert_eq!(h, 4);
        // All pixels should be "outside" with low alpha (< 128)
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4 + 3) as usize;
                assert!(
                    rgba[idx] < 128,
                    "pixel ({x},{y}) alpha={} >= 128",
                    rgba[idx]
                );
            }
        }
    }

    #[test]
    fn edge_mask_produces_near_128_alpha() {
        // Left half white, right half black — 8x4 mask, scale=2 → 4x2 output
        let scale = 2u32;
        let mask_w = 8u32;
        let mask_h = 4u32;
        let mut mask = vec![0u8; (mask_w * mask_h) as usize];
        for y in 0..mask_h {
            for x in 0..mask_w / 2 {
                mask[(y * mask_w + x) as usize] = 255;
            }
        }
        let config = DistanceFieldConfig {
            scale,
            spread: 4.0,
            color: [255, 255, 255],
        };
        let (rgba, _w, _h) = generate_distance_field(&mask, mask_w, mask_h, &config).unwrap();
        // Pixel at x=1 (near left side, inside) should have alpha > 128
        let inside_alpha = rgba[(1 * 4 + 3) as usize];
        assert!(inside_alpha >= 128);
        // Pixel at x=2 (near right side, outside) should have alpha < 128
        let outside_alpha = rgba[(2 * 4 + 3) as usize];
        assert!(outside_alpha < 128);
    }

    fn assert_matches_reference(mask: &[u8], mask_w: u32, mask_h: u32, config: &DistanceFieldConfig) {
        let expected = reference_generate_distance_field(mask, mask_w, mask_h, config);
        let actual = generate_distance_field(mask, mask_w, mask_h, config).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn optimised_search_matches_reference_output() {
        let mask_w = 13u32;
        let mask_h = 11u32;
        let mut mask = vec![0u8; (mask_w * mask_h) as usize];
        for y in 1..10 {
            mask[(y * mask_w + 2) as usize] = 255;
        }
        for x in 2..11 {
            mask[(5 * mask_w + x) as usize] = 255;
        }
        mask[(2 * mask_w + 9) as usize] = 255;
        mask[(8 * mask_w + 8) as usize] = 255;

        let config = DistanceFieldConfig {
            scale: 3,
            spread: 2.5,
            color: [0x12, 0x34, 0x56],
        };

        assert_matches_reference(&mask, mask_w, mask_h, &config);
    }

    #[test]
    fn optimised_search_matches_reference_for_ring_shape() {
        let mask_w = 16u32;
        let mask_h = 16u32;
        let mut mask = vec![0u8; (mask_w * mask_h) as usize];
        for y in 2..14 {
            for x in 2..14 {
                mask[(y * mask_w + x) as usize] = 255;
            }
        }
        for y in 5..11 {
            for x in 5..11 {
                mask[(y * mask_w + x) as usize] = 0;
            }
        }

        let config = DistanceFieldConfig {
            scale: 4,
            spread: 3.5,
            color: [0xaa, 0xbb, 0xcc],
        };

        assert_matches_reference(&mask, mask_w, mask_h, &config);
    }

    #[test]
    fn optimised_search_matches_reference_for_sparse_diagonal() {
        let mask_w = 15u32;
        let mask_h = 15u32;
        let mut mask = vec![0u8; (mask_w * mask_h) as usize];
        for i in 1..14 {
            mask[(i * mask_w + i) as usize] = 255;
            if i + 1 < 15 {
                mask[(i * mask_w + (i + 1)) as usize] = 255;
            }
        }

        let config = DistanceFieldConfig {
            scale: 3,
            spread: 4.0,
            color: [0x20, 0x40, 0x60],
        };

        assert_matches_reference(&mask, mask_w, mask_h, &config);
    }

    #[test]
    fn build_row_index_packs_positions_into_single_contiguous_buffer() {
        let mask_w = 8u32;
        let mask_h = 4u32;
        let mask = vec![
            255, 255, 255, 255, 255, 255, 255, 0,
            0, 0, 0, 0, 255, 0, 0, 0,
            255, 0, 255, 0, 255, 0, 255, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
        ];

        let table = build_row_index(&mask, mask_w, mask_h);
        let stored_positions: usize = table
            .rows
            .iter()
            .map(|row| row.inside_len + row.outside_len)
            .sum();

        assert_eq!(stored_positions, mask.len());
        assert_eq!(table.xs.len(), mask.len());
    }
}
