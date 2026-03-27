//! Text BMFont (.fnt) exporter.
//!
//! Produces the AngelCode BMFont text format from internal model types.
//! Field order and formatting match Hiero's output for diff-friendliness.

use std::fmt::Write;

use crate::model::{
    AtlasPage, BmFont, BmFontChar, BmFontCommon, BmFontInfo, BmFontKerning,
    BmFontPage, FontbakeError, GlyphRecord,
};

/// Export a [`BmFont`] structure to text `.fnt` format.
pub fn export_fnt(bmfont: &BmFont) -> String {
    let mut out = String::with_capacity(4096);

    // info line
    write_info(&mut out, &bmfont.info);
    // common line
    write_common(&mut out, &bmfont.common);
    // page lines
    for p in &bmfont.pages {
        writeln!(out, "page id={} file=\"{}\"", p.id, p.file).unwrap();
    }
    // chars count
    writeln!(out, "chars count={}", bmfont.chars.len()).unwrap();
    // char lines
    for ch in &bmfont.chars {
        write_char(&mut out, ch);
    }
    // kernings
    if !bmfont.kernings.is_empty() {
        writeln!(out, "kernings count={}", bmfont.kernings.len()).unwrap();
        for k in &bmfont.kernings {
            write_kerning(&mut out, k);
        }
    } else {
        writeln!(out, "kernings count=0").unwrap();
    }

    out
}

/// Build a [`BmFont`] from a set of packed [`GlyphRecord`]s and metadata,
/// then export to text `.fnt` format.
///
/// `page_filenames` maps page index → filename (e.g. `"output_0.png"`).
pub fn glyphs_to_fnt(
    glyphs: &[GlyphRecord],
    face: &str,
    font_size: i32,
    line_height: u32,
    base: u32,
    page_width: u32,
    page_height: u32,
    page_filenames: &[String],
    padding: [i32; 4],
    spacing: [i32; 2],
) -> String {
    let bmfont = BmFont {
        info: BmFontInfo {
            face: face.to_string(),
            size: font_size,
            bold: false,
            italic: false,
            charset: String::new(),
            unicode: false,
            stretch_h: 100,
            smooth: true,
            aa: 1,
            padding,
            spacing,
        },
        common: BmFontCommon {
            line_height,
            base,
            scale_w: page_width,
            scale_h: page_height,
            pages: page_filenames.len() as u32,
            packed: false,
        },
        pages: page_filenames
            .iter()
            .enumerate()
            .map(|(i, f)| BmFontPage {
                id: i as u32,
                file: f.clone(),
            })
            .collect(),
        chars: glyphs
            .iter()
            .map(|g| BmFontChar {
                id: g.codepoint,
                x: g.x,
                y: g.y,
                width: g.width,
                height: g.height,
                xoffset: g.xoffset,
                yoffset: g.yoffset,
                xadvance: g.xadvance,
                page: g.page,
                chnl: 0,
            })
            .collect(),
        kernings: glyphs
            .iter()
            .flat_map(|g| {
                g.kernings.iter().map(move |(second, amount)| BmFontKerning {
                    first: g.codepoint,
                    second: *second,
                    amount: *amount,
                })
            })
            .collect(),
    };

    export_fnt(&bmfont)
}

/// Encode an [`AtlasPage`] as a PNG byte buffer.
pub fn encode_atlas_png(page: &AtlasPage) -> Result<Vec<u8>, FontbakeError> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, page.width, page.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| FontbakeError::Png(e.to_string()))?;
        writer
            .write_image_data(&page.rgba)
            .map_err(|e| FontbakeError::Png(e.to_string()))?;
    }
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Formatting helpers — match Hiero's output style
// ---------------------------------------------------------------------------

fn write_info(out: &mut String, info: &BmFontInfo) {
    write!(
        out,
        "info face=\"{}\" size={} bold={} italic={} charset=\"{}\" unicode={} stretchH={} smooth={} aa={} padding={},{},{},{} spacing={},{}\n",
        info.face,
        info.size,
        info.bold as u32,
        info.italic as u32,
        info.charset,
        info.unicode as u32,
        info.stretch_h,
        info.smooth as u32,
        info.aa,
        info.padding[0], info.padding[1], info.padding[2], info.padding[3],
        info.spacing[0], info.spacing[1],
    )
    .unwrap();
}

fn write_common(out: &mut String, common: &BmFontCommon) {
    writeln!(
        out,
        "common lineHeight={} base={} scaleW={} scaleH={} pages={} packed={}",
        common.line_height,
        common.base,
        common.scale_w,
        common.scale_h,
        common.pages,
        common.packed as u32,
    )
    .unwrap();
}

fn write_char(out: &mut String, ch: &BmFontChar) {
    // Hiero uses fixed-width columns with tab-like spacing. We use simple
    // padding to keep diffs readable.
    writeln!(
        out,
        "char id={:<7} x={:<5} y={:<5} width={:<5} height={:<5} xoffset={:<4} yoffset={:<4} xadvance={:<4} page={:<4} chnl={}",
        ch.id, ch.x, ch.y, ch.width, ch.height,
        ch.xoffset, ch.yoffset, ch.xadvance, ch.page, ch.chnl,
    )
    .unwrap();
}

fn write_kerning(out: &mut String, k: &BmFontKerning) {
    writeln!(
        out,
        "kerning first={} second={} amount={}",
        k.first, k.second, k.amount,
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::bmfont_text::parse_fnt;

    /// Roundtrip: parse → export → re-parse should yield identical structures.
    #[test]
    fn roundtrip_fnt() {
        let original = concat!(
            "info face=\"HUN2\" size=52 bold=0 italic=0 charset=\"\" unicode=0 stretchH=100 smooth=1 aa=1 padding=4,4,4,4 spacing=-8,-8\n",
            "common lineHeight=52 base=40 scaleW=512 scaleH=512 pages=1 packed=0\n",
            "page id=0 file=\"hun.png\"\n",
            "chars count=2\n",
            "char id=32      x=0    y=0    width=0    height=0    xoffset=-4   yoffset=0    xadvance=18   page=0    chnl=0 \n",
            "char id=65      x=95   y=49   width=36   height=48   xoffset=-1   yoffset=-4   xadvance=34   page=0    chnl=0 \n",
            "kernings count=0\n",
        );

        let parsed = parse_fnt(original).unwrap();
        let exported = export_fnt(&parsed);
        let reparsed = parse_fnt(&exported).unwrap();

        // Structural equality
        assert_eq!(parsed.info.face, reparsed.info.face);
        assert_eq!(parsed.info.size, reparsed.info.size);
        assert_eq!(parsed.info.padding, reparsed.info.padding);
        assert_eq!(parsed.info.spacing, reparsed.info.spacing);
        assert_eq!(parsed.common.line_height, reparsed.common.line_height);
        assert_eq!(parsed.common.base, reparsed.common.base);
        assert_eq!(parsed.common.scale_w, reparsed.common.scale_w);
        assert_eq!(parsed.pages.len(), reparsed.pages.len());
        assert_eq!(parsed.pages[0].file, reparsed.pages[0].file);
        assert_eq!(parsed.chars.len(), reparsed.chars.len());
        for (a, b) in parsed.chars.iter().zip(reparsed.chars.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
            assert_eq!(a.width, b.width);
            assert_eq!(a.height, b.height);
            assert_eq!(a.xoffset, b.xoffset);
            assert_eq!(a.yoffset, b.yoffset);
            assert_eq!(a.xadvance, b.xadvance);
            assert_eq!(a.page, b.page);
        }
    }

    #[test]
    fn encode_atlas_png_produces_valid_png() {
        let page = AtlasPage::new(4, 4);
        let bytes = encode_atlas_png(&page).unwrap();
        // PNG magic bytes
        assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
    }
}
