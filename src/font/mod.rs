//! Font toolkit: decode/preview/rasterize/diff the 910 bitmap-font formats
//! (FontMetrics archive 13 + glyph atlas archive 8) and the 948 modern font
//! system (FontMetrics2 archive 58 + Ttf archive 59).
//!
//! Ports the one-off relic-system-948 `FontRaster.java` / `FontVerify.java`
//! pipeline into a reusable subcommand. The deterministic decoders/encoders
//! (`bitmap`, `modern`) are byte-locked against the committed relic oracle.
//!
//! TTF→bitmap rasterization has two paths:
//!   * `raster_awt` (DEFAULT) shells out to the proven AWT rasterizer (the shared
//!     copy of `FontRaster.java`), so `font rasterize` byte-reproduces the
//!     committed golden font groups — a pure-Rust rasterizer cannot match AWT's
//!     glyph metrics + anti-aliasing. See `font_oracle` (the regression-lock).
//!   * `raster` is the pure-Rust `ab_glyph` rasterizer, kept behind
//!     `--experimental`; it is locked only at the structural/invariant level (its
//!     anti-aliasing differs from AWT — see that module's docs).
//!
//! Other Rust tools reuse `bitmap` / `modern` for any donor UI port that
//! touches fonts.

pub mod bitmap;
pub mod cli;
pub mod modern;
pub mod raster;
pub mod raster_awt;

/// Glyphs in a bitmap font: one per byte 0..=255 (Cp1252-indexed).
pub const GLYPH_COUNT: usize = 256;

/// windows-1252 high-range table for bytes 0x80..=0x9F, mirroring the client
/// `com.jagex.core.utils.Cp1252.field8326`. `0` marks an undefined byte.
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0x0000, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x0000, 0x017D, 0x0000, 0x0000, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x0000, 0x017E, 0x0178,
];

/// Map a Cp1252 byte to its unicode `char`, or `None` if the byte is undefined
/// in windows-1252 (0x81/0x8D/0x8F/0x90/0x9D) or the NUL byte. Mirrors the
/// oracle `FontRaster.cp1252ToChar` (which returns null for the replacement
/// char). Bytes < 0x80 and >= 0xA0 are identity; 0x80..0x9F use `CP1252_HIGH`.
pub fn cp1252_byte_to_char(b: u8) -> Option<char> {
    match b {
        0 => None,
        0x80..=0x9F => {
            let cp = CP1252_HIGH[(b - 0x80) as usize];
            if cp == 0 {
                None
            } else {
                char::from_u32(u32::from(cp))
            }
        }
        _ => Some(b as char),
    }
}

/// Encode a single `char` to its Cp1252 byte, a faithful port of the client
/// `com.jagex.core.utils.Cp1252.encode(char)` (the render-time inverse used to
/// look glyphs up by byte). Unencodable chars fall back to `'?'` (0x3F).
pub fn cp1252_encode_char(ch: char) -> u8 {
    let c = ch as u32;
    // ASCII (1..127) and Latin-1 (160..255) are identity.
    if (c > 0 && c < 128) || (160..=255).contains(&c) {
        return c as u8;
    }
    // High-range characters via the reverse of CP1252_HIGH.
    for (i, &cp) in CP1252_HIGH.iter().enumerate() {
        if cp != 0 && u32::from(cp) == c {
            return 0x80 + i as u8;
        }
    }
    b'?'
}

/// Encode a string to its Cp1252 byte sequence (per-char `cp1252_encode_char`).
pub fn cp1252_encode(s: &str) -> Vec<u8> {
    s.chars().map(cp1252_encode_char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1252_roundtrip_ascii_and_latin1() {
        for b in 0x20u8..=0x7E {
            let ch = cp1252_byte_to_char(b).unwrap();
            assert_eq!(cp1252_encode_char(ch), b, "ascii byte {b:#x}");
        }
        for b in 0xA0u8..=0xFF {
            let ch = cp1252_byte_to_char(b).unwrap();
            assert_eq!(cp1252_encode_char(ch), b, "latin1 byte {b:#x}");
        }
    }

    #[test]
    fn cp1252_high_range_roundtrip() {
        for b in 0x80u8..=0x9F {
            match cp1252_byte_to_char(b) {
                Some(ch) => assert_eq!(cp1252_encode_char(ch), b, "high byte {b:#x}"),
                None => assert!(
                    matches!(b, 0x81 | 0x8D | 0x8F | 0x90 | 0x9D),
                    "byte {b:#x} unexpectedly undefined"
                ),
            }
        }
    }

    #[test]
    fn cp1252_undefined_bytes_and_nul() {
        for b in [0u8, 0x81, 0x8D, 0x8F, 0x90, 0x9D] {
            assert!(
                cp1252_byte_to_char(b).is_none(),
                "byte {b:#x} must be undefined"
            );
        }
    }

    #[test]
    fn cp1252_known_high_mappings() {
        // Euro at 0x80, trademark at 0x99 (matches Cp1252.field8326).
        assert_eq!(cp1252_byte_to_char(0x80), Some('\u{20AC}'));
        assert_eq!(cp1252_encode_char('\u{20AC}'), 0x80);
        assert_eq!(cp1252_byte_to_char(0x99), Some('\u{2122}'));
        assert_eq!(cp1252_encode_char('\u{2122}'), 0x99);
    }
}
