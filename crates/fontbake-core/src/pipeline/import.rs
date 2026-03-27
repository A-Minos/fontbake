//! `import_bmfont` pipeline — import an existing text BMFont (.fnt + PNG pages)
//! into internal `GlyphRecord`s.
//!
//! This is the entry point for the "import existing atlas" workflow.
//! Imported glyphs are pixel-passthrough — no re-rasterization, no SDF.

use crate::model::{BmFont, FontbakeError, GlyphRecord};
use crate::source::bmfont_text::{bmfont_to_glyphs, parse_fnt};

/// Decoded PNG page: (width, height, RGBA bytes).
pub struct PngPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Import a text BMFont from its `.fnt` content and decoded PNG pages.
///
/// `fnt_text` is the raw text content of the `.fnt` file.
/// `pages` must be ordered by page id (index 0 = page 0, etc.).
///
/// Returns the parsed `BmFont` structure and the extracted `GlyphRecord`s.
pub fn import_bmfont(
    fnt_text: &str,
    pages: &[PngPage],
    source_id: &str,
) -> Result<(BmFont, Vec<GlyphRecord>), FontbakeError> {
    let bmfont = parse_fnt(fnt_text)?;

    let page_data: Vec<(u32, u32, &[u8])> = pages
        .iter()
        .map(|p| (p.width, p.height, p.rgba.as_slice()))
        .collect();

    let glyphs = bmfont_to_glyphs(&bmfont, &page_data, source_id)?;

    Ok((bmfont, glyphs))
}

/// Decode a PNG byte buffer into `PngPage`.
pub fn decode_png_page(png_bytes: &[u8]) -> Result<PngPage, FontbakeError> {
    let mut decoder = png::Decoder::new(png_bytes);
    // Expand palette (Indexed) to RGB/RGBA and expand 1/2/4-bit grayscale to 8-bit.
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::ALPHA,
    );
    let mut reader = decoder
        .read_info()
        .map_err(|e| FontbakeError::Png(e.to_string()))?;

    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| FontbakeError::Png(e.to_string()))?;

    let width = info.width;
    let height = info.height;

    // After EXPAND+ALPHA transforms, indexed becomes RGBA, RGB stays RGB, etc.
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => {
            let src = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for chunk in src.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            let src = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for &g in src {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let src = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for chunk in src.chunks(2) {
                let g = chunk[0];
                let a = chunk.get(1).copied().unwrap_or(255);
                rgba.extend_from_slice(&[g, g, g, a]);
            }
            rgba
        }
        // Indexed is fully expanded to RGB or RGBA by the EXPAND transform above.
        // If somehow still indexed, treat each byte as gray.
        png::ColorType::Indexed => {
            let src = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for &idx in src {
                rgba.extend_from_slice(&[idx, idx, idx, 255]);
            }
            rgba
        }
    };

    Ok(PngPage {
        width,
        height,
        rgba,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::bmfont_text::encode_atlas_png;
    use crate::model::AtlasPage;

    #[test]
    fn decode_roundtrip_png() {
        let page = AtlasPage::new(4, 4);
        let png_bytes = encode_atlas_png(&page).unwrap();
        let decoded = decode_png_page(&png_bytes).unwrap();
        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(decoded.rgba.len(), 4 * 4 * 4);
    }

    #[test]
    fn import_minimal_bmfont() {
        let fnt = concat!(
            "info face=\"T\" size=12 bold=0 italic=0 charset=\"\" unicode=0 stretchH=100 smooth=1 aa=1 padding=0,0,0,0 spacing=0,0\n",
            "common lineHeight=12 base=10 scaleW=4 scaleH=4 pages=1 packed=0\n",
            "page id=0 file=\"t.png\"\n",
            "chars count=1\n",
            "char id=65 x=0 y=0 width=2 height=2 xoffset=0 yoffset=0 xadvance=3 page=0 chnl=0\n",
            "kernings count=0\n",
        );
        let page = PngPage {
            width: 4,
            height: 4,
            rgba: vec![255u8; 4 * 4 * 4],
        };
        let (bmfont, glyphs) = import_bmfont(fnt, &[page], "test").unwrap();
        assert_eq!(bmfont.chars.len(), 1);
        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].codepoint, 65);
        assert_eq!(glyphs[0].width, 2);
        assert_eq!(glyphs[0].height, 2);
        // Bitmap should be 2x2 RGBA = 16 bytes
        assert_eq!(glyphs[0].bitmap_rgba.len(), 2 * 2 * 4);
    }
}
