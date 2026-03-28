use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "fontbake", about = "Font build tool — CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a font from a .hiero configuration
    Build {
        /// Path to the .hiero configuration file
        config: PathBuf,
        /// Output directory (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },
    /// Import an existing BMFont (.fnt + .png) and re-export
    Import {
        /// Path to the .fnt file
        fnt: PathBuf,
        /// Output directory (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },
    /// Merge multiple BMFont sources into one
    Merge {
        /// Paths to .fnt files to merge
        #[arg(required = true)]
        sources: Vec<PathBuf>,
        /// Output face name
        #[arg(short, long, default_value = "merged")]
        name: String,
        /// Page width
        #[arg(long, default_value = "1024")]
        page_width: u32,
        /// Page height
        #[arg(long, default_value = "1024")]
        page_height: u32,
        /// Output directory
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },
    /// Inspect a .hiero config or .fnt file
    Inspect {
        /// Path to file to inspect
        file: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Build { config, output } => cmd_build(&config, &output),
        Commands::Import { fnt, output } => cmd_import(&fnt, &output),
        Commands::Merge {
            sources,
            name,
            page_width,
            page_height,
            output,
        } => cmd_merge(&sources, &name, page_width, page_height, &output),
        Commands::Inspect { file } => cmd_inspect(&file),
    }
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

fn cmd_build(config_path: &Path, output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let config_text = fs::read_to_string(config_path)?;
    let spec = fontbake_core::config::parse_hiero(&config_text)?;

    println!("Building font: {}", spec.font_name);
    println!("  Font size: {}", spec.font_size);
    println!("  Render mode: {:?}", spec.render_mode);
    println!("  Effects: {} configured", spec.effects.len());
    println!("  Glyph text: {} chars", spec.glyph_text.chars().count());
    println!("  Primary font: {}", spec.primary_font_path);
    println!("  Fallback fonts: {}", spec.fallback_font_paths.len());

    // Load font files
    let primary_data = fs::read(&spec.primary_font_path)
        .map_err(|e| format!("cannot read primary font '{}': {e}", spec.primary_font_path))?;

    let mut fallback_data: Vec<(Vec<u8>, String)> = Vec::new();
    for path in &spec.fallback_font_paths {
        let data =
            fs::read(path).map_err(|e| format!("cannot read fallback font '{path}': {e}"))?;
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        fallback_data.push((data, name));
    }

    let fallback_refs: Vec<(&[u8], String)> = fallback_data
        .iter()
        .map(|(d, n)| (d.as_slice(), n.clone()))
        .collect();

    let result =
        fontbake_core::pipeline::build::build_from_config(&spec, &primary_data, &fallback_refs)?;

    // Write output
    fs::create_dir_all(output_dir)?;

    let fnt_path = output_dir.join(format!("{}.fnt", spec.font_name));
    fs::write(&fnt_path, &result.fnt_text)?;
    println!("  Wrote: {}", fnt_path.display());

    for (i, png) in result.page_pngs.iter().enumerate() {
        let suffix = if i == 0 {
            String::new()
        } else {
            format!("_{i}")
        };
        let png_path = output_dir.join(format!("{}{suffix}.png", spec.font_name));
        fs::write(&png_path, png)?;
        println!("  Wrote: {}", png_path.display());
    }

    println!(
        "Done: {} glyphs, {} pages",
        result.glyphs.len(),
        result.page_pngs.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

fn cmd_import(fnt_path: &Path, output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let fnt_text = fs::read_to_string(fnt_path)?;
    let fnt_dir = fnt_path.parent().unwrap_or(Path::new("."));

    // Parse to find page filenames
    let bmfont = fontbake_core::source::bmfont_text::parse_fnt(&fnt_text)?;

    println!("Importing BMFont: {}", bmfont.info.face);
    println!("  Pages: {}", bmfont.pages.len());
    println!("  Chars: {}", bmfont.chars.len());
    println!("  Kernings: {}", bmfont.kernings.len());

    // Load PNG pages
    let mut pages = Vec::new();
    for page in &bmfont.pages {
        let png_path = fnt_dir.join(&page.file);
        let png_bytes = fs::read(&png_path)
            .map_err(|e| format!("cannot read page '{}': {e}", png_path.display()))?;
        let decoded = fontbake_core::pipeline::import::decode_png_page(&png_bytes)?;
        pages.push(decoded);
    }

    let source_id = fnt_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "imported".into());

    let (_bmfont, glyphs) =
        fontbake_core::pipeline::import::import_bmfont(&fnt_text, &pages, &source_id)?;

    // Re-export (roundtrip)
    fs::create_dir_all(output_dir)?;
    let out_fnt = output_dir.join(format!("{source_id}_reimport.fnt"));

    let page_filenames: Vec<String> = bmfont.pages.iter().map(|p| p.file.clone()).collect();

    let fnt_out = fontbake_core::export::bmfont_text::glyphs_to_fnt(
        &glyphs,
        &bmfont.info.face,
        bmfont.info.size,
        bmfont.common.line_height,
        bmfont.common.base,
        bmfont.common.scale_w,
        bmfont.common.scale_h,
        &page_filenames,
        bmfont.info.padding,
        bmfont.info.spacing,
    );

    fs::write(&out_fnt, &fnt_out)?;
    println!("  Wrote: {}", out_fnt.display());
    println!("Done: {} glyphs imported", glyphs.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

fn cmd_merge(
    sources: &[PathBuf],
    name: &str,
    page_width: u32,
    page_height: u32,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Merging {} sources into '{name}'", sources.len());

    let mut all_glyphs: Vec<Vec<fontbake_core::model::GlyphRecord>> = Vec::new();

    for src in sources {
        let fnt_text = fs::read_to_string(src)?;
        let fnt_dir = src.parent().unwrap_or(Path::new("."));
        let bmfont = fontbake_core::source::bmfont_text::parse_fnt(&fnt_text)?;

        let mut pages = Vec::new();
        for page in &bmfont.pages {
            let png_path = fnt_dir.join(&page.file);
            let png_bytes = fs::read(&png_path)
                .map_err(|e| format!("cannot read '{}': {e}", png_path.display()))?;
            let decoded = fontbake_core::pipeline::import::decode_png_page(&png_bytes)?;
            pages.push(decoded);
        }

        let source_id = src
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());

        let (_bm, glyphs) =
            fontbake_core::pipeline::import::import_bmfont(&fnt_text, &pages, &source_id)?;
        println!("  {} → {} glyphs", src.display(), glyphs.len());
        all_glyphs.push(glyphs);
    }

    let glyph_refs: Vec<&[fontbake_core::model::GlyphRecord]> =
        all_glyphs.iter().map(|v| v.as_slice()).collect();

    let result = fontbake_core::pipeline::merge::merge_fonts(
        &glyph_refs,
        name,
        52, // default size
        52,
        40,
        page_width,
        page_height,
        [0, 0, 0, 0],
        [0, 0],
    )?;

    fs::create_dir_all(output_dir)?;

    let fnt_path = output_dir.join(format!("{name}.fnt"));
    fs::write(&fnt_path, &result.fnt_text)?;
    println!("  Wrote: {}", fnt_path.display());

    for (i, png) in result.page_pngs.iter().enumerate() {
        let suffix = if i == 0 {
            String::new()
        } else {
            format!("_{i}")
        };
        let png_path = output_dir.join(format!("{name}{suffix}.png"));
        fs::write(&png_path, png)?;
        println!("  Wrote: {}", png_path.display());
    }

    println!(
        "Done: {} glyphs merged, {} pages",
        result.glyphs.len(),
        result.page_pngs.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Inspect
// ---------------------------------------------------------------------------

fn cmd_inspect(file_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(file_path)?;
    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "hiero" => {
            let spec = fontbake_core::config::parse_hiero(&content)?;
            println!("{}", serde_json::to_string_pretty(&spec)?);
        }
        "fnt" => {
            let bmfont = fontbake_core::source::bmfont_text::parse_fnt(&content)?;
            println!("{}", serde_json::to_string_pretty(&bmfont)?);
        }
        other => {
            eprintln!("unknown file type: .{other} (expected .hiero or .fnt)");
            std::process::exit(1);
        }
    }
    Ok(())
}
