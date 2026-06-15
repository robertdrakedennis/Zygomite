//! 948 modern font system: `FontMetrics2` (archive 58) reference decode +
//! `Ttf` (archive 59) face extraction.
//!
//! Ported from the relic-system-948 discovery step
//! (`build-fonts.ts`): the 910 client has no modern-font reader, so archive 58
//! is the *reference table* that points each modern font id at a TTF/OTF face
//! (archive 59) plus a size. Two entry shapes exist:
//!   * `fmt == 2` (6 bytes `[2][faceId:4 BE][sizePx:1]`): render `faceId` @
//!     `sizePx` em pixels.
//!   * `fmt == 1` (version-1 bitmap metrics, 1808 bytes for the relic fonts):
//!     a pre-baked bitmap whose pixels are not 910-decodable; the trailing 6
//!     bytes are the classic `[line, f8568, f8567, ascent, descent, divisor]`,
//!     so `target_ascent = round(ascent / divisor)` recovers the layout the
//!     interface was authored against. The face is supplied externally
//!     (`fmt=1` carries no face reference — fonts 56/57 are Cinzel).
//!
//! These decoders are reused by `font decode --archive 58|59` and by
//! `font rasterize` to build its worklist from a live pack.

use crate::error::Result;
use crate::packet::Packet;
use crate::{cache_bail, font::GLYPH_COUNT};

/// A decoded archive-58 (`FontMetrics2`) reference-table entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModernFontRef {
    /// `fmt == 2`: a reference to a TTF/OTF face at an explicit pixel size.
    TtfRef { face: u32, size_px: u8 },
    /// `fmt == 1`: a pre-baked version-1 bitmap font. The face is not encoded;
    /// the recovered `target_ascent` (= `ascent / divisor`) and the raw
    /// `ascent`/`descent`/`divisor` are exposed so the rasterizer can match the
    /// donor cap ascent.
    Fmt1Bitmap {
        target_ascent: u8,
        ascent: u8,
        descent: u8,
        divisor: u8,
        line_height: u8,
    },
}

impl ModernFontRef {
    /// Decode an archive-58 payload (already JS5-decompressed).
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let fmt = *payload
            .first()
            .ok_or_else(|| crate::error::CacheError::message("empty fontmetrics2 payload"))?;
        match fmt {
            2 => {
                if payload.len() < 6 {
                    cache_bail!("fmt=2 fontmetrics2 payload too short ({} bytes)", payload.len());
                }
                let mut p = Packet::with_pos(payload, 1)?;
                // faceId is a big-endian u32 (matches build-fonts.ts shifts).
                let face = u32::from(p.g1()?) << 24
                    | u32::from(p.g1()?) << 16
                    | u32::from(p.g1()?) << 8
                    | u32::from(p.g1()?);
                let size_px = p.g1()?;
                Ok(Self::TtfRef { face, size_px })
            }
            1 => {
                // version-1 bitmap metrics. The trailing 6 bytes are read as g1
                // from the end, robust regardless of the middle table layout:
                // [line, f8568, f8567, ascent, descent, divisor].
                if payload.len() < 6 {
                    cache_bail!("fmt=1 fontmetrics2 payload too short ({} bytes)", payload.len());
                }
                let n = payload.len();
                let line_height = payload[n - 6];
                let ascent = payload[n - 3];
                let descent = payload[n - 2];
                let divisor = if payload[n - 1] == 0 { 1 } else { payload[n - 1] };
                let target_ascent =
                    u8::try_from((u32::from(ascent) / u32::from(divisor)).max(1)).unwrap_or(u8::MAX);
                Ok(Self::Fmt1Bitmap {
                    target_ascent,
                    ascent,
                    descent,
                    divisor,
                    line_height,
                })
            }
            other => cache_bail!("unexpected fontmetrics2 fmt={other} (len={})", payload.len()),
        }
    }

    /// The 256-glyph table size a fmt=1 record should occupy (sanity bound).
    /// `version` byte + `has_kerning` byte are not part of fmt=1 here; the relic
    /// fmt=1 blobs are 1808 bytes. Exposed for diagnostics.
    pub const FMT1_EXPECTED_LEN: usize = 2 + GLYPH_COUNT * 7 + 6; // == 1808
}

/// Recognised face container magic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceKind {
    /// `00 01 00 00` — TrueType outlines.
    TrueType,
    /// `OTTO` — OpenType/CFF outlines (the Cinzel faces 3/4 are this).
    OpenTypeCff,
}

/// An extracted archive-59 face: the raw TTF/OTF bytes plus its container kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedFace {
    pub kind: FaceKind,
    pub bytes: Vec<u8>,
}

impl ExtractedFace {
    /// Validate + classify a JS5-decompressed archive-59 face payload.
    pub fn from_payload(payload: Vec<u8>) -> Result<Self> {
        let kind = classify_face(&payload)?;
        Ok(Self { kind, bytes: payload })
    }
}

/// Classify a font face container by magic, rejecting anything that is not a
/// TTF/OTF (mirrors the `build-fonts.ts` `extractFace` guard).
pub fn classify_face(payload: &[u8]) -> Result<FaceKind> {
    let magic = payload.get(..4).unwrap_or(&[]);
    match magic {
        [0x00, 0x01, 0x00, 0x00] => Ok(FaceKind::TrueType),
        [0x4f, 0x54, 0x54, 0x4f] => Ok(FaceKind::OpenTypeCff), // "OTTO"
        _ => {
            let hex: Vec<String> = magic.iter().map(|b| format!("{b:02x}")).collect();
            cache_bail!("face is not TTF/OTF (magic {})", hex.join(" "))
        }
    }
}
