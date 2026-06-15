//! `rs3-cache-rs font …` subcommand: decode / preview / rasterize / diff the
//! 910 bitmap fonts and the 948 modern (TTF) font system.
//!
//! Promotes the one-off relic-system-948 `build-fonts.ts` → `FontRaster` →
//! `build-relic-font-groups.ts` pipeline and the `FontVerify` round-trip into a
//! reusable command. Reads the live runtime pack (default the 910-base/948
//! overlay pack the client runs) for archives 8 (glyph atlas), 13 (FontMetrics),
//! 58 (FontMetrics2 ref), 59 (Ttf), and the interfaces archive (for `--interface`
//! font discovery).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde::Serialize;

use crate::error::{Context, Result};
use crate::font::bitmap::{FontMetrics, GlyphAtlasSprite};
use crate::font::modern::{ExtractedFace, FaceKind, ModernFontRef};
use crate::font::raster::{self, RasterFont, SizeMode};
use crate::font::raster_awt;
use crate::interface::parse_component_deps;
use crate::js5::decompress;
use crate::js5pack::PackArchive;
use crate::{cache_bail, font::GLYPH_COUNT};

/// Bitmap-font archive ids in the 910 numbering.
const ARCHIVE_BITMAP_FONTMETRICS: u32 = 13;
const ARCHIVE_GLYPH_ATLAS: u32 = 8;
/// Modern-font archive ids in the 948 numbering.
const ARCHIVE_FONTMETRICS2: u32 = 58;
const ARCHIVE_TTF: u32 = 59;
const ARCHIVE_INTERFACES: u32 = 3;

/// Default runtime pack root. Reuses `explain-interface`'s constant so the two
/// commands agree (the previous `../server/...` literal was one `../` short of
/// the crate working dir → the default pack was never found). When a requested
/// group (font/interface) is absent from this pack, the readers fall back to the
/// donor pack via [`crate::pack_root::resolve_noting`] (noted once on stderr),
/// so donor-only fonts/interfaces work without an explicit `--pack-root`.
const DEFAULT_PACK_ROOT: &str = crate::explain::DEFAULT_PACK_ROOT;
const DEFAULT_SAMPLE: &str = "Relic Powers 50/500 - Font of Life";

/// `.js5` pack file names by archive, in the runtime pack root.
fn pack_file_for(pack_root: &Path, archive: u32) -> Result<PathBuf> {
    let name = match archive {
        ARCHIVE_GLYPH_ATLAS => "client.sprites.js5",
        ARCHIVE_BITMAP_FONTMETRICS => "client.fontmetrics.js5",
        ARCHIVE_FONTMETRICS2 => "client.fontmetrics2.js5",
        ARCHIVE_TTF => "client.ttf.js5",
        ARCHIVE_INTERFACES => "client.interfaces.js5",
        other => cache_bail!("no pack file mapping for archive {other}"),
    };
    Ok(pack_root.join(name))
}

#[derive(Subcommand, Debug)]
pub enum FontCommand {
    /// Structured dump of a font group: FontMetrics (13), glyph atlas (8),
    /// modern reference (58), or TTF header (59).
    Decode {
        /// Cache archive id: 13 (FontMetrics) | 8 (glyph atlas) | 58
        /// (FontMetrics2 ref) | 59 (Ttf).
        #[arg(long)]
        archive: u32,
        /// Group (font/face) id.
        #[arg(long)]
        group: u32,
        /// Runtime pack root holding the `client.*.js5` files. READ-ONLY.
        #[arg(long, default_value = DEFAULT_PACK_ROOT)]
        pack_root: PathBuf,
        /// Emit the dump as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Atlas + sample-string PNG round-trip (the `FontVerify` check) for a font.
    ///
    /// By default reads the already-built bitmap groups (archive 13 + 8). With
    /// `--from-modern` it instead resolves the archive-58 reference, extracts
    /// the TTF (archive 59) and rasterizes it fresh (the full pipeline preview).
    Preview {
        /// Font id (group in archive 13/8, and in archive 58 when --from-modern).
        #[arg(long)]
        font: u32,
        /// Sample string to render.
        #[arg(long, default_value = DEFAULT_SAMPLE)]
        text: String,
        /// Output sample PNG path. An `atlas-*.png` sibling is written too.
        #[arg(long)]
        out: PathBuf,
        /// Rasterize from the modern archive-58/59 reference instead of reading
        /// the pre-built bitmap groups.
        #[arg(long)]
        from_modern: bool,
        /// For `--from-modern` fmt=1 fonts (56/57), the face id to substitute
        /// (Cinzel). Defaults: 56→3, 57→4.
        #[arg(long)]
        face: Option<u32>,
        #[arg(long, default_value = DEFAULT_PACK_ROOT)]
        pack_root: PathBuf,
    },
    /// Rasterize every modern font an interface references into 910 bitmap-font
    /// `.bin` payloads (FontMetrics[13] + glyph atlas[8]) and JS5 groups. One
    /// command for the `build-fonts.ts → FontRaster → build-relic-font-groups.ts`
    /// pipeline.
    ///
    /// By default this shells out to the proven AWT rasterizer (the shared
    /// `FontRaster.java`, requires a JDK on PATH) so the output byte-matches the
    /// committed golden font groups. `--experimental` swaps in the pure-Rust
    /// `ab_glyph` rasterizer (no JDK, but its anti-aliasing does NOT match AWT,
    /// so it does not reproduce the goldens).
    Rasterize {
        /// Interface id whose text components are scanned for font references.
        #[arg(long)]
        interface: Option<u32>,
        /// Rasterize a single explicit font id instead of (or in addition to)
        /// an interface's references.
        #[arg(long)]
        font: Vec<u32>,
        /// Output directory for `<id>.metrics.bin` / `<id>.sprite.bin` and the
        /// JS5 `groups/{fontmetrics,sprites}/<id>.dat(+.metadata.json)`.
        #[arg(long)]
        out_dir: PathBuf,
        /// Suppress the JS5-wrapped raw groups (groups are written by default).
        #[arg(long)]
        no_groups: bool,
        /// Use the experimental pure-Rust `ab_glyph` rasterizer instead of the
        /// AWT one. No JDK required, but the output does NOT byte-match the
        /// golden font groups (the Rust anti-aliasing differs from AWT).
        #[arg(long)]
        experimental: bool,
        /// For fmt=1 fonts with no face reference, the face assignment as
        /// `font:face` pairs (default 56:3,57:4).
        #[arg(long, value_delimiter = ',')]
        fmt1_face: Vec<String>,
        #[arg(long, default_value = DEFAULT_PACK_ROOT)]
        pack_root: PathBuf,
    },
    /// Visual + metric diff of two bitmap fonts (by id, from the pre-built
    /// groups). Reports per-glyph advance/height/offset deltas and atlas size.
    Diff {
        /// First font id.
        #[arg(long)]
        a: u32,
        /// Second font id.
        #[arg(long)]
        b: u32,
        #[arg(long, default_value = DEFAULT_PACK_ROOT)]
        pack_root: PathBuf,
        /// Emit the diff as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Read a single-file group payload (decompressed) from a runtime `.js5` pack.
/// Falls back to the donor pack (noting it once) when `pack_root` lacks the
/// group, so donor-only fonts/interfaces resolve without an explicit
/// `--pack-root`.
fn read_single_file_group(pack_root: &Path, archive: u32, group: u32) -> Result<Vec<u8>> {
    let pack_file = pack_file_for(pack_root, archive)?;
    let pack_name = pack_file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let pack_path = crate::pack_root::resolve_noting(pack_root, pack_name, group)?;
    let pack = PackArchive::open(&pack_path)
        .with_context(|| format!("open pack {}", pack_path.display()))?;
    let files = pack
        .group_files(group)?
        .ok_or_else(|| crate::error::CacheError::message(format!(
            "archive {archive} group {group} absent in {}",
            pack_path.display()
        )))?;
    files
        .into_iter()
        .next()
        .map(|(_, bytes)| bytes)
        .ok_or_else(|| crate::error::CacheError::message(format!(
            "archive {archive} group {group} has no files"
        )))
}

/// Default Cinzel face assignment for fmt=1 fonts (mirrors `build-fonts.ts`
/// `FMT1_FACE`): 56 → Cinzel Bold (face 3), 57 → Cinzel Regular (face 4).
fn default_fmt1_face(font: u32) -> Option<u32> {
    match font {
        56 => Some(3),
        57 => Some(4),
        _ => None,
    }
}

/// Resolve a modern font id to (face bytes, size mode), reading archive 58 then
/// 59. `face_override` forces the face for fmt=1 (or fmt=2 if explicitly given).
fn resolve_modern_font(
    pack_root: &Path,
    font: u32,
    face_override: Option<u32>,
) -> Result<(ExtractedFace, SizeMode, ModernFontRef)> {
    let ref_payload = read_single_file_group(pack_root, ARCHIVE_FONTMETRICS2, font)?;
    let font_ref = ModernFontRef::decode(&ref_payload)?;
    let (face_id, mode) = match font_ref {
        ModernFontRef::TtfRef { face, size_px } => {
            (face_override.unwrap_or(face), SizeMode::Px(f32::from(size_px)))
        }
        ModernFontRef::Fmt1Bitmap { target_ascent, .. } => {
            let face = face_override
                .or_else(|| default_fmt1_face(font))
                .ok_or_else(|| crate::error::CacheError::message(format!(
                    "font {font}: fmt=1 has no face reference; pass --face / --fmt1-face"
                )))?;
            (face, SizeMode::Ascent(f32::from(target_ascent)))
        }
    };
    let face_payload = read_single_file_group(pack_root, ARCHIVE_TTF, face_id)?;
    let face = ExtractedFace::from_payload(face_payload)
        .with_context(|| format!("font {font} face {face_id}"))?;
    Ok((face, mode, font_ref))
}

/// Discover the modern font ids an interface's text components reference.
/// Falls back to the donor interfaces pack (noting it) for donor-only ids.
fn discover_interface_fonts(pack_root: &Path, interface: u32) -> Result<BTreeSet<u32>> {
    let pack_name = pack_file_for(pack_root, ARCHIVE_INTERFACES)?;
    let pack_name = pack_name
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let pack_path = crate::pack_root::resolve_noting(pack_root, pack_name, interface)?;
    let pack = PackArchive::open(&pack_path)
        .with_context(|| format!("open interfaces pack {}", pack_path.display()))?;
    let files = pack
        .group_files(interface)?
        .ok_or_else(|| crate::error::CacheError::message(format!(
            "interface {interface} absent in {}",
            pack_path.display()
        )))?;
    let mut fonts = BTreeSet::new();
    for (component_id, bytes) in files {
        // Build 910 is the runtime base; the codec branches on build internally.
        let deps = parse_component_deps(component_id, &bytes, crate::constants::BUILD)?;
        fonts.extend(deps.fontmetrics);
    }
    Ok(fonts)
}

// ── JSON dump shapes ──

#[derive(Serialize)]
#[serde(tag = "kind")]
enum DecodeDump {
    #[serde(rename = "fontmetrics")]
    Metrics(MetricsDump),
    #[serde(rename = "glyph_atlas")]
    Atlas(AtlasDump),
    #[serde(rename = "modern_ref")]
    ModernRef(ModernRefDump),
    #[serde(rename = "ttf")]
    Ttf(TtfDump),
}

#[derive(Serialize)]
struct MetricsDump {
    archive: u32,
    group: u32,
    version: u8,
    has_kerning: bool,
    atlas_w: u16,
    atlas_h: u16,
    line_height: u8,
    ascent: u8,
    descent: u8,
    divisor: u8,
    ink_glyphs: u32,
    /// Sample of printable-ASCII glyph metrics, keyed by byte.
    sample_glyphs: Vec<GlyphDump>,
}

#[derive(Serialize)]
struct GlyphDump {
    byte: u8,
    ch: Option<char>,
    advance: u8,
    height: u8,
    y_offset: u8,
    atlas_x: u16,
    atlas_y: u16,
}

#[derive(Serialize)]
struct AtlasDump {
    archive: u32,
    group: u32,
    width: u16,
    height: u16,
    nonzero_coverage: usize,
}

#[derive(Serialize)]
struct ModernRefDump {
    archive: u32,
    group: u32,
    fmt: u8,
    face: Option<u32>,
    size_px: Option<u8>,
    target_ascent: Option<u8>,
    ascent: Option<u8>,
    descent: Option<u8>,
    divisor: Option<u8>,
    line_height: Option<u8>,
}

#[derive(Serialize)]
struct TtfDump {
    archive: u32,
    group: u32,
    kind: &'static str,
    bytes: usize,
}

fn metrics_dump(archive: u32, group: u32, m: &FontMetrics) -> MetricsDump {
    let ink_glyphs = m.glyph_height.iter().filter(|&&h| h > 0).count() as u32;
    let mut sample_glyphs = Vec::new();
    for &byte in &[b'A', b'a', b'M', b'g', b'0', b'.', b' '] {
        let c = byte as usize;
        sample_glyphs.push(GlyphDump {
            byte,
            ch: crate::font::cp1252_byte_to_char(byte),
            advance: m.advance[c],
            height: m.glyph_height[c],
            y_offset: m.y_offset[c],
            atlas_x: m.atlas_x[c],
            atlas_y: m.atlas_y[c],
        });
    }
    MetricsDump {
        archive,
        group,
        version: m.version,
        has_kerning: m.has_kerning,
        atlas_w: m.atlas_w,
        atlas_h: m.atlas_h,
        line_height: m.line_height,
        ascent: m.ascent,
        descent: m.descent,
        divisor: m.divisor,
        ink_glyphs,
        sample_glyphs,
    }
}

fn run_decode(archive: u32, group: u32, pack_root: &Path, json: bool) -> Result<()> {
    let payload = read_single_file_group(pack_root, archive, group)?;
    let dump = match archive {
        ARCHIVE_BITMAP_FONTMETRICS => {
            let m = FontMetrics::decode(&payload)?;
            DecodeDump::Metrics(metrics_dump(archive, group, &m))
        }
        ARCHIVE_GLYPH_ATLAS => {
            let sprite = GlyphAtlasSprite::decode(&payload, None)?;
            let nonzero = sprite.alpha.iter().filter(|&&a| a != 0).count();
            DecodeDump::Atlas(AtlasDump {
                archive,
                group,
                width: sprite.width,
                height: sprite.height,
                nonzero_coverage: nonzero,
            })
        }
        ARCHIVE_FONTMETRICS2 => {
            let r = ModernFontRef::decode(&payload)?;
            let dump = match r {
                ModernFontRef::TtfRef { face, size_px } => ModernRefDump {
                    archive,
                    group,
                    fmt: 2,
                    face: Some(face),
                    size_px: Some(size_px),
                    target_ascent: None,
                    ascent: None,
                    descent: None,
                    divisor: None,
                    line_height: None,
                },
                ModernFontRef::Fmt1Bitmap {
                    target_ascent,
                    ascent,
                    descent,
                    divisor,
                    line_height,
                } => ModernRefDump {
                    archive,
                    group,
                    fmt: 1,
                    face: None,
                    size_px: None,
                    target_ascent: Some(target_ascent),
                    ascent: Some(ascent),
                    descent: Some(descent),
                    divisor: Some(divisor),
                    line_height: Some(line_height),
                },
            };
            DecodeDump::ModernRef(dump)
        }
        ARCHIVE_TTF => {
            let face = ExtractedFace::from_payload(payload)?;
            let kind = match face.kind {
                FaceKind::TrueType => "truetype",
                FaceKind::OpenTypeCff => "opentype-cff",
            };
            DecodeDump::Ttf(TtfDump {
                archive,
                group,
                kind,
                bytes: face.bytes.len(),
            })
        }
        other => cache_bail!("font decode: unsupported archive {other} (use 8|13|58|59)"),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&dump)?);
    } else {
        print_decode_human(&dump);
    }
    Ok(())
}

fn print_decode_human(dump: &DecodeDump) {
    match dump {
        DecodeDump::Metrics(m) => {
            println!(
                "FontMetrics[{}] group {}: atlas {}x{} ascent={} descent={} line={} divisor={} ink_glyphs={}",
                m.archive, m.group, m.atlas_w, m.atlas_h, m.ascent, m.descent, m.line_height, m.divisor, m.ink_glyphs
            );
            for g in &m.sample_glyphs {
                println!(
                    "  byte {:#04x} {:?}: adv={} h={} yoff={} atlas=({},{})",
                    g.byte, g.ch, g.advance, g.height, g.y_offset, g.atlas_x, g.atlas_y
                );
            }
        }
        DecodeDump::Atlas(a) => println!(
            "GlyphAtlas[{}] group {}: {}x{}, {} inked px",
            a.archive, a.group, a.width, a.height, a.nonzero_coverage
        ),
        DecodeDump::ModernRef(r) => {
            if r.fmt == 2 {
                println!(
                    "FontMetrics2[{}] group {}: fmt=2 → face {} @ {}px",
                    r.archive,
                    r.group,
                    r.face.unwrap_or(0),
                    r.size_px.unwrap_or(0)
                );
            } else {
                println!(
                    "FontMetrics2[{}] group {}: fmt=1 bitmap, target_ascent={} (donor ascent {}/{} line={})",
                    r.archive,
                    r.group,
                    r.target_ascent.unwrap_or(0),
                    r.ascent.unwrap_or(0),
                    r.divisor.unwrap_or(1),
                    r.line_height.unwrap_or(0)
                );
            }
        }
        DecodeDump::Ttf(t) => println!(
            "Ttf[{}] group {}: {} face, {} bytes",
            t.archive, t.group, t.kind, t.bytes
        ),
    }
}

fn write_png(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let buf: image::RgbImage = image::ImageBuffer::from_raw(width, height, rgb.to_vec())
        .ok_or_else(|| crate::error::CacheError::message("RGB buffer size mismatch for PNG"))?;
    buf.save(path)
        .map_err(|e| crate::error::CacheError::message(format!("save {}: {e}", path.display())))?;
    Ok(())
}

/// Sibling atlas path for a sample `out` path: `dir/atlas-<stem>.png`.
fn atlas_sibling(out: &Path) -> PathBuf {
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("font");
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("atlas-{stem}.png"))
}

fn run_preview(
    font: u32,
    text: &str,
    out: &Path,
    from_modern: bool,
    face: Option<u32>,
    pack_root: &Path,
) -> Result<()> {
    let (metrics, atlas) = if from_modern {
        let (face_bytes, mode, _) = resolve_modern_font(pack_root, font, face)?;
        let raster = raster::rasterize(&face_bytes.bytes, mode)?;
        let m = raster.to_metrics()?;
        let atlas = raster.atlas_alpha().to_vec();
        (m, atlas)
    } else {
        let m_bytes = read_single_file_group(pack_root, ARCHIVE_BITMAP_FONTMETRICS, font)?;
        let m = FontMetrics::decode(&m_bytes)?;
        let s_bytes = read_single_file_group(pack_root, ARCHIVE_GLYPH_ATLAS, font)?;
        let sprite = GlyphAtlasSprite::decode(&s_bytes, Some((m.atlas_w, m.atlas_h)))?;
        (m, sprite.alpha)
    };

    let (sw, sh, srgb) = raster::render_sample(&metrics, &atlas, text)?;
    write_png(out, sw, sh, &srgb)?;
    let (aw, ah, argb) = raster::render_atlas(&metrics, &atlas)?;
    let atlas_path = atlas_sibling(out);
    write_png(&atlas_path, aw, ah, &argb)?;

    println!(
        "font {font}: atlas {}x{} ascent={} descent={} line={}  sample {sw}x{sh}\n  sample → {}\n  atlas  → {}",
        metrics.atlas_w,
        metrics.atlas_h,
        metrics.ascent,
        metrics.descent,
        metrics.line_height,
        out.display(),
        atlas_path.display()
    );
    Ok(())
}

/// Wrap a raw bitmap-font payload into a JS5 raw group identical (after
/// decompression) to the relic overlay artifacts: a JS5 container (compression
/// 2 = gzip) of the single-file payload, plus a 2-byte big-endian version=1
/// trailer. NB: the gzip byte stream is not reproducible across zlib
/// implementations (the committed oracle `.dat` was gzipped by Node), so this
/// matches the oracle on the *decompressed payload*, not the compressed bytes —
/// `decompress(group)` round-trips to the input. See `font::tests`.
fn wrap_raw_group(payload: &[u8], version: u16) -> Result<Vec<u8>> {
    use std::io::Write as _;
    let mut encoder =
        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(payload)?;
    let mut gz = encoder.finish()?;
    // Js5.packGroup zeroes the gzip OS byte (offset 9).
    if gz.len() > 9 {
        gz[9] = 0;
    }
    let ulen = u32::try_from(payload.len()).context("payload too large")?;
    let clen = u32::try_from(gz.len()).context("compressed too large")?;
    let mut out = Vec::with_capacity(gz.len() + 11);
    out.push(2); // compression: gzip
    out.extend_from_slice(&clen.to_be_bytes());
    out.extend_from_slice(&ulen.to_be_bytes());
    out.extend_from_slice(&gz);
    out.extend_from_slice(&version.to_be_bytes());
    Ok(out)
}

/// The single-file group metadata `build-relic-font-groups.ts` writes next to
/// each `.dat` (`{groupSize:1, groupCapacity:1, fileIds:[0]}`, 2-space pretty).
const SINGLE_FILE_GROUP_META: &str =
    "{\n  \"groupSize\": 1,\n  \"groupCapacity\": 1,\n  \"fileIds\": [\n    0\n  ]\n}";

/// Write the two `.bin` payloads for a font plus (unless suppressed) the JS5
/// `groups/{fontmetrics,sprites}/<id>.dat(+.metadata.json)` — exactly the files
/// `build-relic-font-groups.ts` emits, so the overlay can import them directly.
fn write_font_outputs(
    out_dir: &Path,
    font: u32,
    metrics_bytes: &[u8],
    sprite_bytes: &[u8],
    groups: bool,
) -> Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(out_dir)?;
    let metrics_path = out_dir.join(format!("{font}.metrics.bin"));
    let sprite_path = out_dir.join(format!("{font}.sprite.bin"));
    std::fs::write(&metrics_path, metrics_bytes)?;
    std::fs::write(&sprite_path, sprite_bytes)?;

    if groups {
        let metrics_grp_dir = out_dir.join("groups").join("fontmetrics");
        let sprites_grp_dir = out_dir.join("groups").join("sprites");
        std::fs::create_dir_all(&metrics_grp_dir)?;
        std::fs::create_dir_all(&sprites_grp_dir)?;
        for (dir, payload) in [
            (&metrics_grp_dir, metrics_bytes),
            (&sprites_grp_dir, sprite_bytes),
        ] {
            std::fs::write(dir.join(format!("{font}.dat")), wrap_raw_group(payload, 1)?)?;
            std::fs::write(dir.join(format!("{font}.metadata.json")), SINGLE_FILE_GROUP_META)?;
        }
    }
    Ok((metrics_path, sprite_path))
}

#[derive(Serialize)]
struct RasterReport {
    font: u32,
    face: u32,
    kind: &'static str,
    size_px: f32,
    atlas_w: i32,
    atlas_h: i32,
    ascent: i32,
    descent: i32,
    line_height: i32,
    ink_glyphs: u32,
    metrics_bin: String,
    sprite_bin: String,
}

/// Resolve the face id a font renders with (override wins, else the archive-58
/// reference / fmt=1 default), independent of the rasterizer used.
fn resolve_face_id(font: u32, font_ref: &ModernFontRef, face_override: Option<u32>) -> u32 {
    match (font_ref, face_override) {
        (_, Some(f)) => f,
        (ModernFontRef::TtfRef { face, .. }, None) => *face,
        (ModernFontRef::Fmt1Bitmap { .. }, None) => default_fmt1_face(font).unwrap_or(0),
    }
}

fn face_kind_str(kind: FaceKind) -> &'static str {
    match kind {
        FaceKind::TrueType => "truetype",
        FaceKind::OpenTypeCff => "opentype-cff",
    }
}

/// Rasterize one font with the **experimental** pure-Rust `ab_glyph` path. Does
/// NOT byte-match the golden font groups (its anti-aliasing differs from AWT).
fn rasterize_one_native(
    pack_root: &Path,
    font: u32,
    face_override: Option<u32>,
    out_dir: &Path,
    groups: bool,
) -> Result<RasterReport> {
    let (face, mode, font_ref) = resolve_modern_font(pack_root, font, face_override)?;
    let face_id = resolve_face_id(font, &font_ref, face_override);
    let raster: RasterFont = raster::rasterize(&face.bytes, mode)?;
    let metrics_bytes = raster.to_metrics()?.encode()?;
    let sprite_bytes = raster.to_sprite()?.encode()?;
    let (metrics_path, sprite_path) =
        write_font_outputs(out_dir, font, &metrics_bytes, &sprite_bytes, groups)?;
    Ok(RasterReport {
        font,
        face: face_id,
        kind: face_kind_str(face.kind),
        size_px: raster.size_px,
        atlas_w: raster.atlas_w,
        atlas_h: raster.atlas_h,
        ascent: raster.ascent,
        descent: raster.descent,
        line_height: raster.line_height,
        ink_glyphs: raster.ink_glyphs,
        metrics_bin: metrics_path.display().to_string(),
        sprite_bin: sprite_path.display().to_string(),
    })
}

/// Build the AWT worklist for `targets`: resolve each font's face TTF bytes and
/// size mode (reusing the archive-58/59 decode), so `FontRaster.java` can render
/// them. Identical resolution to the native path — only the rasterizer differs.
fn build_awt_worklist(
    pack_root: &Path,
    targets: &BTreeSet<u32>,
    face_for: &impl Fn(u32) -> Option<u32>,
) -> Result<Vec<raster_awt::AwtWorkItem>> {
    let mut items = Vec::with_capacity(targets.len());
    for &font in targets {
        let face_override = face_for(font);
        let (face, mode, font_ref) = resolve_modern_font(pack_root, font, face_override)?;
        let face_id = resolve_face_id(font, &font_ref, face_override);
        items.push(raster_awt::AwtWorkItem {
            font_id: font,
            face_id,
            face,
            mode,
        });
    }
    Ok(items)
}

/// Rasterize `targets` through the AWT `FontRaster` (the byte-faithful default),
/// writing each font's `.bin` payloads + JS5 groups and returning a report per
/// font. The metrics/atlas dims in the report are decoded from the AWT output.
fn rasterize_awt_targets(
    pack_root: &Path,
    targets: &BTreeSet<u32>,
    face_for: &impl Fn(u32) -> Option<u32>,
    out_dir: &Path,
    groups: bool,
) -> Result<Vec<RasterReport>> {
    let items = build_awt_worklist(pack_root, targets, face_for)?;
    let outputs = raster_awt::rasterize_awt(&items)?;
    let mut reports = Vec::with_capacity(outputs.len());
    for out in outputs {
        let (metrics_path, sprite_path) =
            write_font_outputs(out_dir, out.font_id, &out.metrics_bin, &out.sprite_bin, groups)?;
        // Decode the AWT FontMetrics to surface the atlas/ascent/line + ink count.
        let m = FontMetrics::decode(&out.metrics_bin)?;
        let ink_glyphs = m.glyph_height.iter().filter(|&&h| h > 0).count() as u32;
        reports.push(RasterReport {
            font: out.font_id,
            face: out.face_id,
            kind: face_kind_str(out.face_kind),
            size_px: out.size_px,
            atlas_w: i32::from(m.atlas_w),
            atlas_h: i32::from(m.atlas_h),
            ascent: i32::from(m.ascent),
            descent: i32::from(m.descent),
            line_height: i32::from(m.line_height),
            ink_glyphs,
            metrics_bin: metrics_path.display().to_string(),
            sprite_bin: sprite_path.display().to_string(),
        });
    }
    Ok(reports)
}

fn parse_fmt1_face(specs: &[String]) -> Result<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    for spec in specs {
        let (font, face) = spec
            .split_once(':')
            .ok_or_else(|| crate::error::CacheError::message(format!(
                "--fmt1-face entry '{spec}' must be font:face"
            )))?;
        out.push((font.trim().parse()?, face.trim().parse()?));
    }
    Ok(out)
}

fn run_rasterize(
    interface: Option<u32>,
    fonts: &[u32],
    out_dir: &Path,
    groups: bool,
    experimental: bool,
    fmt1_face: &[String],
    pack_root: &Path,
) -> Result<()> {
    let face_overrides = parse_fmt1_face(fmt1_face)?;
    let face_for = |font: u32| -> Option<u32> {
        face_overrides
            .iter()
            .find(|(f, _)| *f == font)
            .map(|(_, face)| *face)
    };

    let mut targets: BTreeSet<u32> = fonts.iter().copied().collect();
    if let Some(iface) = interface {
        let discovered = discover_interface_fonts(pack_root, iface)?;
        println!(
            "interface {iface}: references {} font(s): {:?}",
            discovered.len(),
            discovered.iter().collect::<Vec<_>>()
        );
        targets.extend(discovered);
    }
    if targets.is_empty() {
        cache_bail!("no fonts to rasterize (pass --interface and/or --font)");
    }

    let reports = if experimental {
        println!("rasterizer: experimental pure-Rust ab_glyph (does NOT byte-match the goldens)");
        let mut reports = Vec::with_capacity(targets.len());
        for &font in &targets {
            reports.push(rasterize_one_native(pack_root, font, face_for(font), out_dir, groups)?);
        }
        reports
    } else {
        println!("rasterizer: AWT FontRaster (byte-matches the golden font groups; needs a JDK)");
        rasterize_awt_targets(pack_root, &targets, &face_for, out_dir, groups)?
    };

    for report in &reports {
        println!(
            "font {}: face {} ({}) @ {:.1}px  atlas {}x{}  ascent={} descent={} line={}  ink_glyphs={}",
            report.font,
            report.face,
            report.kind,
            report.size_px,
            report.atlas_w,
            report.atlas_h,
            report.ascent,
            report.descent,
            report.line_height,
            report.ink_glyphs
        );
    }
    println!(
        "rasterized {} font(s){} → {}",
        reports.len(),
        if groups { " + JS5 groups" } else { "" },
        out_dir.display()
    );
    Ok(())
}

#[derive(Serialize)]
struct DiffReport {
    a: u32,
    b: u32,
    atlas_a: (u16, u16),
    atlas_b: (u16, u16),
    ascent_a: u8,
    ascent_b: u8,
    descent_a: u8,
    descent_b: u8,
    line_a: u8,
    line_b: u8,
    advance_diffs: usize,
    height_diffs: usize,
    yoffset_diffs: usize,
    max_advance_delta: i32,
    sample_glyph_diffs: Vec<GlyphDiff>,
}

#[derive(Serialize)]
struct GlyphDiff {
    byte: u8,
    ch: Option<char>,
    advance_a: u8,
    advance_b: u8,
    height_a: u8,
    height_b: u8,
    yoffset_a: u8,
    yoffset_b: u8,
}

fn run_diff(a: u32, b: u32, pack_root: &Path, json: bool) -> Result<()> {
    let ma = FontMetrics::decode(&read_single_file_group(
        pack_root,
        ARCHIVE_BITMAP_FONTMETRICS,
        a,
    )?)?;
    let mb = FontMetrics::decode(&read_single_file_group(
        pack_root,
        ARCHIVE_BITMAP_FONTMETRICS,
        b,
    )?)?;

    let (mut adv_d, mut h_d, mut y_d, mut max_adv) = (0usize, 0usize, 0usize, 0i32);
    let mut sample = Vec::new();
    for c in 0..GLYPH_COUNT {
        if ma.advance[c] != mb.advance[c] {
            adv_d += 1;
            max_adv = max_adv.max((i32::from(ma.advance[c]) - i32::from(mb.advance[c])).abs());
        }
        if ma.glyph_height[c] != mb.glyph_height[c] {
            h_d += 1;
        }
        if ma.y_offset[c] != mb.y_offset[c] {
            y_d += 1;
        }
    }
    for &byte in &[b'A', b'a', b'M', b'g', b'0'] {
        let c = byte as usize;
        sample.push(GlyphDiff {
            byte,
            ch: crate::font::cp1252_byte_to_char(byte),
            advance_a: ma.advance[c],
            advance_b: mb.advance[c],
            height_a: ma.glyph_height[c],
            height_b: mb.glyph_height[c],
            yoffset_a: ma.y_offset[c],
            yoffset_b: mb.y_offset[c],
        });
    }

    let report = DiffReport {
        a,
        b,
        atlas_a: (ma.atlas_w, ma.atlas_h),
        atlas_b: (mb.atlas_w, mb.atlas_h),
        ascent_a: ma.ascent,
        ascent_b: mb.ascent,
        descent_a: ma.descent,
        descent_b: mb.descent,
        line_a: ma.line_height,
        line_b: mb.line_height,
        advance_diffs: adv_d,
        height_diffs: h_d,
        yoffset_diffs: y_d,
        max_advance_delta: max_adv,
        sample_glyph_diffs: sample,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "diff font {a} vs {b}: atlas {}x{} vs {}x{}; ascent {}/{} descent {}/{} line {}/{}",
            report.atlas_a.0,
            report.atlas_a.1,
            report.atlas_b.0,
            report.atlas_b.1,
            report.ascent_a,
            report.ascent_b,
            report.descent_a,
            report.descent_b,
            report.line_a,
            report.line_b
        );
        println!(
            "  glyph metric diffs: advance={} height={} y_offset={} (max |Δadvance|={})",
            report.advance_diffs, report.height_diffs, report.yoffset_diffs, report.max_advance_delta
        );
    }
    Ok(())
}

/// The AWT raster + JS5-group result for one font, returned by
/// [`rasterize_fonts_awt`] for the regression-lock test (and any external reuse).
pub struct FontGroupArtifacts {
    pub font_id: u32,
    /// FontMetrics (archive 13) `.bin` payload from AWT `FontRaster`.
    pub metrics_bin: Vec<u8>,
    /// Glyph-atlas (archive 8) `.bin` payload from AWT `FontRaster`.
    pub sprite_bin: Vec<u8>,
    /// The metrics `.bin` wrapped in a JS5 raw group (version 1).
    pub metrics_group: Vec<u8>,
    /// The sprite `.bin` wrapped in a JS5 raw group (version 1).
    pub sprite_group: Vec<u8>,
}

/// Rasterize the given fonts from a runtime pack via the AWT `FontRaster` and
/// return their `.bin` payloads + JS5 raw groups — no filesystem output. This is
/// the in-memory form of `font rasterize` (AWT path): resolve the worklist from
/// archive 58/59, shell out to the shared `FontRaster.java`, and wrap each
/// payload as `build-relic-font-groups.ts` does. Used by the `font_oracle`
/// regression-lock to assert the decoded group payloads byte-match the goldens.
///
/// `fmt1_face` is the `font:face` override list (default `56:3,57:4` applied
/// automatically). Requires a JDK on `PATH` (see [`raster_awt`]).
pub fn rasterize_fonts_awt(
    pack_root: &Path,
    fonts: &BTreeSet<u32>,
    fmt1_face: &[String],
) -> Result<Vec<FontGroupArtifacts>> {
    let face_overrides = parse_fmt1_face(fmt1_face)?;
    let face_for = |font: u32| -> Option<u32> {
        face_overrides
            .iter()
            .find(|(f, _)| *f == font)
            .map(|(_, face)| *face)
    };
    let items = build_awt_worklist(pack_root, fonts, &face_for)?;
    let outputs = raster_awt::rasterize_awt(&items)?;
    outputs
        .into_iter()
        .map(|o| {
            Ok(FontGroupArtifacts {
                font_id: o.font_id,
                metrics_group: wrap_raw_group(&o.metrics_bin, 1)?,
                sprite_group: wrap_raw_group(&o.sprite_bin, 1)?,
                metrics_bin: o.metrics_bin,
                sprite_bin: o.sprite_bin,
            })
        })
        .collect()
}

/// Verify a JS5 raw group wraps a payload such that decompression round-trips,
/// used by `wrap_raw_group` callers/tests. Exposed for the regression-lock test.
pub fn decompress_raw_group(raw_group: &[u8]) -> Result<Vec<u8>> {
    if raw_group.len() < 2 {
        cache_bail!("raw group too short to hold a version trailer");
    }
    // Strip the 2-byte version trailer, then JS5-decompress the container.
    decompress(&raw_group[..raw_group.len() - 2])
}

pub fn run(cmd: &FontCommand) -> Result<()> {
    match cmd {
        FontCommand::Decode {
            archive,
            group,
            pack_root,
            json,
        } => run_decode(*archive, *group, pack_root, *json),
        FontCommand::Preview {
            font,
            text,
            out,
            from_modern,
            face,
            pack_root,
        } => run_preview(*font, text, out, *from_modern, *face, pack_root),
        FontCommand::Rasterize {
            interface,
            font,
            out_dir,
            no_groups,
            experimental,
            fmt1_face,
            pack_root,
        } => run_rasterize(
            *interface,
            font,
            out_dir,
            !*no_groups,
            *experimental,
            fmt1_face,
            pack_root,
        ),
        FontCommand::Diff {
            a,
            b,
            pack_root,
            json,
        } => run_diff(*a, *b, pack_root, *json),
    }
}
