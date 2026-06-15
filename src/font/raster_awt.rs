//! AWT-backed TTF→bitmap rasterizer: the *byte-faithful* path for
//! `font rasterize`.
//!
//! A pure-Rust outline rasterizer (`raster.rs`, behind `--experimental`) cannot
//! byte-match Java `java.awt`'s glyph metrics + anti-aliasing, so the committed,
//! live-validated relic golden font groups can only be reproduced by the proven
//! AWT rasterizer. This module therefore *shells out* to that rasterizer:
//!
//!   1. The Rust decode side (`modern.rs`) builds the worklist (fontId → face
//!      TTF bytes, `px`/`ascent` size mode) — identical to `build-fonts.ts`.
//!   2. This module writes the worklist as `<work>/fonts/fonts.tsv` plus each
//!      face as `<work>/fonts/ttf/face-<id>.ttf`, drops the *shared* copy of
//!      `FontRaster.java` (`tools/font-raster/FontRaster.java`, byte-identical to
//!      the relic oracle, embedded here with `include_str!`), compiles it with
//!      `javac`, and runs `java -Djava.awt.headless=true FontRaster <work>` —
//!      exactly the relic `FontRaster` step.
//!   3. It reads the AWT outputs back: `<work>/fonts/metrics/<id>.bin`
//!      (FontMetrics archive 13) and `<work>/fonts/sprites/<id>.bin` (glyph
//!      atlas archive 8). The Rust `bitmap`/group code then packs those `.bin`
//!      payloads (the deterministic, already byte-locked half).
//!
//! **External coupling.** This path requires a JDK on `PATH` (`javac` + `java`,
//! JDK 21 — the relic build's toolchain). The work is done in a scratch
//! `tempfile::TempDir` (auto-cleaned); nothing under the relic overlay tree is
//! touched. The relic copy of `FontRaster.java` and its build are left
//! untouched; the shared copy here is the one invoked.

use std::path::Path;
use std::process::Command;

use crate::cache_bail;
use crate::error::{Context, Result};
use crate::font::modern::ExtractedFace;
use crate::font::raster::SizeMode;

/// The shared, byte-identical copy of the relic AWT rasterizer, embedded so the
/// path resolves regardless of the working directory the binary runs from. It is
/// written into the scratch work dir and compiled there at run time.
const FONT_RASTER_JAVA: &str = include_str!("../../../font-raster/FontRaster.java");

/// One worklist entry: a font id, the face it renders, and how to size it. The
/// AWT side reads these from `fonts.tsv` (one line `fontId\tttfRel\tmode\tvalue`).
pub struct AwtWorkItem {
    pub font_id: u32,
    pub face_id: u32,
    pub face: ExtractedFace,
    pub mode: SizeMode,
}

/// The AWT raster output for one font: the two `.bin` payloads the packing step
/// wraps into JS5 groups, plus the resolved face/size for reporting.
pub struct AwtRasterOutput {
    pub font_id: u32,
    pub face_id: u32,
    pub face_kind: crate::font::modern::FaceKind,
    pub size_px: f32,
    pub metrics_bin: Vec<u8>,
    pub sprite_bin: Vec<u8>,
}

/// Format one `fonts.tsv` line for a work item, matching `build-fonts.ts`:
/// `fontId \t fonts/ttf/face-<faceId>.ttf \t (px|ascent) \t value`.
fn tsv_line(item: &AwtWorkItem) -> String {
    let (mode, value) = match item.mode {
        SizeMode::Px(px) => ("px", px),
        SizeMode::Ascent(a) => ("ascent", a),
    };
    // The relic worklist writes integer sizes (sizePx/targetAscentPx are bytes);
    // emit without a trailing `.0` so the tsv matches the relic form exactly.
    format!(
        "{}\tfonts/ttf/face-{}.ttf\t{}\t{}",
        item.font_id,
        item.face_id,
        mode,
        format_size(value)
    )
}

/// Render a size the way the relic `fonts.tsv` does: an integer when whole
/// (the archive-58 `sizePx`/`targetAscent` are byte fields), else a plain
/// decimal. Java `Float.parseFloat` accepts both.
fn format_size(v: f32) -> String {
    if (v.fract()).abs() < f32::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Locate a JDK tool (`javac` / `java`), honouring `JAVA_HOME` first, then
/// `PATH`. Returns the program string `Command` should spawn.
fn jdk_tool(tool: &str) -> String {
    if let Ok(home) = std::env::var("JAVA_HOME") {
        let candidate = Path::new(&home).join("bin").join(tool);
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    tool.to_string()
}

/// Run a JDK command, turning a missing binary or non-zero exit into a clear
/// cache error (the AWT path's only external dependency is the JDK).
fn run_jdk(program: &str, args: &[&str], what: &str) -> Result<()> {
    let output = Command::new(program).args(args).output().map_err(|e| {
        crate::error::CacheError::message(format!(
            "font rasterize needs a JDK on PATH (or JAVA_HOME): failed to run `{program}` for \
             {what}: {e}. The AWT rasterizer is the only path that byte-matches the golden \
             fonts; install JDK 21 or use --experimental for the (non-matching) native path."
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        cache_bail!(
            "`{program}` {what} failed ({}):\n{}{}",
            output.status,
            stdout.trim_end(),
            stderr.trim_end()
        );
    }
    Ok(())
}

/// Rasterize every work item through the AWT `FontRaster`, returning the raw
/// `.bin` payloads per font (in the input order). Sets up a scratch work tree,
/// compiles + runs the embedded shared `FontRaster.java`, and reads its output
/// back. Empty input is rejected by the caller, but handled gracefully here.
pub fn rasterize_awt(items: &[AwtWorkItem]) -> Result<Vec<AwtRasterOutput>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let work = tempfile::Builder::new()
        .prefix("rs3-font-raster-")
        .tempdir()
        .context("create scratch dir for the AWT font rasterizer")?;
    let base = work.path();
    let fonts_dir = base.join("fonts");
    let ttf_dir = fonts_dir.join("ttf");
    let classes_dir = base.join("classes");
    std::fs::create_dir_all(&ttf_dir)?;
    std::fs::create_dir_all(&classes_dir)?;

    // Write each distinct face once (multiple fonts share a face).
    let mut written_faces = std::collections::BTreeSet::new();
    for item in items {
        if written_faces.insert(item.face_id) {
            std::fs::write(
                ttf_dir.join(format!("face-{}.ttf", item.face_id)),
                &item.face.bytes,
            )?;
        }
    }

    // Worklist tsv (one line per font), matching the relic `fonts.tsv` form.
    let tsv: String = items.iter().map(tsv_line).collect::<Vec<_>>().join("\n");
    std::fs::write(fonts_dir.join("fonts.tsv"), format!("{tsv}\n"))?;

    // Drop + compile the shared AWT rasterizer in the scratch dir.
    let java_src = base.join("FontRaster.java");
    std::fs::write(&java_src, FONT_RASTER_JAVA)?;
    let javac = jdk_tool("javac");
    run_jdk(
        &javac,
        &[
            "-d",
            &classes_dir.to_string_lossy(),
            &java_src.to_string_lossy(),
        ],
        "compiling FontRaster.java",
    )?;

    // Run headless AWT on the worklist; it writes fonts/metrics|sprites/<id>.bin.
    let java = jdk_tool("java");
    run_jdk(
        &java,
        &[
            "-Djava.awt.headless=true",
            "-cp",
            &classes_dir.to_string_lossy(),
            "FontRaster",
            &base.to_string_lossy(),
        ],
        "running FontRaster",
    )?;

    // Read the AWT outputs back, in input order.
    let metrics_dir = fonts_dir.join("metrics");
    let sprites_dir = fonts_dir.join("sprites");
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let metrics_bin = read_output(&metrics_dir, item.font_id, "metrics")?;
        let sprite_bin = read_output(&sprites_dir, item.font_id, "sprite")?;
        out.push(AwtRasterOutput {
            font_id: item.font_id,
            face_id: item.face_id,
            face_kind: item.face.kind,
            size_px: probe_size_px(item),
            metrics_bin,
            sprite_bin,
        });
    }
    Ok(out)
}

/// Read one `<id>.bin` AWT output, mapping a missing file to a clear error (the
/// rasterizer skips a font only if the worklist line was malformed).
fn read_output(dir: &Path, font_id: u32, what: &str) -> Result<Vec<u8>> {
    let path = dir.join(format!("{font_id}.bin"));
    std::fs::read(&path).map_err(|e| {
        crate::error::CacheError::message(format!(
            "font {font_id}: AWT FontRaster produced no {what} payload ({}): {e}",
            path.display()
        ))
    })
}

/// The size the report shows. `px` mode renders at this literal em size; `ascent`
/// mode is binary-searched inside AWT (FontRaster logs the chosen px), so here we
/// surface the requested target — the authoritative atlas/metrics come from the
/// `.bin` payloads regardless.
fn probe_size_px(item: &AwtWorkItem) -> f32 {
    match item.mode {
        SizeMode::Px(px) | SizeMode::Ascent(px) => px,
    }
}
