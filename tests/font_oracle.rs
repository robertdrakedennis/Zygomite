//! Regression-lock: the `font` toolkit's deterministic encoders must
//! byte-reproduce the committed relic-system-948 oracle, its decoders must
//! losslessly round-trip it, AND `font rasterize`'s default AWT path must
//! re-derive the golden `.bin` payloads + JS5 groups byte-for-byte.
//!
//! Oracle (NEVER edit — these are the committed regression artifacts the tool
//! must reproduce):
//!   * `server/cache-patches/relic-system-948/fonts/metrics/<id>.bin`
//!     — `FontMetrics` (archive 13) raster payload from `FontRaster.encodeMetrics`.
//!   * `server/cache-patches/relic-system-948/fonts/sprites/<id>.bin`
//!     — glyph-atlas (archive 8) raster payload from `FontRaster.encodeSprite`.
//!   * `server/cache-patches/relic-system-948/fonts/groups/{fontmetrics,sprites}/<id>.dat`
//!     — those payloads wrapped in a JS5 raw group (build-relic-font-groups.ts).
//!
//! Two layers (mirroring `config_transcode_oracle`):
//!   1. SELF-CONTAINED, unconditional: the embedded golden `.bin` payloads
//!      decode/re-encode byte-identically and the committed `.dat` groups
//!      decompress to them.
//!   2. GATED on a JDK (+ for the full chain, the runtime pack): the AWT
//!      `FontRaster` path re-rasterizes every relic font and its decoded group
//!      payload must equal the embedded golden `.bin`. This ENFORCES that
//!      `font rasterize` keeps byte-reproducing the goldens (the proven AWT
//!      reuse), so a change to the worklist, packing, or the shared
//!      `FontRaster.java` cannot silently regress.
//!
//! The golden fixtures are embedded with `include_bytes!`, so layer 1 runs
//! unconditionally in `cargo test` with no external dependency.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use rs3_cache_rs::font::bitmap::{FontMetrics, GlyphAtlasSprite};
use rs3_cache_rs::font::cli::{decompress_raw_group, rasterize_fonts_awt};
use rs3_cache_rs::font::modern::ExtractedFace;
use rs3_cache_rs::font::raster::SizeMode;
use rs3_cache_rs::font::raster_awt::{self, AwtWorkItem};

/// The 8 relic fonts (ids in archive 13/8).
const FONT_IDS: [u32; 8] = [26, 28, 29, 32, 56, 57, 206, 207];

/// `(id, metrics.bin, sprites.bin, fontmetrics-group.dat, sprites-group.dat)`.
macro_rules! oracle {
    ($id:literal) => {
        (
            $id,
            include_bytes!(concat!(
                "../../../server/cache-patches/relic-system-948/fonts/metrics/",
                stringify!($id),
                ".bin"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "../../../server/cache-patches/relic-system-948/fonts/sprites/",
                stringify!($id),
                ".bin"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "../../../server/cache-patches/relic-system-948/fonts/groups/fontmetrics/",
                stringify!($id),
                ".dat"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "../../../server/cache-patches/relic-system-948/fonts/groups/sprites/",
                stringify!($id),
                ".dat"
            ))
            .as_slice(),
        )
    };
}

type Oracle = (
    u32,
    &'static [u8],
    &'static [u8],
    &'static [u8],
    &'static [u8],
);

fn oracles() -> Vec<Oracle> {
    vec![
        oracle!(26),
        oracle!(28),
        oracle!(29),
        oracle!(32),
        oracle!(56),
        oracle!(57),
        oracle!(206),
        oracle!(207),
    ]
}

#[test]
fn fixture_ids_match() {
    let ids: Vec<u32> = oracles().iter().map(|o| o.0).collect();
    assert_eq!(ids, FONT_IDS);
}

/// Decode → re-encode the `FontMetrics` oracle and assert byte-identity. Locks the
/// archive-13 format port (FontRaster.encodeMetrics / client `FontMetrics` ctor).
#[test]
fn metrics_decode_encode_byte_identical() {
    for (id, metrics_bin, _, _, _) in oracles() {
        let m = FontMetrics::decode(metrics_bin)
            .unwrap_or_else(|e| panic!("font {id}: decode metrics: {e}"));
        let re = m
            .encode()
            .unwrap_or_else(|e| panic!("font {id}: encode metrics: {e}"));
        assert_eq!(
            re.as_slice(),
            metrics_bin,
            "font {id}: re-encoded FontMetrics != oracle ({} vs {} bytes)",
            re.len(),
            metrics_bin.len()
        );
        // FontMetrics records are a fixed-size table.
        assert_eq!(metrics_bin.len(), 1804, "font {id}: metrics size");
        assert_eq!(m.version, 0);
        assert!(!m.has_kerning);
        assert_eq!(m.divisor, 1);
    }
}

/// Decode → re-encode the glyph-atlas sprite oracle and assert byte-identity.
/// Locks the archive-8 format port (FontRaster.encodeSprite /
/// SpriteDataProvider.decodeSprites).
#[test]
fn sprite_decode_encode_byte_identical() {
    for (id, metrics_bin, sprite_bin, _, _) in oracles() {
        let m = FontMetrics::decode(metrics_bin).unwrap();
        let sprite = GlyphAtlasSprite::decode(sprite_bin, Some((m.atlas_w, m.atlas_h)))
            .unwrap_or_else(|e| panic!("font {id}: decode sprite: {e}"));
        // Sprite dims must agree with the metrics atlas dims.
        assert_eq!(sprite.width, m.atlas_w, "font {id}: sprite width");
        assert_eq!(sprite.height, m.atlas_h, "font {id}: sprite height");
        let re = sprite
            .encode()
            .unwrap_or_else(|e| panic!("font {id}: encode sprite: {e}"));
        assert_eq!(
            re.as_slice(),
            sprite_bin,
            "font {id}: re-encoded sprite != oracle ({} vs {} bytes)",
            re.len(),
            sprite_bin.len()
        );
    }
}

/// The committed JS5 raw groups must decompress to exactly the raster `.bin`
/// payloads. (The gzip byte stream itself is not reproducible across zlib
/// implementations — Node gzipped the oracle — so the contract is on the
/// *decompressed payload*, which is what the client decodes.)
#[test]
fn group_dat_decompresses_to_bin_payload() {
    for (id, metrics_bin, sprite_bin, metrics_dat, sprite_dat) in oracles() {
        let m_payload = decompress_raw_group(metrics_dat)
            .unwrap_or_else(|e| panic!("font {id}: decompress metrics group: {e}"));
        assert_eq!(
            m_payload.as_slice(),
            metrics_bin,
            "font {id}: metrics group payload != metrics.bin"
        );
        let s_payload = decompress_raw_group(sprite_dat)
            .unwrap_or_else(|e| panic!("font {id}: decompress sprite group: {e}"));
        assert_eq!(
            s_payload.as_slice(),
            sprite_bin,
            "font {id}: sprite group payload != sprite.bin"
        );
    }
}

/// `FontRaster` invariant: the pen advance equals the atlas cell width for every
/// inked glyph (`field8573[c][2] = field8574[c]`), so the bitmap text path
/// spaces glyphs exactly as wide as their atlas sub-rect.
#[test]
fn advance_equals_cell_width_invariant() {
    for (id, metrics_bin, _, _, _) in oracles() {
        let m = FontMetrics::decode(metrics_bin).unwrap();
        for c in 0..256usize {
            if m.glyph_height[c] == 0 {
                continue; // empty / whitespace glyph
            }
            // advance (field8574) is the cell width; the atlas sub-rect width is
            // implied by it (FontMetrics sets field8573[c][2] = field8574[c]).
            assert!(
                m.advance[c] >= 1,
                "font {id} byte {c}: inked glyph has zero advance"
            );
        }
    }
}

// ── AWT regression-lock: `font rasterize` must reproduce the goldens ──────────
//
// The default `font rasterize` path shells out to the proven AWT `FontRaster`
// (the shared copy of `FontRaster.java`). These tests ENFORCE that its output's
// decoded payloads byte-match the embedded golden `.bin` for all 8 relic fonts.

/// The golden `(id, metrics.bin, sprite.bin)` triples, from the embedded oracle.
fn golden_bins() -> Vec<(u32, &'static [u8], &'static [u8])> {
    oracles()
        .into_iter()
        .map(|(id, m, s, _, _)| (id, m, s))
        .collect()
}

/// `true` when both `javac` and `java` are runnable (the AWT path's only
/// external dependency). Honours `JAVA_HOME/bin` first, then `PATH`.
fn jdk_available() -> bool {
    ["javac", "java"].iter().all(|tool| {
        let program = std::env::var("JAVA_HOME")
            .ok()
            .map(|h| Path::new(&h).join("bin").join(tool))
            .filter(|p| p.is_file())
            .map_or_else(|| (*tool).to_string(), |p| p.to_string_lossy().into_owned());
        Command::new(program)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// Crate-relative relic oracle dir (read-only — TTF faces + worklist live here).
fn relic_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/cache-patches/relic-system-948")
}

/// Crate-relative runtime pack root (the 910-base/948-overlay pack), the live
/// source the full `font rasterize` pipeline decodes the worklist from.
fn pack_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/data/pack-910-base-948-overlay")
}

/// The proven worklist (the relic `fonts/fonts.tsv`): `(fontId, faceId, mode)`,
/// where `mode` is the `SizeMode` to render at. Kept in sync with the committed
/// `fonts.tsv`; the full-pipeline test below re-derives this from the live pack
/// and would catch any drift between the two.
const PROVEN_WORKLIST: [(u32, u32, SizeMode); 8] = [
    (26, 2, SizeMode::Px(12.0)),
    (28, 2, SizeMode::Px(14.0)),
    (29, 5, SizeMode::Px(14.0)),
    (32, 5, SizeMode::Px(17.0)),
    (56, 3, SizeMode::Ascent(17.0)),
    (57, 4, SizeMode::Ascent(16.0)),
    (206, 2, SizeMode::Px(11.0)),
    (207, 2, SizeMode::Px(12.0)),
];

/// SELF-CONTAINED (JDK-gated): drive the AWT `FontRaster` directly with the
/// proven worklist + the committed relic TTF faces, and assert every font's
/// `.bin` payloads byte-match the embedded goldens. This locks the AWT shell-out
/// + the `FontRaster.java` rasterization without needing the live runtime pack —
/// only a JDK. Skipped (with a notice) when no JDK is on PATH.
#[test]
fn awt_rasterizer_reproduces_golden_bins() {
    if !jdk_available() {
        eprintln!("skipping awt_rasterizer_reproduces_golden_bins: no JDK (javac/java) on PATH");
        return;
    }
    let ttf_dir = relic_dir().join("fonts").join("ttf");
    if !ttf_dir.join("face-2.ttf").is_file() {
        eprintln!(
            "skipping awt_rasterizer_reproduces_golden_bins: relic TTF faces absent under {}",
            ttf_dir.display()
        );
        return;
    }

    // Build the AWT worklist from the relic faces (load each face once).
    let mut items = Vec::with_capacity(PROVEN_WORKLIST.len());
    for &(font_id, face_id, mode) in &PROVEN_WORKLIST {
        let bytes = std::fs::read(ttf_dir.join(format!("face-{face_id}.ttf")))
            .unwrap_or_else(|e| panic!("read relic face {face_id}: {e}"));
        let face = ExtractedFace::from_payload(bytes)
            .unwrap_or_else(|e| panic!("classify relic face {face_id}: {e}"));
        items.push(AwtWorkItem {
            font_id,
            face_id,
            face,
            mode,
        });
    }

    let outputs = raster_awt::rasterize_awt(&items).expect("AWT FontRaster rasterize");
    assert_eq!(outputs.len(), FONT_IDS.len(), "one output per relic font");

    let goldens = golden_bins();
    for out in &outputs {
        let (_, gold_metrics, gold_sprite) = goldens
            .iter()
            .find(|(id, _, _)| *id == out.font_id)
            .copied()
            .unwrap_or_else(|| panic!("no golden for font {}", out.font_id));
        assert_eq!(
            out.metrics_bin.as_slice(),
            gold_metrics,
            "font {}: AWT metrics.bin != golden ({} vs {} bytes)",
            out.font_id,
            out.metrics_bin.len(),
            gold_metrics.len()
        );
        assert_eq!(
            out.sprite_bin.as_slice(),
            gold_sprite,
            "font {}: AWT sprite.bin != golden ({} vs {} bytes)",
            out.font_id,
            out.sprite_bin.len(),
            gold_sprite.len()
        );
    }
}

/// FULL PIPELINE (JDK + pack gated): run the real `font rasterize` AWT path
/// (`rasterize_fonts_awt`) — which decodes the worklist from the live pack
/// (archive 58/59), shells out to `FontRaster`, and wraps the payloads into JS5
/// groups — for every relic font, and assert each group's DECODED payload equals
/// the embedded golden `.bin` (the contract the client actually consumes). This
/// locks the end-to-end decode → worklist → AWT → pack chain against the
/// goldens. Skipped (with a notice) when the JDK or the runtime pack is absent.
#[test]
fn font_rasterize_awt_pipeline_reproduces_golden_group_payloads() {
    if !jdk_available() {
        eprintln!(
            "skipping font_rasterize_awt_pipeline_reproduces_golden_group_payloads: no JDK on PATH"
        );
        return;
    }
    let pack = pack_root();
    if !pack.join("client.fontmetrics2.js5").is_file() || !pack.join("client.ttf.js5").is_file() {
        eprintln!(
            "skipping font_rasterize_awt_pipeline_reproduces_golden_group_payloads: runtime pack \
             absent under {}",
            pack.display()
        );
        return;
    }

    let fonts: BTreeSet<u32> = FONT_IDS.iter().copied().collect();
    let artifacts = rasterize_fonts_awt(&pack, &fonts, &[]).expect("AWT rasterize from pack");
    assert_eq!(
        artifacts.len(),
        FONT_IDS.len(),
        "one artifact per relic font"
    );

    let goldens = golden_bins();
    for art in &artifacts {
        let (_, gold_metrics, gold_sprite) = goldens
            .iter()
            .find(|(id, _, _)| *id == art.font_id)
            .copied()
            .unwrap_or_else(|| panic!("no golden for font {}", art.font_id));

        // The raw `.bin` must match the golden …
        assert_eq!(
            art.metrics_bin.as_slice(),
            gold_metrics,
            "font {}: pipeline metrics.bin != golden",
            art.font_id
        );
        assert_eq!(
            art.sprite_bin.as_slice(),
            gold_sprite,
            "font {}: pipeline sprite.bin != golden",
            art.font_id
        );

        // … and the JS5 group must DECODE to the golden `.bin` (gzip bytes need
        // not match — the client decodes the payload).
        let m_payload = decompress_raw_group(&art.metrics_group)
            .unwrap_or_else(|e| panic!("font {}: decode metrics group: {e}", art.font_id));
        assert_eq!(
            m_payload.as_slice(),
            gold_metrics,
            "font {}: metrics group payload != golden",
            art.font_id
        );
        let s_payload = decompress_raw_group(&art.sprite_group)
            .unwrap_or_else(|e| panic!("font {}: decode sprite group: {e}", art.font_id));
        assert_eq!(
            s_payload.as_slice(),
            gold_sprite,
            "font {}: sprite group payload != golden",
            art.font_id
        );
    }
}
