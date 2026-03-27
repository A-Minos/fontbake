//! Parser for Hiero .hiero configuration files.
//!
//! Format: one `key=value` pair per line, with `#ID:` prefix tokens that Hiero
//! uses internally for ordering. Lines with no `=` (including blank lines and
//! lines that are only a `#ID:` token) are silently skipped.
//!
//! Only `render_type=0` (Java path) is supported; any other value is an error.
//! Unknown keys are ignored so that forward-compatible configs don't fail.

use std::collections::HashMap;

use crate::model::{
    AdvanceAdjust, BuildSpec, EffectSpec, FontbakeError, Padding, RenderMode,
};

/// Parse a `.hiero` file byte slice into a [`BuildSpec`].
///
/// The caller is responsible for resolving any relative font paths — this
/// function returns them verbatim from the config.
pub fn parse_hiero(input: &str) -> Result<BuildSpec, FontbakeError> {
    let kv = extract_kv(input);

    // --- render type ---
    let render_type: u32 = kv
        .get("render_type")
        .map(|v| {
            v.parse::<u32>()
                .map_err(|_| FontbakeError::Config(format!("invalid render_type: {v}")))
        })
        .transpose()?
        .unwrap_or(0);
    if render_type != 0 {
        return Err(FontbakeError::Unsupported(format!(
            "render_type={render_type} is not supported (only 0 / Java path)"
        )));
    }

    // --- primary font ---
    let primary_font_path = kv
        .get("font2.file")
        .cloned()
        .unwrap_or_default();

    // --- fallback fonts: fallback.font.0, fallback.font.1, … ---
    let mut fallback_font_paths: Vec<String> = Vec::new();
    for i in 0.. {
        let key = format!("fallback.font.{i}");
        match kv.get(&key) {
            Some(v) => fallback_font_paths.push(v.clone()),
            None => break,
        }
    }

    // --- basic settings ---
    let font_name = kv.get("font.name").cloned().unwrap_or_default();
    let font_size = parse_u32(&kv, "font.size", 52)?;
    let bold = parse_bool(&kv, "font.bold", false)?;
    let italic = parse_bool(&kv, "font.italic", false)?;
    let gamma = parse_f32(&kv, "font.gamma", 1.8)?;
    let mono = parse_bool(&kv, "font.mono", false)?;

    // --- padding ---
    let padding = Padding {
        top:    parse_i32(&kv, "pad.top",    0)?,
        right:  parse_i32(&kv, "pad.right",  0)?,
        bottom: parse_i32(&kv, "pad.bottom", 0)?,
        left:   parse_i32(&kv, "pad.left",   0)?,
    };

    // --- advance adjust ---
    let advance_adjust = AdvanceAdjust {
        x: parse_i32(&kv, "pad.advance.x", 0)?,
        y: parse_i32(&kv, "pad.advance.y", 0)?,
    };

    // --- page size ---
    let page_width  = parse_u32(&kv, "glyph.page.width",  1024)?;
    let page_height = parse_u32(&kv, "glyph.page.height", 1024)?;

    // --- glyph set ---
    // The value is the literal character sequence; Hiero stores it verbatim.
    let glyph_text = kv.get("glyph.text").cloned().unwrap_or_default();

    // --- effects ---
    // Hiero stores a single effect as:
    //   effect.class=com.badlogic.gdx.tools.hiero.unicodefont.effects.Foo
    //   effect.Foo_param=value
    // We only support DistanceFieldEffect and treat any other class as unsupported.
    let effects = parse_effects(&kv)?;

    Ok(BuildSpec {
        font_name,
        primary_font_path,
        fallback_font_paths,
        font_size,
        bold,
        italic,
        gamma,
        mono,
        padding,
        advance_adjust,
        page_width,
        page_height,
        glyph_text,
        render_mode: RenderMode::Java,
        effects,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Strip the `#XXXX:` prefix token (if present) and return the `key=value`
/// pairs as a `HashMap`. Order is not preserved; duplicate keys keep the last
/// value (matches Java Hiero behaviour on reload).
fn extract_kv(input: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for raw_line in input.lines() {
        // Strip optional `#ID:` prefix (uppercase alphanumeric, exactly
        // matching Hiero's obfuscated key format).
        let line = if let Some(pos) = raw_line.find(':') {
            let prefix = &raw_line[..pos];
            // Prefix must be `#` followed by 2-4 uppercase-alphanum chars.
            let inner = &prefix[1.min(prefix.len())..];
            if prefix.starts_with('#') && inner.len() >= 2 && inner.len() <= 4
                && inner.chars().all(|c| c.is_ascii_alphanumeric())
            {
                raw_line[pos + 1..].trim_start()
            } else {
                raw_line.trim()
            }
        } else {
            raw_line.trim()
        };

        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let value = if key == "glyph.text" {
                v.to_string()
            } else {
                v.trim().to_string()
            };
            map.insert(key, value);
        }
    }
    map
}

/// Parse `effect.class` and the matching `effect.*` parameters into an
/// `EffectSpec`. Returns an empty vec when no `effect.class` key is present.
fn parse_effects(kv: &HashMap<String, String>) -> Result<Vec<EffectSpec>, FontbakeError> {
    let class = match kv.get("effect.class") {
        Some(c) => c,
        None => return Ok(vec![]),
    };

    // Only the short class name after the last `.` matters for dispatch.
    let short = class.rsplit('.').next().unwrap_or(class.as_str());

    match short {
        "DistanceFieldEffect" => {
            let color = kv
                .get("effect.Color")
                .cloned()
                .unwrap_or_else(|| "ffffff".to_string());
            let scale = kv
                .get("effect.Scale")
                .map(|v| {
                    v.parse::<u32>().map_err(|_| {
                        FontbakeError::Config(format!("invalid effect.Scale: {v}"))
                    })
                })
                .transpose()?
                .unwrap_or(32);
            let spread = kv
                .get("effect.Spread")
                .map(|v| {
                    v.parse::<f32>().map_err(|_| {
                        FontbakeError::Config(format!("invalid effect.Spread: {v}"))
                    })
                })
                .transpose()?
                .unwrap_or(3.5);

            Ok(vec![EffectSpec::DistanceField {
                color,
                scale,
                spread,
            }])
        }
        other => Err(FontbakeError::Unsupported(format!(
            "effect class '{other}' is not supported in v1 (only DistanceFieldEffect)"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Typed parse helpers — all propagate FontbakeError::Config on failure
// ---------------------------------------------------------------------------

fn parse_u32(kv: &HashMap<String, String>, key: &str, default: u32) -> Result<u32, FontbakeError> {
    kv.get(key)
        .map(|v| {
            v.parse::<u32>()
                .map_err(|_| FontbakeError::Config(format!("invalid {key}: {v}")))
        })
        .transpose()
        .map(|o| o.unwrap_or(default))
}

fn parse_i32(kv: &HashMap<String, String>, key: &str, default: i32) -> Result<i32, FontbakeError> {
    kv.get(key)
        .map(|v| {
            v.parse::<i32>()
                .map_err(|_| FontbakeError::Config(format!("invalid {key}: {v}")))
        })
        .transpose()
        .map(|o| o.unwrap_or(default))
}

fn parse_f32(kv: &HashMap<String, String>, key: &str, default: f32) -> Result<f32, FontbakeError> {
    kv.get(key)
        .map(|v| {
            v.parse::<f32>()
                .map_err(|_| FontbakeError::Config(format!("invalid {key}: {v}")))
        })
        .transpose()
        .map(|o| o.unwrap_or(default))
}

fn parse_bool(
    kv: &HashMap<String, String>,
    key: &str,
    default: bool,
) -> Result<bool, FontbakeError> {
    kv.get(key)
        .map(|v| match v.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(FontbakeError::Config(format!("invalid {key}: {other}"))),
        })
        .transpose()
        .map(|o| o.unwrap_or(default))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EffectSpec;

    const SAMPLE: &str = r#"
#YQ:font.name=HUN2
#BQ:font.size=52
#PS:font.bold=false
#HN:font.italic=false
#VN:font.gamma=1.8
#RV:font.mono=false
#HN:
#QP:font2.file=/home/miansoft/Project/bmfont-test/hun2.ttf
#KH:font2.use=true
#SK:
#QW:pad.top=4
#WQ:pad.right=4
#XN:pad.bottom=4
#WZ:pad.left=4
#QP:pad.advance.x=-8
#TY:pad.advance.y=-8
#HX:
#YS:glyph.native.rendering=false
#MV:glyph.page.width=1024
#SM:glyph.page.height=1024
#PT:glyph.text=ABC
#ZP:
#QT:render_type=0
#KW:
#MK:fallback.font.0=/home/miansoft/Project/bmfont-test/chinese_font.otf
#HK:
#KV:effect.class=com.badlogic.gdx.tools.hiero.unicodefont.effects.DistanceFieldEffect
#MS:effect.Color=ffffff
#TK:effect.Scale=32
#QQ:effect.Spread=3.5
"#;

    #[test]
    fn parses_hun2_config() {
        let spec = parse_hiero(SAMPLE).expect("parse failed");
        assert_eq!(spec.font_name, "HUN2");
        assert_eq!(spec.font_size, 52);
        assert!(!spec.bold);
        assert!(!spec.italic);
        assert_eq!(spec.gamma, 1.8);
        assert_eq!(spec.padding.top, 4);
        assert_eq!(spec.padding.right, 4);
        assert_eq!(spec.advance_adjust.x, -8);
        assert_eq!(spec.page_width, 1024);
        assert_eq!(spec.glyph_text, "ABC");
        assert_eq!(spec.primary_font_path, "/home/miansoft/Project/bmfont-test/hun2.ttf");
        assert_eq!(spec.fallback_font_paths, vec!["/home/miansoft/Project/bmfont-test/chinese_font.otf"]);
        assert!(matches!(spec.render_mode, RenderMode::Java));
        assert_eq!(spec.effects.len(), 1);
        match &spec.effects[0] {
            EffectSpec::DistanceField { color, scale, spread } => {
                assert_eq!(color, "ffffff");
                assert_eq!(*scale, 32);
                assert!((spread - 3.5).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn rejects_unsupported_render_type() {
        let input = "#AA:render_type=1\n";
        assert!(parse_hiero(input).is_err());
    }

    #[test]
    fn rejects_unknown_effect_class() {
        let input = concat!(
            "#AA:render_type=0\n",
            "#BB:effect.class=com.badlogic.gdx.tools.hiero.unicodefont.effects.ColorEffect\n"
        );
        assert!(parse_hiero(input).is_err());
    }

    #[test]
    fn fallback_chain_ordering() {
        let input = concat!(
            "#AA:render_type=0\n",
            "#BB:fallback.font.0=/fonts/a.ttf\n",
            "#CC:fallback.font.1=/fonts/b.ttf\n",
            "#DD:fallback.font.2=/fonts/c.ttf\n",
        );
        let spec = parse_hiero(input).unwrap();
        assert_eq!(spec.fallback_font_paths, vec!["/fonts/a.ttf", "/fonts/b.ttf", "/fonts/c.ttf"]);
    }
}