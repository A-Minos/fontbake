//! Text BMFont (.fnt) parser.
//!
//! Parses the AngelCode BMFont text format as produced by Hiero / BMFont.
//! Only the text format is supported in v1; binary BMFont v3 is deferred.
//!
//! Reference: <https://www.angelcode.com/products/bmfont/doc/file_format.html>

use crate::model::{
    BmFont, BmFontChar, BmFontCommon, BmFontInfo, BmFontKerning, BmFontPage, FontbakeError,
    GlyphRecord, SourceKind,
};

/// Parse a text `.fnt` file into a [`BmFont`] structure.
pub fn parse_fnt(input: &str) -> Result<BmFont, FontbakeError> {
    let mut info: Option<BmFontInfo> = None;
    let mut common: Option<BmFontCommon> = None;
    let mut pages: Vec<BmFontPage> = Vec::new();
    let mut chars: Vec<BmFontChar> = Vec::new();
    let mut kernings: Vec<BmFontKerning> = Vec::new();

    for (line_no, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        let (tag, rest) = split_tag(line);
        let attrs = parse_attrs(rest);

        match tag {
            "info" => {
                info = Some(BmFontInfo {
                    face: get_str(&attrs, "face"),
                    size: get_i32(&attrs, "size", line_no)?,
                    bold: get_u32(&attrs, "bold", line_no)? != 0,
                    italic: get_u32(&attrs, "italic", line_no)? != 0,
                    charset: get_str(&attrs, "charset"),
                    unicode: get_u32_or(&attrs, "unicode", 0) != 0,
                    stretch_h: get_u32_or(&attrs, "stretchH", 100),
                    smooth: get_u32_or(&attrs, "smooth", 0) != 0,
                    aa: get_u32_or(&attrs, "aa", 1),
                    padding: parse_csv_i32_4(&attrs, "padding", line_no)?,
                    spacing: parse_csv_i32_2(&attrs, "spacing", line_no)?,
                });
            }
            "common" => {
                common = Some(BmFontCommon {
                    line_height: get_u32(&attrs, "lineHeight", line_no)?,
                    base: get_u32(&attrs, "base", line_no)?,
                    scale_w: get_u32(&attrs, "scaleW", line_no)?,
                    scale_h: get_u32(&attrs, "scaleH", line_no)?,
                    pages: get_u32(&attrs, "pages", line_no)?,
                    packed: get_u32_or(&attrs, "packed", 0) != 0,
                });
            }
            "page" => {
                pages.push(BmFontPage {
                    id: get_u32(&attrs, "id", line_no)?,
                    file: get_str(&attrs, "file"),
                });
            }
            "char" => {
                chars.push(BmFontChar {
                    id: get_u32(&attrs, "id", line_no)?,
                    x: get_u32(&attrs, "x", line_no)?,
                    y: get_u32(&attrs, "y", line_no)?,
                    width: get_u32(&attrs, "width", line_no)?,
                    height: get_u32(&attrs, "height", line_no)?,
                    xoffset: get_i32(&attrs, "xoffset", line_no)?,
                    yoffset: get_i32(&attrs, "yoffset", line_no)?,
                    xadvance: get_i32(&attrs, "xadvance", line_no)?,
                    page: get_u32(&attrs, "page", line_no)?,
                    chnl: get_u32_or(&attrs, "chnl", 0),
                });
            }
            "kerning" => {
                kernings.push(BmFontKerning {
                    first: get_u32(&attrs, "first", line_no)?,
                    second: get_u32(&attrs, "second", line_no)?,
                    amount: get_i32(&attrs, "amount", line_no)? as i16,
                });
            }
            // "chars count=N" and "kernings count=N" are informational; skip.
            "chars" | "kernings" => {}
            _ => {
                // Unknown tags are silently ignored for forward compatibility.
            }
        }
    }

    let info = info.ok_or_else(|| FontbakeError::BmfontParse("missing 'info' line".into()))?;
    let common =
        common.ok_or_else(|| FontbakeError::BmfontParse("missing 'common' line".into()))?;

    Ok(BmFont {
        info,
        common,
        pages,
        chars,
        kernings,
    })
}

/// Convert a parsed [`BmFont`] + atlas page pixel data into [`GlyphRecord`]s.
///
/// `page_images` maps page id → RGBA pixel buffer (width × height × 4 bytes).
/// Each glyph's bitmap is cropped from the corresponding atlas page.
pub fn bmfont_to_glyphs(
    bmfont: &BmFont,
    page_images: &[(u32, u32, &[u8])], // (width, height, rgba) per page, indexed by page id
    source_id: &str,
) -> Result<Vec<GlyphRecord>, FontbakeError> {
    let mut glyphs = Vec::with_capacity(bmfont.chars.len());

    for ch in &bmfont.chars {
        let mut rec = GlyphRecord::new(ch.id, SourceKind::BitmapImport, source_id.to_string());
        rec.width = ch.width;
        rec.height = ch.height;
        rec.xoffset = ch.xoffset;
        rec.yoffset = ch.yoffset;
        rec.xadvance = ch.xadvance;
        rec.page = ch.page;
        rec.x = ch.x;
        rec.y = ch.y;

        // Collect kerning pairs where this char is the first
        for k in &bmfont.kernings {
            if k.first == ch.id {
                rec.kernings.push((k.second, k.amount));
            }
        }

        // Crop bitmap from atlas page
        if ch.width > 0 && ch.height > 0 {
            let page_idx = ch.page as usize;
            if page_idx >= page_images.len() {
                return Err(FontbakeError::BmfontParse(format!(
                    "char id={} references page {} but only {} pages loaded",
                    ch.id,
                    ch.page,
                    page_images.len()
                )));
            }
            let (pw, _ph, pixels) = &page_images[page_idx];
            let stride = (*pw as usize) * 4;
            let mut bitmap = Vec::with_capacity((ch.width * ch.height * 4) as usize);
            for row in 0..ch.height {
                let src_y = (ch.y + row) as usize;
                let src_x = ch.x as usize;
                let start = src_y * stride + src_x * 4;
                let end = start + (ch.width as usize) * 4;
                if end > pixels.len() {
                    return Err(FontbakeError::BmfontParse(format!(
                        "char id={} rect exceeds atlas page {} bounds",
                        ch.id, ch.page
                    )));
                }
                bitmap.extend_from_slice(&pixels[start..end]);
            }
            rec.bitmap_rgba = bitmap;
        }

        glyphs.push(rec);
    }

    Ok(glyphs)
}

// ---------------------------------------------------------------------------
// Attribute parsing helpers
// ---------------------------------------------------------------------------

/// Split a line into (tag, rest). E.g. `"char id=32 x=0"` → `("char", "id=32 x=0")`.
fn split_tag(line: &str) -> (&str, &str) {
    match line.find(|c: char| c.is_whitespace()) {
        Some(pos) => (&line[..pos], line[pos..].trim_start()),
        None => (line, ""),
    }
}

/// Parse `key=value` pairs from the rest of a BMFont line.
/// Values may be quoted (`face="HUN2"`) or unquoted (`size=52`).
fn parse_attrs(input: &str) -> Vec<(&str, &str)> {
    let mut attrs = Vec::new();
    let mut remaining = input;

    while !remaining.is_empty() {
        // Skip whitespace
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        // Find `=`
        let eq_pos = match remaining.find('=') {
            Some(p) => p,
            None => break,
        };
        let key = remaining[..eq_pos].trim();
        remaining = &remaining[eq_pos + 1..];

        // Value: quoted or unquoted
        let value;
        if remaining.starts_with('"') {
            // Quoted value — find closing quote
            remaining = &remaining[1..];
            let end = remaining.find('"').unwrap_or(remaining.len());
            value = &remaining[..end];
            remaining = if end < remaining.len() {
                &remaining[end + 1..]
            } else {
                ""
            };
        } else {
            // Unquoted — ends at next whitespace
            let end = remaining
                .find(|c: char| c.is_whitespace())
                .unwrap_or(remaining.len());
            value = &remaining[..end];
            remaining = &remaining[end..];
        }

        attrs.push((key, value));
    }

    attrs
}

fn get_str(attrs: &[(&str, &str)], key: &str) -> String {
    attrs
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
        .unwrap_or_default()
}

fn get_u32(attrs: &[(&str, &str)], key: &str, line_no: usize) -> Result<u32, FontbakeError> {
    let val = attrs
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            FontbakeError::BmfontParse(format!("line {}: missing key '{key}'", line_no + 1))
        })?;
    val.parse::<u32>().map_err(|_| {
        FontbakeError::BmfontParse(format!(
            "line {}: invalid u32 for '{key}': {val}",
            line_no + 1
        ))
    })
}

fn get_u32_or(attrs: &[(&str, &str)], key: &str, default: u32) -> u32 {
    attrs
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v.parse::<u32>().ok())
        .unwrap_or(default)
}

fn get_i32(attrs: &[(&str, &str)], key: &str, line_no: usize) -> Result<i32, FontbakeError> {
    let val = attrs
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            FontbakeError::BmfontParse(format!("line {}: missing key '{key}'", line_no + 1))
        })?;
    val.parse::<i32>().map_err(|_| {
        FontbakeError::BmfontParse(format!(
            "line {}: invalid i32 for '{key}': {val}",
            line_no + 1
        ))
    })
}

/// Parse a comma-separated list of 4 i32 values, e.g. `"4,4,4,4"`.
fn parse_csv_i32_4(
    attrs: &[(&str, &str)],
    key: &str,
    line_no: usize,
) -> Result<[i32; 4], FontbakeError> {
    let raw = get_str(attrs, key);
    if raw.is_empty() {
        return Ok([0; 4]);
    }
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 4 {
        return Err(FontbakeError::BmfontParse(format!(
            "line {}: expected 4 values for '{key}', got {}",
            line_no + 1,
            parts.len()
        )));
    }
    let mut out = [0i32; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse::<i32>().map_err(|_| {
            FontbakeError::BmfontParse(format!("line {}: invalid i32 in '{key}': {p}", line_no + 1))
        })?;
    }
    Ok(out)
}

/// Parse a comma-separated list of 2 i32 values, e.g. `"-8,-8"`.
fn parse_csv_i32_2(
    attrs: &[(&str, &str)],
    key: &str,
    line_no: usize,
) -> Result<[i32; 2], FontbakeError> {
    let raw = get_str(attrs, key);
    if raw.is_empty() {
        return Ok([0; 2]);
    }
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 2 {
        return Err(FontbakeError::BmfontParse(format!(
            "line {}: expected 2 values for '{key}', got {}",
            line_no + 1,
            parts.len()
        )));
    }
    let mut out = [0i32; 2];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse::<i32>().map_err(|_| {
            FontbakeError::BmfontParse(format!("line {}: invalid i32 in '{key}': {p}", line_no + 1))
        })?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FNT: &str = r#"info face="HUN2" size=52 bold=0 italic=0 charset="" unicode=0 stretchH=100 smooth=1 aa=1 padding=4,4,4,4 spacing=-8,-8
common lineHeight=52 base=40 scaleW=512 scaleH=512 pages=1 packed=0
page id=0 file="hun.png"
chars count=3
char id=32      x=0    y=0    width=0    height=0    xoffset=-4   yoffset=0    xadvance=18   page=0    chnl=0 
char id=33      x=66   y=0    width=16   height=48   xoffset=0    yoffset=-4   xadvance=16   page=0    chnl=0 
char id=65      x=95   y=49   width=36   height=48   xoffset=-1   yoffset=-4   xadvance=34   page=0    chnl=0 
kernings count=0"#;

    #[test]
    fn parse_info_line() {
        let bmfont = parse_fnt(SAMPLE_FNT).unwrap();
        assert_eq!(bmfont.info.face, "HUN2");
        assert_eq!(bmfont.info.size, 52);
        assert!(!bmfont.info.bold);
        assert!(!bmfont.info.italic);
        assert_eq!(bmfont.info.padding, [4, 4, 4, 4]);
        assert_eq!(bmfont.info.spacing, [-8, -8]);
    }

    #[test]
    fn parse_common_line() {
        let bmfont = parse_fnt(SAMPLE_FNT).unwrap();
        assert_eq!(bmfont.common.line_height, 52);
        assert_eq!(bmfont.common.base, 40);
        assert_eq!(bmfont.common.scale_w, 512);
        assert_eq!(bmfont.common.scale_h, 512);
        assert_eq!(bmfont.common.pages, 1);
        assert!(!bmfont.common.packed);
    }

    #[test]
    fn parse_pages() {
        let bmfont = parse_fnt(SAMPLE_FNT).unwrap();
        assert_eq!(bmfont.pages.len(), 1);
        assert_eq!(bmfont.pages[0].id, 0);
        assert_eq!(bmfont.pages[0].file, "hun.png");
    }

    #[test]
    fn parse_chars() {
        let bmfont = parse_fnt(SAMPLE_FNT).unwrap();
        assert_eq!(bmfont.chars.len(), 3);
        // Space
        assert_eq!(bmfont.chars[0].id, 32);
        assert_eq!(bmfont.chars[0].xadvance, 18);
        // '!'
        assert_eq!(bmfont.chars[1].id, 33);
        assert_eq!(bmfont.chars[1].x, 66);
        assert_eq!(bmfont.chars[1].width, 16);
        assert_eq!(bmfont.chars[1].height, 48);
        // 'A'
        assert_eq!(bmfont.chars[2].id, 65);
        assert_eq!(bmfont.chars[2].xoffset, -1);
        assert_eq!(bmfont.chars[2].xadvance, 34);
    }

    #[test]
    fn parse_empty_kernings() {
        let bmfont = parse_fnt(SAMPLE_FNT).unwrap();
        assert!(bmfont.kernings.is_empty());
    }

    #[test]
    fn parse_kernings() {
        let input = concat!(
            "info face=\"T\" size=12 bold=0 italic=0 charset=\"\" unicode=0 stretchH=100 smooth=1 aa=1 padding=0,0,0,0 spacing=0,0\n",
            "common lineHeight=12 base=10 scaleW=64 scaleH=64 pages=1 packed=0\n",
            "page id=0 file=\"t.png\"\n",
            "chars count=0\n",
            "kernings count=2\n",
            "kerning first=65 second=86 amount=-2\n",
            "kerning first=86 second=65 amount=-1\n",
        );
        let bmfont = parse_fnt(input).unwrap();
        assert_eq!(bmfont.kernings.len(), 2);
        assert_eq!(bmfont.kernings[0].first, 65);
        assert_eq!(bmfont.kernings[0].second, 86);
        assert_eq!(bmfont.kernings[0].amount, -2);
    }
}
