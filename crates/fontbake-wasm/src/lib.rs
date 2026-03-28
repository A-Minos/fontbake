use wasm_bindgen::prelude::*;

/// Parse a `.hiero` config string and return a JSON representation of the BuildSpec.
#[wasm_bindgen]
pub fn parse_hiero(config_text: &str) -> Result<String, JsValue> {
    let spec = fontbake_core::config::parse_hiero(config_text)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(&spec).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Parse a text BMFont `.fnt` string and return a JSON representation.
#[wasm_bindgen]
pub fn parse_bmfont(fnt_text: &str) -> Result<String, JsValue> {
    let bmfont = fontbake_core::source::bmfont_text::parse_fnt(fnt_text)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(&bmfont).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Build a font from a `.hiero` config, primary font bytes, and optional fallback font bytes.
///
/// # Arguments
/// - `config_text` — raw `.hiero` file content
/// - `primary_font` — TTF/OTF bytes of the primary font
/// - `fallback_fonts_json` — JSON array of `{"name": "...", "data": [u8...]}` objects
///
/// # Returns
/// JSON object: `{ "fnt_text": "...", "page_pngs": [[u8...], ...], "glyph_count": N, "glyphs_json": "[...]" }`
#[wasm_bindgen]
pub fn build_font(
    config_text: &str,
    primary_font: &[u8],
    fallback_fonts_json: &str,
) -> Result<JsValue, JsValue> {
    let spec = fontbake_core::config::parse_hiero(config_text)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    // Parse fallback fonts from JSON
    let fallback_entries: Vec<FallbackEntry> =
        serde_json::from_str(fallback_fonts_json).unwrap_or_default();

    let fallback_refs: Vec<(&[u8], String)> = fallback_entries
        .iter()
        .map(|e| (e.data.as_slice(), e.name.clone()))
        .collect();

    let result =
        fontbake_core::pipeline::build::build_from_config(&spec, primary_font, &fallback_refs)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let glyphs_json = serde_json::to_string(&result.glyphs)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let output = BuildOutput {
        fnt_text: result.fnt_text,
        page_pngs: result.page_pngs,
        glyph_count: result.glyphs.len(),
        glyphs_json,
    };

    serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Import a BMFont (.fnt text + PNG page bytes) and return extracted glyphs as JSON.
///
/// # Arguments
/// - `fnt_text` — raw `.fnt` file content
/// - `png_pages` — array of PNG file bytes (ordered by page id)
/// - `source_id` — identifier for provenance tracking
///
/// # Returns
/// JSON object: `{ "bmfont_json": "...", "glyph_count": N, "glyphs_json": "[...]" }`
#[wasm_bindgen]
pub fn import_bmfont(
    fnt_text: &str,
    png_pages_json: &str,
    source_id: &str,
) -> Result<JsValue, JsValue> {
    let png_bytes_list: Vec<Vec<u8>> = serde_json::from_str(png_pages_json).unwrap_or_default();

    let mut pages = Vec::new();
    for png_bytes in &png_bytes_list {
        let page = fontbake_core::pipeline::import::decode_png_page(png_bytes)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        pages.push(page);
    }

    let (bmfont, glyphs) =
        fontbake_core::pipeline::import::import_bmfont(fnt_text, &pages, source_id)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let glyphs_json = serde_json::to_string(&glyphs)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let output = ImportOutput {
        bmfont_json: serde_json::to_string(&bmfont).unwrap_or_default(),
        glyph_count: glyphs.len(),
        glyphs_json,
    };

    serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Merge multiple glyph sets into a single font output.
///
/// # Arguments
/// - `glyph_sets_json` — JSON array of glyph-set arrays: `[[GlyphRecord, ...], ...]`
///   (ordered by priority — first set wins on codepoint conflicts)
/// - `merge_config_json` — JSON object with merge parameters:
///   `{ "face": "...", "font_size": N, "line_height": N, "base": N,
///      "page_width": N, "page_height": N, "padding": [T,R,B,L], "spacing": [H,V] }`
///
/// # Returns
/// JSON object: `{ "fnt_text": "...", "page_pngs": [[u8...], ...], "glyph_count": N, "glyphs_json": "[...]" }`
#[wasm_bindgen]
pub fn merge_fonts(glyph_sets_json: &str, merge_config_json: &str) -> Result<JsValue, JsValue> {
    let glyph_sets: Vec<Vec<fontbake_core::model::GlyphRecord>> =
        serde_json::from_str(glyph_sets_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse glyph_sets: {e}")))?;

    let config: MergeConfig = serde_json::from_str(merge_config_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse merge_config: {e}")))?;

    let set_refs: Vec<&[fontbake_core::model::GlyphRecord]> =
        glyph_sets.iter().map(|s| s.as_slice()).collect();

    let result = fontbake_core::pipeline::merge::merge_fonts(
        &set_refs,
        &config.face,
        config.font_size,
        config.line_height,
        config.base,
        config.page_width,
        config.page_height,
        config.padding,
        config.spacing,
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let glyphs_json = serde_json::to_string(&result.glyphs)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let output = BuildOutput {
        fnt_text: result.fnt_text,
        page_pngs: result.page_pngs,
        glyph_count: result.glyphs.len(),
        glyphs_json,
    };

    serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
}

// ---------------------------------------------------------------------------
// Internal types for JSON serialization
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct FallbackEntry {
    name: String,
    data: Vec<u8>,
}

#[derive(serde::Serialize)]
struct BuildOutput {
    fnt_text: String,
    page_pngs: Vec<Vec<u8>>,
    glyph_count: usize,
    /// JSON array of GlyphRecord objects
    glyphs_json: String,
}

#[derive(serde::Serialize)]
struct ImportOutput {
    bmfont_json: String,
    glyph_count: usize,
    /// JSON array of GlyphRecord objects
    glyphs_json: String,
}

#[derive(serde::Deserialize)]
struct MergeConfig {
    face: String,
    font_size: i32,
    line_height: u32,
    base: u32,
    page_width: u32,
    page_height: u32,
    padding: [i32; 4],
    spacing: [i32; 2],
}
