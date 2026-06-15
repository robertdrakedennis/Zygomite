//! TTF/OTF → 910 bitmap-font rasterizer.
//!
//! A faithful Rust port of the relic-system-948 oracle `FontRaster.java`
//! (`rasterize` + `pickSizeForAscent`), substituting the AWT font scaler with
//! the pure-Rust `ab_glyph` outline rasterizer. The encoded byte layout
//! (`bitmap::FontMetrics` / `bitmap::GlyphAtlasSprite`) is identical to the
//! oracle's; the *coverage pixels* differ slightly because `ab_glyph`'s
//! anti-aliasing is not AWT's (documented, expected, and asserted only at the
//! structural/legibility level — see `font::tests`).
//!
//! Invariants preserved from FontRaster (the bitmap font path depends on them):
//!   * glyph index `c` is the Cp1252 byte (`drawChars` looks glyphs up by
//!     `Cp1252.encode(char)`), so byte `c` renders the windows-1252 character;
//!   * `advance == atlas cell width == field8574` (FontMetrics sets
//!     `field8573[c][2] = field8574[c]`);
//!   * `field8565 = lineHeight − glyphTopAbove` (the y-offset from the line
//!     reference);
//!   * the coverage atlas is exactly `field8571 × field8572`, shelf-packed.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};

use crate::error::Result;
use crate::font::GLYPH_COUNT;
use crate::font::bitmap::{FontMetrics, GlyphAtlasSprite};
use crate::{cache_bail, font::cp1252_byte_to_char};

/// Shelf-packer minimum wrap width (px), matching `FontRaster.ATLAS_WRAP_MIN`.
const ATLAS_WRAP_MIN: i32 = 256;
/// Cap/digit sample used to size `ascent`-mode fonts, matching `FontRaster`.
const CAP_SAMPLE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// How to size a face when rasterizing.
#[derive(Clone, Copy, Debug)]
pub enum SizeMode {
    /// Render at this em pixel size (archive-58 `fmt=2` `sizePx`).
    Px(f32),
    /// Pick the em size whose cap/digit visual ascent best matches this px
    /// (archive-58 `fmt=1`, sized from the donor ascent).
    Ascent(f32),
}

/// Per-glyph raster cell + atlas placement (mirrors `FontRaster.Glyph`).
#[derive(Clone, Debug, Default)]
struct GlyphCell {
    w: i32,
    h: i32,
    advance: i32,
    top_above: i32,
    atlas_x: i32,
    atlas_y: i32,
    /// Coverage, `w*h` row-major (empty if no ink).
    alpha: Vec<u8>,
}

/// The full rasterized font, ready to encode as archive-13 metrics + archive-8
/// glyph atlas, plus the metadata `font preview`/diagnostics report.
#[derive(Clone, Debug)]
pub struct RasterFont {
    pub atlas_w: i32,
    pub atlas_h: i32,
    pub ascent: i32,
    pub descent: i32,
    pub line_height: i32,
    pub ink_glyphs: u32,
    pub size_px: f32,
    glyphs: Vec<GlyphCell>,
    atlas_alpha: Vec<u8>,
}

impl RasterFont {
    /// Encode the FontMetrics (archive 13) record.
    pub fn to_metrics(&self) -> Result<FontMetrics> {
        let mut advance = vec![0u8; GLYPH_COUNT];
        let mut glyph_height = vec![0u8; GLYPH_COUNT];
        let mut y_offset = vec![0u8; GLYPH_COUNT];
        let mut atlas_x = vec![0u16; GLYPH_COUNT];
        let mut atlas_y = vec![0u16; GLYPH_COUNT];
        for c in 0..GLYPH_COUNT {
            let g = &self.glyphs[c];
            advance[c] = clamp_u8(g.advance);
            glyph_height[c] = clamp_u8(g.h);
            // field8565 = lineHeight − glyphTopAbove
            y_offset[c] = clamp_u8(self.line_height - g.top_above);
            atlas_x[c] = clamp_u16(g.atlas_x);
            atlas_y[c] = clamp_u16(g.atlas_y);
        }
        Ok(FontMetrics {
            version: 0,
            has_kerning: false,
            advance,
            glyph_height,
            y_offset,
            atlas_w: clamp_u16(self.atlas_w),
            atlas_h: clamp_u16(self.atlas_h),
            atlas_x,
            atlas_y,
            line_height: clamp_u8(self.line_height),
            cs2_ascent: clamp_u8(self.ascent),
            cs2_descent: clamp_u8(self.descent),
            ascent: clamp_u8(self.ascent),
            descent: clamp_u8(self.descent),
            divisor: 1,
        })
    }

    /// Encode the glyph-coverage atlas (archive 8) sprite.
    pub fn to_sprite(&self) -> Result<GlyphAtlasSprite> {
        GlyphAtlasSprite::from_alpha(
            clamp_u16(self.atlas_w),
            clamp_u16(self.atlas_h),
            self.atlas_alpha.clone(),
        )
    }

    /// Borrow the coverage atlas buffer (`atlas_w * atlas_h` row-major).
    pub fn atlas_alpha(&self) -> &[u8] {
        &self.atlas_alpha
    }
}

/// Rasterize a TTF/OTF face into the 910 bitmap-font model.
///
/// `face_bytes` is the raw TTF/OTF (archive 59) payload; `mode` chooses the em
/// size. Ports `FontRaster.rasterize` glyph-for-glyph.
pub fn rasterize(face_bytes: &[u8], mode: SizeMode) -> Result<RasterFont> {
    let font = FontVec::try_from_vec(face_bytes.to_vec())
        .map_err(|e| crate::error::CacheError::message(format!("invalid font face: {e}")))?;

    let size_px = match mode {
        SizeMode::Px(px) => px,
        SizeMode::Ascent(target) => pick_size_for_ascent(&font, target),
    };
    // AWT's deriveFont(size) makes the em-square `size` pixels tall; ab_glyph's
    // PxScale scales by the typographic height (ascent − descent) instead, so we
    // convert the requested em-pixel size into the equivalent PxScale. This
    // reproduces the oracle FontMetrics (ascent/descent/line + atlas dims) for
    // the px-mode fonts exactly.
    let px_scale = em_scale(&font, size_px);
    let scaled = font.as_scaled(px_scale);

    let mut glyphs: Vec<GlyphCell> = (0..GLYPH_COUNT).map(|_| GlyphCell::default()).collect();
    let (mut max_top_above, mut max_below, mut max_glyph_w) = (0i32, 0i32, 0i32);
    let mut ink_glyphs = 0u32;

    // First pass: measure + render each glyph cell.
    for c in 0..GLYPH_COUNT {
        let Some(ch) = cp1252_byte_to_char(c as u8) else {
            continue; // undefined byte in windows-1252 → empty glyph
        };
        if (ch as u32) < 32 {
            continue; // control char → empty glyph
        }
        let gid = scaled.glyph_id(ch);
        if gid.0 == 0 {
            continue; // .notdef → unsupported char, empty glyph
        }
        let g = &mut glyphs[c];
        let adv = scaled.h_advance(gid).max(0.0).round() as i32;
        g.advance = adv;

        // Outline at the baseline origin (0,0); px_bounds gives the ink rect in
        // pixels with y negative above the baseline — the ab_glyph analogue of
        // AWT getVisualBounds().
        let glyph = gid.with_scale_and_position(px_scale, point(0.0, 0.0));
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue; // whitespace (e.g. space): advance only, no ink
        };
        let bounds = outlined.px_bounds();
        let ink_left = bounds.min.x.floor() as i32;
        let ink_top = bounds.min.y.floor() as i32;
        let ink_right = bounds.max.x.ceil() as i32;
        let ink_bottom = bounds.max.y.ceil() as i32;
        let ink_w = ink_right - ink_left;
        let cell_h = ink_bottom - ink_top;
        if ink_w <= 0 || cell_h <= 0 {
            continue;
        }
        let left_pad = ink_left.max(0);
        let cell_w = adv.max(left_pad + ink_w);
        g.w = cell_w;
        g.h = cell_h;
        g.top_above = -ink_top;
        // advance/cell-width must agree (FontMetrics sets field8573[c][2] = field8574[c]).
        g.advance = cell_w;

        // Render coverage into the cell. ab_glyph's draw() reports coverage at
        // pixel (x,y) relative to px_bounds.min; we offset by the left pad so
        // the ink sits at the same place FontRaster placed it (drawX = leftPad
        // − minX), and by the ink top so row 0 is the cell top.
        let mut alpha = vec![0u8; (cell_w * cell_h) as usize];
        let x_shift = left_pad - ink_left; // == leftPad − floor(minX)
        let mut any = false;
        outlined.draw(|gx, gy, coverage| {
            let px = gx as i32 + ink_left + x_shift; // = gx + leftPad
            let py = gy as i32; // px_bounds.min.y maps to row 0
            if px < 0 || px >= cell_w || py < 0 || py >= cell_h {
                return;
            }
            let a = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            if a != 0 {
                alpha[(py * cell_w + px) as usize] = a;
                any = true;
            }
        });
        if !any {
            g.w = 0;
            g.h = 0;
            g.top_above = 0;
            g.advance = adv.max(0); // keep pen advance for spacing
            continue;
        }
        g.alpha = alpha;
        ink_glyphs += 1;
        max_top_above = max_top_above.max(g.top_above);
        max_below = max_below.max(cell_h - g.top_above);
        max_glyph_w = max_glyph_w.max(cell_w);
    }

    let ascent = max_top_above.max(1);
    let descent = max_below.max(0);
    let line_height = ascent + descent;

    // Second pass: shelf-pack ink cells into the atlas (FontRaster order).
    let wrap = ATLAS_WRAP_MIN.max(max_glyph_w);
    let (mut cur_x, mut cur_y, mut shelf_h, mut max_right) = (0i32, 0i32, 0i32, 0i32);
    for c in 0..GLYPH_COUNT {
        if glyphs[c].alpha.is_empty() {
            continue;
        }
        let gw = glyphs[c].w;
        let gh = glyphs[c].h;
        if cur_x + gw > wrap {
            cur_y += shelf_h;
            cur_x = 0;
            shelf_h = 0;
        }
        glyphs[c].atlas_x = cur_x;
        glyphs[c].atlas_y = cur_y;
        cur_x += gw;
        shelf_h = shelf_h.max(gh);
        max_right = max_right.max(cur_x);
    }
    let atlas_w = max_right.max(1);
    let atlas_h = (cur_y + shelf_h).max(1);

    // Blit cells into the atlas coverage buffer.
    let mut atlas_alpha = vec![0u8; (atlas_w * atlas_h) as usize];
    for c in 0..GLYPH_COUNT {
        let g = &glyphs[c];
        if g.alpha.is_empty() {
            continue;
        }
        for y in 0..g.h {
            let dst = ((g.atlas_y + y) * atlas_w + g.atlas_x) as usize;
            let src = (y * g.w) as usize;
            atlas_alpha[dst..dst + g.w as usize]
                .copy_from_slice(&g.alpha[src..src + g.w as usize]);
        }
    }

    Ok(RasterFont {
        atlas_w,
        atlas_h,
        ascent,
        descent,
        line_height,
        ink_glyphs,
        size_px,
        glyphs,
        atlas_alpha,
    })
}

/// Binary-search the em px size whose cap/digit visual ascent best matches
/// `target_ascent` px. Ports `FontRaster.pickSizeForAscent` (24 iterations,
/// 6..48 px bracket, tracking the best error).
fn pick_size_for_ascent(font: &FontVec, target_ascent: f32) -> f32 {
    let (mut lo, mut hi) = (6.0f32, 48.0f32);
    let (mut best, mut best_err) = (12.0f32, f32::MAX);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        let ascent = cap_visual_ascent(font, mid);
        let err = (ascent - target_ascent).abs();
        if err < best_err {
            best_err = err;
            best = mid;
        }
        if ascent < target_ascent {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    best
}

/// Visual ascent (px above baseline) of the cap/digit sample at em-pixel size
/// `size_px` — the ab_glyph analogue of `-gv.getVisualBounds().getMinY()`.
fn cap_visual_ascent(font: &FontVec, size_px: f32) -> f32 {
    let scale = em_scale(font, size_px);
    let scaled = font.as_scaled(scale);
    let mut min_y = 0.0f32;
    let mut any = false;
    let mut caret = 0.0f32;
    for ch in CAP_SAMPLE.chars() {
        let gid = scaled.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, point(caret, 0.0));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let top = outlined.px_bounds().min.y; // negative above baseline
            if top < min_y {
                min_y = top;
            }
            any = true;
        }
        caret += scaled.h_advance(gid);
    }
    if any { -min_y } else { 0.0 }
}

/// Convert an em-pixel size (AWT `deriveFont` semantics: the em-square is
/// `size_px` pixels tall) into the `ab_glyph` `PxScale` that produces it.
///
/// `ab_glyph` defines `PxScale` as the *typographic* pixel-height and scales
/// glyphs by `PxScale / height_unscaled` where `height_unscaled = ascent −
/// descent`. To make the em-square (`units_per_em` design units) span `size_px`
/// pixels we therefore set `PxScale = size_px × height_unscaled / units_per_em`.
/// Falls back to `size_px` if the face omits `units_per_em` (degenerate).
fn em_scale(font: &FontVec, size_px: f32) -> PxScale {
    let upm = font.units_per_em().unwrap_or(0.0);
    if upm <= 0.0 {
        return PxScale::from(size_px);
    }
    PxScale::from(size_px * font.height_unscaled() / upm)
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn clamp_u16(v: i32) -> u16 {
    v.clamp(0, i32::from(u16::MAX)) as u16
}

/// Render a sample string into an RGB image (white text on a dark background),
/// using the exact `GlFont` glyph model the verifier uses: atlas sub-rect per
/// `field8573`, vertical offset per `field8565`, pen advance per `field8574`.
/// Returns `(width, height, rgb)` with `rgb` row-major `width*height*3`.
///
/// This is the `font preview` sample round-trip; if the encode is correct the
/// text is legible — proving the client will render it too.
pub fn render_sample(metrics: &FontMetrics, atlas: &[u8], text: &str) -> Result<(u32, u32, Vec<u8>)> {
    let atlas_w = i32::from(metrics.atlas_w);
    let atlas_h = i32::from(metrics.atlas_h);
    if atlas.len() != (atlas_w * atlas_h) as usize {
        cache_bail!("atlas buffer {} != {atlas_w}x{atlas_h}", atlas.len());
    }
    let pad = 6i32;
    let ascent = i32::from(metrics.ascent);
    let descent = i32::from(metrics.descent);
    let line = i32::from(metrics.line_height);
    let bytes = crate::font::cp1252_encode(text);

    let mut w = pad * 2;
    for &c in &bytes {
        w += i32::from(metrics.advance[c as usize]);
    }
    let h = ascent + descent + pad * 2;
    let img_w = w.max(32);
    let img_h = h.max(16);

    // Dark background 0x2b2b2b.
    let (bg_r, bg_g, bg_b) = (0x2bu32, 0x2bu32, 0x2bu32);
    let mut rgb = vec![0u8; (img_w * img_h * 3) as usize];
    for px in rgb.chunks_exact_mut(3) {
        px[0] = bg_r as u8;
        px[1] = bg_g as u8;
        px[2] = bg_b as u8;
    }

    let mut pen_x = pad;
    let baseline = pad + ascent;
    let line_origin = baseline - line;
    for &c in &bytes {
        let ci = c as usize;
        let gw = i32::from(metrics.advance[ci]);
        let gh = i32::from(metrics.glyph_height[ci]);
        let sx = i32::from(metrics.atlas_x[ci]);
        let sy = i32::from(metrics.atlas_y[ci]);
        let top = line_origin + i32::from(metrics.y_offset[ci]);
        for y in 0..gh {
            let dy = top + y;
            if dy < 0 || dy >= img_h {
                continue;
            }
            for x in 0..gw {
                if sx + x >= atlas_w || sy + y >= atlas_h {
                    continue; // advance may exceed the ink sub-rect harmlessly
                }
                let a = i32::from(atlas[((sy + y) * atlas_w + (sx + x)) as usize]);
                if a == 0 {
                    continue;
                }
                let dx = pen_x + x;
                if dx < 0 || dx >= img_w {
                    continue;
                }
                let au = a as u32;
                let r = (255 * au + bg_r * (255 - au)) / 255;
                let g = (255 * au + bg_g * (255 - au)) / 255;
                let b = (255 * au + bg_b * (255 - au)) / 255;
                let idx = ((dy * img_w + dx) * 3) as usize;
                rgb[idx] = r as u8;
                rgb[idx + 1] = g as u8;
                rgb[idx + 2] = b as u8;
            }
        }
        pen_x += gw;
    }
    Ok((img_w as u32, img_h as u32, rgb))
}

/// Render the coverage atlas itself as a grayscale-on-RGB image (white coverage
/// on black), matching `FontVerify`'s atlas dump.
pub fn render_atlas(metrics: &FontMetrics, atlas: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let aw = i32::from(metrics.atlas_w);
    let ah = i32::from(metrics.atlas_h);
    if atlas.len() != (aw * ah) as usize {
        cache_bail!("atlas buffer {} != {aw}x{ah}", atlas.len());
    }
    let mut rgb = vec![0u8; atlas.len() * 3];
    for (i, &a) in atlas.iter().enumerate() {
        rgb[i * 3] = a;
        rgb[i * 3 + 1] = a;
        rgb[i * 3 + 2] = a;
    }
    Ok((aw as u32, ah as u32, rgb))
}
