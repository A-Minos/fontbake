//! Outline font source — TTF/OTF loading via `ttf-parser`.
//!
//! Provides cmap lookup, glyph outline extraction (as tiny-skia path segments),
//! and basic horizontal metrics. No file-system access — all inputs are byte slices.

use crate::model::FontbakeError;

/// A loaded outline font, wrapping a parsed `ttf_parser::Face`.
/// Lifetime `'a` is tied to the font data byte slice.
pub struct OutlineFont<'a> {
    face: ttf_parser::Face<'a>,
    /// Human-readable identifier for provenance tracking
    pub source_id: String,
    /// Units-per-em for this font
    pub units_per_em: u16,
}

impl<'a> OutlineFont<'a> {
    /// Load a font from raw TTF/OTF bytes.
    pub fn load(data: &'a [u8], source_id: String) -> Result<Self, FontbakeError> {
        let face = ttf_parser::Face::parse(data, 0)
            .map_err(|e| FontbakeError::FontLoad(format!("{source_id}: {e}")))?;
        let units_per_em = face.units_per_em();
        Ok(Self {
            face,
            source_id,
            units_per_em,
        })
    }

    /// Check whether this font has a glyph for the given codepoint.
    pub fn has_glyph(&self, codepoint: char) -> bool {
        self.face.glyph_index(codepoint).is_some()
    }

    /// Get the glyph ID for a codepoint, or `None` if not present.
    pub fn glyph_id(&self, codepoint: char) -> Option<ttf_parser::GlyphId> {
        self.face.glyph_index(codepoint)
    }

    /// Get horizontal advance width for a glyph in font units.
    pub fn advance_width(&self, glyph_id: ttf_parser::GlyphId) -> Option<u16> {
        self.face.glyph_hor_advance(glyph_id)
    }

    /// Get horizontal left side bearing for a glyph in font units.
    pub fn left_side_bearing(&self, glyph_id: ttf_parser::GlyphId) -> Option<i16> {
        self.face.glyph_hor_side_bearing(glyph_id)
    }

    /// Get the bounding box for a glyph in font units.
    /// Returns `None` for empty glyphs (e.g. space).
    pub fn glyph_bbox(&self, glyph_id: ttf_parser::GlyphId) -> Option<ttf_parser::Rect> {
        self.face.glyph_bounding_box(glyph_id)
    }

    /// Get ascender in font units.
    pub fn ascender(&self) -> i16 {
        self.face.ascender()
    }

    /// Get OS/2 usWinAscent in font units (what Java AWT uses for base).
    /// Falls back to hhea ascender if OS/2 is not present.
    pub fn win_ascender(&self) -> u16 {
        self.face
            .tables()
            .os2
            .map(|os2| os2.windows_ascender() as u16)
            .unwrap_or(self.face.ascender().unsigned_abs())
    }

    /// Get descender in font units (typically negative).
    pub fn descender(&self) -> i16 {
        self.face.descender()
    }

    /// Get line gap in font units.
    pub fn line_gap(&self) -> i16 {
        self.face.line_gap()
    }

    /// Outline a glyph into a series of path commands.
    /// Returns `None` if the glyph has no outline (e.g. space, or bitmap-only).
    pub fn outline_glyph(&self, glyph_id: ttf_parser::GlyphId) -> Option<Vec<PathCommand>> {
        let mut builder = PathBuilder::new();
        self.face.outline_glyph(glyph_id, &mut builder)?;
        Some(builder.commands)
    }

    /// Scale factor to convert font units to pixels at a given point size.
    pub fn scale_for_size(&self, size_px: f32) -> f32 {
        size_px / self.units_per_em as f32
    }

    /// Get kerning between two glyphs in font units.
    /// Returns 0 if no kern table or no pair entry.
    pub fn kern(&self, left: ttf_parser::GlyphId, right: ttf_parser::GlyphId) -> i16 {
        self.face
            .tables()
            .kern
            .and_then(|kern| {
                kern.subtables.into_iter().find_map(|st| {
                    if st.horizontal && !st.variable {
                        st.glyphs_kerning(left, right)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Path commands — intermediate representation for glyph outlines
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum PathCommand {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    QuadTo {
        x1: f32,
        y1: f32,
        x: f32,
        y: f32,
    },
    CurveTo {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x: f32,
        y: f32,
    },
    Close,
}

struct PathBuilder {
    commands: Vec<PathCommand>,
}

impl PathBuilder {
    fn new() -> Self {
        Self {
            commands: Vec::with_capacity(64),
        }
    }
}

impl ttf_parser::OutlineBuilder for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::MoveTo { x, y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::LineTo { x, y });
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.commands.push(PathCommand::QuadTo { x1, y1, x, y });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.commands.push(PathCommand::CurveTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        });
    }

    fn close(&mut self) {
        self.commands.push(PathCommand::Close);
    }
}

// ---------------------------------------------------------------------------
// Fallback resolver
// ---------------------------------------------------------------------------

/// Resolve which font in a chain can render a given codepoint.
/// Returns the index into the chain (0 = primary) and the glyph ID.
///
/// `fonts` should be ordered: `[primary, fallback_0, fallback_1, …]`.
pub fn resolve_codepoint<'a>(
    fonts: &[&OutlineFont<'a>],
    codepoint: char,
) -> Option<(usize, ttf_parser::GlyphId)> {
    for (idx, font) in fonts.iter().enumerate() {
        if let Some(gid) = font.glyph_id(codepoint) {
            return Some((idx, gid));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid TrueType font (empty, but parseable).
    /// We test with a real font in integration tests; here we just verify
    /// the error path and the resolve_codepoint logic.
    #[test]
    fn load_invalid_data_returns_error() {
        let result = OutlineFont::load(b"not a font", "bad.ttf".into());
        match result {
            Err(e) => assert!(e.to_string().contains("bad.ttf")),
            Ok(_) => panic!("expected error for invalid font data"),
        }
    }

    #[test]
    fn resolve_codepoint_returns_none_for_empty_chain() {
        let fonts: Vec<&OutlineFont> = vec![];
        assert!(resolve_codepoint(&fonts, 'A').is_none());
    }
}
