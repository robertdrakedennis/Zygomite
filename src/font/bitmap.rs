//! 910 bitmap-font byte formats: `FontMetrics` (archive 13) + glyph-atlas
//! `SpriteData` (archive 8) encode/decode.
//!
//! This is the deterministic core of the toolkit, ported directly from the
//! committed relic-system-948 oracle (`fontraster/FontRaster.java`
//! `encodeMetrics`/`encodeSprite` and `fontraster/FontVerify.java`
//! `decodeMetrics`/`decodeSpriteAlpha`) and cross-checked against the live
//! client decoders (`com/jagex/graphics/FontMetrics.java` ctor and
//! `com/jagex/graphics/SpriteDataProvider.decodeSprites`). The encode side here
//! must byte-reproduce the oracle `fonts/metrics/<id>.bin` (FontMetrics record)
//! and `fonts/sprites/<id>.bin` (single translucent paletted sprite); the
//! decode side is the inverse the verifier and other agents reuse.

use crate::error::Result;
use crate::packet::{ByteWriter, Packet};
use crate::{cache_bail, font::GLYPH_COUNT};

/// Decoded `FontMetrics` (archive 13) record — one bitmap font.
///
/// Field names mirror the obfuscated client fields (`com.jagex.graphics
/// .FontMetrics`) so callers can reason about them against the engine:
/// `advance`=`field8574`, `glyph_height`=`field8564`, `y_offset`=`field8565`,
/// `atlas_w`/`atlas_h`=`field8571`/`field8572`, `atlas_x`/`atlas_y`=
/// `field8573[c][0..1]`, `line_height`=`field8566`, `ascent`=`field8562`,
/// `descent`=`field8569`, `divisor`=`field8570`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontMetrics {
    pub version: u8,
    pub has_kerning: bool,
    /// Pen advance per glyph (== atlas cell width). `field8574`.
    pub advance: Vec<u8>,
    /// Glyph cell height per glyph. `field8564`.
    pub glyph_height: Vec<u8>,
    /// Glyph y-offset from the `line_height` reference. `field8565`.
    pub y_offset: Vec<u8>,
    /// Atlas (coverage texture) dimensions. `field8571` / `field8572`.
    pub atlas_w: u16,
    pub atlas_h: u16,
    /// Per-glyph atlas sub-rect top-left. `field8573[c][0]` / `[1]`.
    pub atlas_x: Vec<u16>,
    pub atlas_y: Vec<u16>,
    /// Line height / y reference + default leading. `field8566`.
    pub line_height: u8,
    /// CS2 fontmetrics-op ascent/descent. `field8568` / `field8567`.
    pub cs2_ascent: u8,
    pub cs2_descent: u8,
    /// Ascent / descent. `field8562` / `field8569`.
    pub ascent: u8,
    pub descent: u8,
    /// Fixed-point divisor (1 in all relic fonts). `field8570`.
    pub divisor: u8,
}

impl FontMetrics {
    /// Decode a FontMetrics archive-13 record, faithful to the client
    /// `FontMetrics(byte[])` ctor and the oracle `FontVerify.decodeMetrics`.
    ///
    /// Only the non-kerning (`has_kerning == false`) shape used by the relic
    /// fonts is fully decoded; a kerning record is rejected (the relic
    /// rasterizer never emits one and the toolkit never needs to read one).
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut p = Packet::new(bytes);
        let version = p.g1()?;
        if version != 0 {
            cache_bail!("unexpected FontMetrics version {version} (expected 0)");
        }
        let has_kerning = p.g1()? == 1;
        if has_kerning {
            cache_bail!("kerning FontMetrics records are not supported by the font toolkit");
        }
        let advance = p.gdata(GLYPH_COUNT)?;
        let glyph_height = p.gdata(GLYPH_COUNT)?;
        let y_offset = p.gdata(GLYPH_COUNT)?;
        let atlas_w = p.g2()?;
        let atlas_h = p.g2()?;
        let mut atlas_x = Vec::with_capacity(GLYPH_COUNT);
        for _ in 0..GLYPH_COUNT {
            atlas_x.push(p.g2()?);
        }
        let mut atlas_y = Vec::with_capacity(GLYPH_COUNT);
        for _ in 0..GLYPH_COUNT {
            atlas_y.push(p.g2()?);
        }
        let line_height = p.g1()?;
        let cs2_ascent = p.g1()?;
        let cs2_descent = p.g1()?;
        let ascent = p.g1()?;
        let descent = p.g1()?;
        let divisor = p.g1()?;
        if divisor != 1 {
            cache_bail!("unexpected FontMetrics divisor {divisor} (expected 1)");
        }
        Ok(Self {
            version,
            has_kerning,
            advance,
            glyph_height,
            y_offset,
            atlas_w,
            atlas_h,
            atlas_x,
            atlas_y,
            line_height,
            cs2_ascent,
            cs2_descent,
            ascent,
            descent,
            divisor,
        })
    }

    /// Encode to the FontMetrics archive-13 payload, byte-for-byte the format
    /// the oracle `FontRaster.encodeMetrics` writes and the client ctor reads.
    pub fn encode(&self) -> Result<Vec<u8>> {
        for (name, v) in [
            ("advance", &self.advance),
            ("glyph_height", &self.glyph_height),
            ("y_offset", &self.y_offset),
        ] {
            if v.len() != GLYPH_COUNT {
                cache_bail!(
                    "FontMetrics.{name} must have {GLYPH_COUNT} entries, got {}",
                    v.len()
                );
            }
        }
        if self.atlas_x.len() != GLYPH_COUNT || self.atlas_y.len() != GLYPH_COUNT {
            cache_bail!("FontMetrics atlas_x/atlas_y must have {GLYPH_COUNT} entries");
        }
        let mut o = ByteWriter::new();
        o.p1(self.version);
        o.p1(u8::from(self.has_kerning));
        o.pdata(&self.advance);
        o.pdata(&self.glyph_height);
        o.pdata(&self.y_offset);
        o.p2(self.atlas_w);
        o.p2(self.atlas_h);
        for &x in &self.atlas_x {
            o.p2(x);
        }
        for &y in &self.atlas_y {
            o.p2(y);
        }
        o.p1(self.line_height);
        o.p1(self.cs2_ascent);
        o.p1(self.cs2_descent);
        o.p1(self.ascent);
        o.p1(self.descent);
        o.p1(self.divisor);
        Ok(o.data)
    }
}

/// A single translucent paletted glyph-coverage sprite (archive 8), the atlas
/// the bitmap font path samples. Decoded into a flat `atlas_w * atlas_h`
/// coverage buffer (0..255 alpha), matching what `GlFont` uploads as an ALPHA
/// texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphAtlasSprite {
    pub width: u16,
    pub height: u16,
    /// Coverage alpha, `width * height` row-major.
    pub alpha: Vec<u8>,
}

impl GlyphAtlasSprite {
    /// Build a coverage sprite from a raw alpha buffer.
    pub fn from_alpha(width: u16, height: u16, alpha: Vec<u8>) -> Result<Self> {
        let expected = usize::from(width) * usize::from(height);
        if alpha.len() != expected {
            cache_bail!(
                "alpha buffer length {} != {width}x{height} = {expected}",
                alpha.len()
            );
        }
        Ok(Self {
            width,
            height,
            alpha,
        })
    }

    /// Encode as the single translucent paletted SpriteData (archive 8) the
    /// oracle `FontRaster.encodeSprite` writes and `SpriteDataProvider
    /// .decodeSprites` reads:
    ///   pixel block: flags=0x02 (alpha, row-major), colour[w*h] (palette idx 1
    ///       where ink else 0), alpha[w*h];
    ///   palette: one entry 0xFFFFFF;
    ///   dims: canvasW, canvasH, paletteCount-1=1, offsetX=0, offsetY=0,
    ///       subW=canvasW, subH=canvasH;
    ///   trailer: (fmt=0 paletted)<<15 | count=1.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let size = usize::from(self.width) * usize::from(self.height);
        if self.alpha.len() != size {
            cache_bail!("alpha buffer length {} != {size}", self.alpha.len());
        }
        let mut o = ByteWriter::new();
        // pixel block
        o.p1(0x02); // flags: bit1 alpha, bit0=0 row-major
        for &a in &self.alpha {
            o.p1(u8::from(a != 0)); // colour index (1 where ink)
        }
        o.pdata(&self.alpha); // alpha coverage
        // palette (one entry, index 1 = white)
        o.p3(0x00FF_FFFF);
        // dims
        o.p2(self.width); // canvas width
        o.p2(self.height); // canvas height
        o.p1(1); // paletteCount-1 => paletteCount = 2
        o.p2(0); // offsetX
        o.p2(0); // offsetY
        o.p2(self.width); // subWidth
        o.p2(self.height); // subHeight
        // trailer: (fmt=0 paletted) << 15 | count=1
        o.p2(1);
        Ok(o.data)
    }

    /// Decode the single translucent paletted sprite into a coverage buffer,
    /// faithful to the oracle `FontVerify.decodeSpriteAlpha` (which is itself a
    /// reduction of `SpriteDataProvider.decodeSprites` for the count=1,
    /// flags=0x02 case the rasterizer emits). `expect_w`/`expect_h`, when given,
    /// are validated against the canvas/sub-rect dimensions.
    pub fn decode(bytes: &[u8], expect: Option<(u16, u16)>) -> Result<Self> {
        if bytes.len() < 2 {
            cache_bail!("sprite payload too short ({} bytes)", bytes.len());
        }
        // trailer: fmt/count
        let mut tail = Packet::with_pos(bytes, bytes.len() - 2)?;
        let last2 = tail.g2()?;
        let fmt = last2 >> 15;
        let count = last2 & 0x7FFF;
        if fmt != 0 || count != 1 {
            cache_bail!("sprite fmt/count {fmt}/{count} (expected 0/1)");
        }
        // dims block: 7 + count*8 bytes from the end.
        let dim_pos = bytes
            .len()
            .checked_sub(7 + usize::from(count) * 8)
            .ok_or_else(|| crate::error::CacheError::message("sprite dims underflow"))?;
        let mut dims = Packet::with_pos(bytes, dim_pos)?;
        let canvas_w = dims.g2()?;
        let canvas_h = dims.g2()?;
        let pal_count = u16::from(dims.g1()?) + 1;
        // count=1: offsetX(2) offsetY(2) subWidth(2) subHeight(2)
        let _offset_x = dims.g2()?;
        let _offset_y = dims.g2()?;
        let sub_w = dims.g2()?;
        let sub_h = dims.g2()?;
        if let Some((ew, eh)) = expect
            && (canvas_w != ew || canvas_h != eh || sub_w != ew || sub_h != eh)
        {
            cache_bail!(
                "sprite dims {canvas_w}x{canvas_h} sub {sub_w}x{sub_h} != expected {ew}x{eh}"
            );
        }
        if canvas_w != sub_w || canvas_h != sub_h {
            cache_bail!(
                "sprite sub-rect {sub_w}x{sub_h} != canvas {canvas_w}x{canvas_h} (atlas must fill canvas)"
            );
        }
        let size = usize::from(sub_w) * usize::from(sub_h);
        // pixel block @0
        let mut px = Packet::new(bytes);
        let flags = px.g1()?;
        if (flags & 0x2) == 0 {
            cache_bail!("sprite not translucent (flags={flags})");
        }
        if (flags & 0x1) != 0 {
            cache_bail!("sprite is column-major (flags={flags}); the font atlas is row-major");
        }
        // skip colour index map, read alpha
        px.set_pos(px.pos() + size)?;
        let alpha = px.gdata(size)?;
        let _ = pal_count;
        Ok(Self {
            width: sub_w,
            height: sub_h,
            alpha,
        })
    }
}
