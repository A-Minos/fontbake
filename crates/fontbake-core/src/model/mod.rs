use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FontbakeError {
    #[error("config parse error: {0}")]
    Config(String),
    #[error("unsupported feature: {0}")]
    Unsupported(String),
    #[error("font loading error: {0}")]
    FontLoad(String),
    #[error("bmfont parse error: {0}")]
    BmfontParse(String),
    #[error("png error: {0}")]
    Png(String),
    #[error("pack error: {0}")]
    Pack(String),
    #[error("io error: {0}")]
    Io(String),
}

// ---------------------------------------------------------------------------
// BuildSpec — normalised build configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSpec {
    /// Display name for the font (used in .fnt `face`)
    pub font_name: String,
    /// Primary font source (raw bytes of TTF/OTF file)
    pub primary_font_path: String,
    /// Fallback font sources, in priority order
    pub fallback_font_paths: Vec<String>,
    /// Nominal font size in points
    pub font_size: u32,
    pub bold: bool,
    pub italic: bool,
    /// Gamma value (kept for config fidelity; v1 may not apply in raster)
    pub gamma: f32,
    /// Monospaced flag
    pub mono: bool,
    pub padding: Padding,
    pub advance_adjust: AdvanceAdjust,
    pub page_width: u32,
    pub page_height: u32,
    /// Characters to include in the font
    pub glyph_text: String,
    pub render_mode: RenderMode,
    pub effects: Vec<EffectSpec>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Padding {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AdvanceAdjust {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderMode {
    /// Java2D path (render_type=0) — the only v1-supported mode
    Java,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EffectSpec {
    DistanceField {
        /// Hex colour, e.g. "ffffff"
        color: String,
        /// Upscale factor for the intermediate mask
        scale: u32,
        /// Spread in pixels
        spread: f32,
    },
}

// ---------------------------------------------------------------------------
// GlyphRecord — per-glyph internal representation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// Glyph was rasterised from an outline font
    Outline,
    /// Glyph was imported as a bitmap (PNG/FNT)
    BitmapImport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphRecord {
    /// Unicode codepoint
    pub codepoint: u32,
    pub source_kind: SourceKind,
    /// Human-readable source identifier (font filename or bmfont name)
    pub source_id: String,
    /// RGBA pixel data for the rendered / imported glyph
    pub bitmap_rgba: Vec<u8>,
    /// Bitmap dimensions
    pub width: u32,
    pub height: u32,
    /// Placement offsets
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    /// Atlas page index (assigned during packing)
    pub page: u32,
    /// Position on atlas (assigned during packing)
    pub x: u32,
    pub y: u32,
    /// Kerning pairs: (second_codepoint, amount)
    pub kernings: Vec<(u32, i16)>,
}

impl GlyphRecord {
    pub fn new(codepoint: u32, source_kind: SourceKind, source_id: String) -> Self {
        Self {
            codepoint,
            source_kind,
            source_id,
            bitmap_rgba: Vec::new(),
            width: 0,
            height: 0,
            xoffset: 0,
            yoffset: 0,
            xadvance: 0,
            page: 0,
            x: 0,
            y: 0,
            kernings: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// AtlasPage — one page of the packed atlas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AtlasPage {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data
    pub rgba: Vec<u8>,
}

impl AtlasPage {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height * 4) as usize;
        Self {
            width,
            height,
            rgba: vec![0u8; size],
        }
    }
}

// ---------------------------------------------------------------------------
// BmFont import model — parsed from text .fnt
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmFontInfo {
    pub face: String,
    pub size: i32,
    pub bold: bool,
    pub italic: bool,
    pub charset: String,
    pub unicode: bool,
    pub stretch_h: u32,
    pub smooth: bool,
    pub aa: u32,
    pub padding: [i32; 4],
    pub spacing: [i32; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmFontCommon {
    pub line_height: u32,
    pub base: u32,
    pub scale_w: u32,
    pub scale_h: u32,
    pub pages: u32,
    pub packed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmFontPage {
    pub id: u32,
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmFontChar {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
    pub page: u32,
    pub chnl: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmFontKerning {
    pub first: u32,
    pub second: u32,
    pub amount: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmFont {
    pub info: BmFontInfo,
    pub common: BmFontCommon,
    pub pages: Vec<BmFontPage>,
    pub chars: Vec<BmFontChar>,
    pub kernings: Vec<BmFontKerning>,
}

// ---------------------------------------------------------------------------
// BuildResult — output of a full pipeline run
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BuildResult {
    /// The generated .fnt content (text BMFont format)
    pub fnt_text: String,
    /// Atlas page PNGs (index = page id)
    pub page_pngs: Vec<Vec<u8>>,
    /// All glyph records (for inspection / debugging)
    pub glyphs: Vec<GlyphRecord>,
}
