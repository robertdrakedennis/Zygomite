//! 948 → 910 interface-component **wire-version transcoder**.
//!
//! # The bug it fixes
//! The 910 client `Component.decode` is a linear, `version`-led decoder. Its
//! per-`type` body is only implemented for the primitive component types
//! **{0 layer, 3 rectangle, 4 text, 5 graphic, 6 model, 9 line}**. A newer donor
//! interface (948, wire version 11) also uses the *composite widget* types
//! **{10 button, 12 check, 13 input, 16 list, …}** whose bodies the 910 decoder
//! has **no case for** — it reads the header, then skips straight to the common
//! tail. The unconsumed widget body shifts the stream, and the trailing op-name
//! section reads a bogus index (255) into a length-1 array →
//! `ArrayIndexOutOfBoundsException` at `Component.decode:973`. Interface **1224**
//! (ritual selection) trips this on its 2 buttons + 1 check; **691** did not (it
//! uses only the primitive types).
//!
//! # The transcode
//! The header and the common tail decode **identically** between the crate's
//! build-947 (948) decoder and the 910 mirror — proven empirically: all 225
//! components of 691 and the 47 primitive-type components of 1224 round-trip
//! through [`super::decode910`] unchanged. So the *only* bytes the 910 decoder
//! cannot consume are the widget **body** of an unsupported `type`.
//!
//! For each component we therefore:
//! 1. **Header** — copy verbatim (version-independent layout), rewriting only the
//!    `type` byte for downcoded widgets, and forcing the wire `version` to **9**
//!    (the highest the 910 decoder fully handles; the primitive bodies + tail are
//!    byte-identical at 9 and 11, so no body bytes change for kept types).
//! 2. **Body** —
//!    * primitive type (kept): copy the original body bytes verbatim.
//!    * composite widget (unsupported): **drop** the widget body and synthesize a
//!      minimal body for a 910-decodable target type that preserves the most
//!      meaning — a [`text`](TargetType::Text) body (font + label + colour) when
//!      the widget had a non-empty text label, else an empty
//!      [`layer`](TargetType::Layer) (an invisible but fully-clickable hotspot).
//!      The op labels, op cursors, hooks (onload/onop/onbuttonclick → server still
//!      drives the action), params and transmit lists all live in the **common
//!      tail**, which is preserved byte-for-byte, so interactivity is intact.
//! 3. **Common tail** — copy verbatim.
//!
//! # v10/v11 fields dropped
//! There are **no `version >= 10` / `>= 11` field reads** in either decoder — the
//! wire-version bump did not append tail fields the 910 decoder skips. The entire
//! incompatibility is the *unsupported composite-widget bodies*. Downcoding a
//! widget therefore drops exactly that widget's body bytes (the checkbox/button
//! sprite + state flags + the `version >= 9` widget margins), documented per
//! target in [`Downcode`].
//!
//! Validation is by re-decoding every output component through the faithful 910
//! mirror ([`super::decode910::decode_component_910`]): it must succeed and end
//! exactly at end-of-buffer.

use std::collections::BTreeMap;

use crate::error::{Context, Result};
use crate::packet::{ByteWriter, Packet};

/// Component types the 910 `Component.decode` implements a body for. Anything
/// else has no 910 body case and must be downcoded.
pub const TYPES_910_HAS_BODY: [u8; 6] = [0, 3, 4, 5, 6, 9];

/// The wire version the 910 decoder fully handles; transcoded components are
/// written at this version.
pub const TARGET_VERSION: u8 = 9;

/// The 910-decodable type an unsupported widget is downcoded to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetType {
    /// `type 0` — empty interactive layer (invisible hotspot). Used for widgets
    /// with no text label (e.g. an image button drawn by a sibling component).
    Layer,
    /// `type 4` — static text. Used for widgets that carried a visible label, so
    /// the label still renders (e.g. the "Show Locked" check).
    Text,
}

impl TargetType {
    const fn type_id(self) -> u8 {
        match self {
            Self::Layer => 0,
            Self::Text => 4,
        }
    }
}

/// What happened to one component during transcode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Downcode {
    /// A primitive type the 910 decoder already handles; body copied verbatim.
    Kept { type_id: u8 },
    /// An unsupported composite widget rewritten to `target`, dropping its body.
    Rewritten {
        /// Original (unsupported) component type.
        from_type: u8,
        /// 910-decodable target type emitted instead.
        target: TargetType,
        /// Number of original widget-body bytes dropped.
        dropped_body_bytes: usize,
    },
}

/// The result of transcoding one component.
#[derive(Clone, Debug)]
pub struct TranscodedComponent {
    /// The re-encoded 910 wire bytes.
    pub bytes: Vec<u8>,
    /// What was done (kept / rewritten).
    pub downcode: Downcode,
}

/// The text-part fields the 910 `type 4` body needs, recovered from a composite
/// widget's text part so a downcoded label still renders.
///
/// `pub(crate)` so the port-layer interface front-end ([`crate::port::ir::interface`])
/// can reuse the single donor field-walk in [`segment`] as its decode source of
/// truth instead of duplicating it.
#[derive(Clone, Debug, Default)]
pub(crate) struct TextPart {
    pub(crate) font: i32,
    pub(crate) fontmono: bool,
    pub(crate) text: String,
    pub(crate) line_height: u8,
    pub(crate) align_h: u8,
    pub(crate) align_v: u8,
    pub(crate) shadow: bool,
    pub(crate) colour: i32,
    pub(crate) trans: u8,
    pub(crate) maxlines: u8,
}

/// The parsed segmentation of a donor component: where the body ends (start of
/// the common tail), plus the bits needed to re-emit a body. `pub(crate)` for the
/// port-layer interface front-end (shared donor field-walk).
pub(crate) struct Segments {
    /// Component type (low 7 bits).
    pub(crate) type_id: u8,
    /// Byte offset of the first type-body byte (== end of header).
    pub(crate) body_start: usize,
    /// Byte offset just past the type body (== start of common tail).
    pub(crate) body_end: usize,
    /// Recovered text part for composite widgets that have one.
    pub(crate) text_part: Option<TextPart>,
}

/// Transcode every component of an interface group's file map to the 910 wire
/// format. Returns `component id → transcoded bytes + disposition`, in ascending
/// component id.
pub fn transcode_group(
    files: &BTreeMap<u32, Vec<u8>>,
    build: u32,
) -> Result<BTreeMap<u32, TranscodedComponent>> {
    let mut out = BTreeMap::new();
    for (&id, bytes) in files {
        let t = transcode_component(bytes, build)
            .with_context(|| format!("transcode component {id}"))?;
        out.insert(id, t);
    }
    Ok(out)
}

/// Transcode one component's donor bytes to 910 wire bytes.
pub fn transcode_component(bytes: &[u8], build: u32) -> Result<TranscodedComponent> {
    let seg = segment(bytes, build)?;

    let header = &bytes[..seg.body_start];
    let body = &bytes[seg.body_start..seg.body_end];
    let tail = &bytes[seg.body_end..];

    let mut w = ByteWriter::with_capacity(bytes.len());

    if TYPES_910_HAS_BODY.contains(&seg.type_id) {
        // Keep: rewrite header with version 9 + same type, copy body + tail.
        write_header(&mut w, header, TARGET_VERSION, seg.type_id);
        w.pdata(body);
        w.pdata(tail);
        Ok(TranscodedComponent {
            bytes: w.data,
            downcode: Downcode::Kept {
                type_id: seg.type_id,
            },
        })
    } else {
        // Downcode: choose target by whether a visible label survives.
        let target = match &seg.text_part {
            Some(tp) if !tp.text.is_empty() => TargetType::Text,
            _ => TargetType::Layer,
        };
        write_header(&mut w, header, TARGET_VERSION, target.type_id());
        match target {
            TargetType::Layer => write_layer_body_v9(&mut w),
            TargetType::Text => {
                let tp = seg.text_part.as_ref().expect("text target needs a text part");
                write_text_body_v9(&mut w, tp)?;
            }
        }
        w.pdata(tail);
        Ok(TranscodedComponent {
            bytes: w.data,
            downcode: Downcode::Rewritten {
                from_type: seg.type_id,
                target,
                dropped_body_bytes: body.len(),
            },
        })
    }
}

// ── group-level pack / container (byte-faithful, mirrors config_transcode) ───

/// Pack per-file payloads (ascending id, dense) into a single-stripe (marker = 1)
/// JS5 group body: every file's bytes concatenated, then one int32 size-delta per
/// file, then the stripe-count marker byte. The inverse of the crate's indexless
/// [`crate::interface::component::decode_interface_group_raw`] unpacker for the
/// 1-stripe case the donor `.dat`s use.
fn pack_group_files(files: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in files {
        out.extend_from_slice(f);
    }
    let mut prev = 0_i32;
    for f in files {
        let len = f.len() as i32;
        out.extend_from_slice(&(len - prev).to_be_bytes());
        prev = len;
    }
    out.push(1); // single stripe
    out
}

/// Wrap a group body in a gzip JS5 container (compression 2) plus a 2-byte version
/// trailer, matching the donor `.dat` framing. The gzip byte stream is not
/// reproducible across zlib implementations, so the regression contract is on the
/// re-decoded component bytes, never the container bytes (same caveat the font /
/// config-transcode oracles note).
fn build_raw_group(body: &[u8], version: u16) -> Result<Vec<u8>> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write as _;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(body).context("gzip-compress group body")?;
    let gz = encoder.finish().context("finish gzip stream")?;

    let gz_len = gz.len() as u32;
    let body_len = body.len() as u32;
    let mut out = Vec::with_capacity(9 + gz.len() + 2);
    out.push(2); // gzip compression
    out.extend_from_slice(&gz_len.to_be_bytes()); // compressed length (excl. ulen)
    out.extend_from_slice(&body_len.to_be_bytes()); // uncompressed length
    out.extend_from_slice(&gz);
    out.extend_from_slice(&version.to_be_bytes()); // version trailer
    Ok(out)
}

/// `pub(crate)` re-export of [`pack_group_files`] for the port-layer interface
/// back-end (shared 1-stripe packer — byte-identical group bodies).
pub(crate) fn pack_group_files_pub(files: &[Vec<u8>]) -> Vec<u8> {
    pack_group_files(files)
}

/// `pub(crate)` re-export of [`build_raw_group`] for the port-layer interface
/// back-end (shared gzip JS5 container — byte-identical `.dat`s).
pub(crate) fn build_raw_group_pub(body: &[u8], version: u16) -> Result<Vec<u8>> {
    build_raw_group(body, version)
}

/// The outcome of transcoding a whole interface group.
pub struct TranscodedGroup {
    /// Per-component results in ascending component id.
    pub components: BTreeMap<u32, TranscodedComponent>,
    /// The re-packed raw group `.dat` bytes (gzip JS5 container + version
    /// trailer), ready to splice into an overlay.
    pub dat: Vec<u8>,
    /// The DECOMPRESSED group body (file chunks + footer) — the byte-stable
    /// artifact the oracle asserts on.
    pub body: Vec<u8>,
    /// Component roster (the dense `0..n` id range).
    pub roster: Vec<u32>,
}

/// Transcode a whole interface group (from its component file map) and re-pack it
/// into a raw group `.dat`. The component ids must be the dense `0..n` range the
/// interface archive uses, which [`pack_group_files`] relies on.
pub fn transcode_and_pack(
    files: &BTreeMap<u32, Vec<u8>>,
    build: u32,
    version: u16,
) -> Result<TranscodedGroup> {
    let components = transcode_group(files, build)?;
    let roster: Vec<u32> = components.keys().copied().collect();
    // Dense, contiguous from 0 — assert so the indexless re-decode recovers them.
    for (expected, &id) in roster.iter().enumerate() {
        let expected = expected as u32;
        if id != expected {
            crate::cache_bail!(
                "interface group roster is not dense from 0 (got {id} at slot {expected}); \
                 the 1-stripe packer needs contiguous component ids"
            );
        }
    }
    let ordered: Vec<Vec<u8>> = roster.iter().map(|id| components[id].bytes.clone()).collect();
    let body = pack_group_files(&ordered);
    let dat = build_raw_group(&body, version)?;
    Ok(TranscodedGroup {
        components,
        dat,
        body,
        roster,
    })
}

/// Re-emit the (version-independent) header with a forced `version` byte and a
/// (possibly rewritten) `type` byte. The header layout is identical across
/// versions; only bytes 0 (version) and the type byte change. The type byte sits
/// at offset 1, but carries the `0x80` name-present bit which we must preserve.
pub(crate) fn write_header(w: &mut ByteWriter, header: &[u8], version: u8, new_type: u8) {
    w.p1(version);
    // Preserve the name-present bit (0x80) from the original type byte.
    let name_bit = header[1] & 0x80;
    w.p1(new_type | name_bit);
    // The rest of the header (name string if present, clientcode, pos, sizes,
    // modes, aspect, layer, flags) is byte-identical → copy verbatim.
    w.pdata(&header[2..]);
}

/// Write an empty `type 0` layer body at version 9: `scrollwidth=0`,
/// `scrollheight=0`, then the `version >= 9` 4-byte margin block (all zero). This
/// is what the 910 `Component.decode` reads for a layer at version 9.
pub(crate) fn write_layer_body_v9(w: &mut ByteWriter) {
    w.p2(0); // scrollwidth
    w.p2(0); // scrollheight
    w.p1(0); // margin[0]  (version >= 9)
    w.p1(0); // margin[1]
    w.p1(0); // margin[2]
    w.p1(0); // margin[3]
}

/// Write a `type 4` text body at version 9 from a recovered [`TextPart`], matching
/// the 910 `Component.decode` text reads (lines 896-911):
/// `textfont(gSmart2or4s) · fontmono(g1, version>=2) · text(jstr) · field2229(g1)
/// · field2223(g1) · field2264(g1) · textshadow(g1) · colour(g4s) · trans(g1) ·
/// maxlines(g1, version>=0)`.
pub(crate) fn write_text_body_v9(w: &mut ByteWriter, tp: &TextPart) -> Result<()> {
    psmart2or4null(w, tp.font);
    w.p1(u8::from(tp.fontmono)); // version >= 2
    w.pjstr(&tp.text)?;
    w.p1(tp.line_height);
    w.p1(tp.align_h);
    w.p1(tp.align_v);
    w.p1(u8::from(tp.shadow));
    w.p4s(tp.colour);
    w.p1(tp.trans);
    w.p1(tp.maxlines); // version >= 0
    Ok(())
}

/// Inverse of [`Packet::gsmart2or4null`]: `-1` → `0x7FFF` (2-byte); `0..=0x7FFE`
/// → 2-byte big-endian; larger → 4-byte with the top bit set.
fn psmart2or4null(w: &mut ByteWriter, value: i32) {
    if value == -1 {
        w.p2(0x7FFF);
    } else if value <= 0x7FFE {
        w.p2(value as u16);
    } else {
        w.p4s(value | (1 << 31));
    }
}

/// Walk the donor component (build-947 / 948 layout) far enough to find the body
/// boundaries and recover a composite widget's text part. Mirrors the field order
/// of [`super::parse_component`] but records offsets instead of formatting lines.
/// `pub(crate)` so the port-layer interface front-end reuses this single donor
/// field-walk (the decode source of truth).
pub(crate) fn segment(bytes: &[u8], build: u32) -> Result<Segments> {
    let mut p = Packet::new(bytes);

    let mut version = i16::from(p.g1()?);
    if version == i16::from(u8::MAX) {
        version = -1;
    }

    let mut type_id = p.g1()?;
    if (type_id & 0x80) != 0 {
        type_id &= 0x7F;
        let _name = p.gjstr()?;
    }

    let _contenttype = p.g2()?;
    let _x = p.g2s()?;
    let _y = p.g2s()?;
    let _w = p.g2()?;
    let _h = p.g2()?;

    let mut width_mode: i8 = 0;
    let mut height_mode: i8 = 0;
    if build >= 493 {
        width_mode = p.g1s()?;
        height_mode = p.g1s()?;
        let _xmode = p.g1s()?;
        let _ymode = p.g1s()?;
    }
    if width_mode == 4 || height_mode == 4 {
        let _aspect_w = p.g2()?;
        let _aspect_h = p.g2()?;
    }
    let _layer = p.g2null()?;
    let _flags = p.g1()?;

    let body_start = p.pos();
    let has_width_mode = width_mode != 0;
    let has_height_mode = height_mode != 0;

    let mut text_part = None;
    match type_id {
        0 => skip_layer_body(&mut p, version, build)?,
        3 => skip_rect_body(&mut p)?,
        4 => {
            text_part = Some(read_text_part(&mut p, version, build)?);
        }
        5 => skip_sprite_part(&mut p, version, build)?,
        6 => skip_model_body(&mut p, build, has_width_mode, has_height_mode)?,
        9 => skip_line_body(&mut p, build)?,
        10 => {
            // button: enabled, cantoggle, unknown1, setlinkobj1, setlinkobj2,
            // textarea[4], trans, colour, sprite-part, text-part.
            for _ in 0..5 {
                let _ = p.g1()?;
            }
            for _ in 0..4 {
                let _ = p.g1()?;
            }
            let _trans = p.g1()?;
            let _colour = p.g4s()?;
            skip_sprite_part(&mut p, version, build)?;
            text_part = Some(read_text_part(&mut p, version, build)?);
        }
        11 => {
            // panel
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g1()?;
            let _ = p.g1()?;
        }
        12 => {
            // check: enabled, checked, alignment, buttonsize, trans, colour,
            // sprite-part, text-part.
            for _ in 0..5 {
                let _ = p.g1()?;
            }
            let _colour = p.g4s()?;
            skip_sprite_part(&mut p, version, build)?;
            text_part = Some(read_text_part(&mut p, version, build)?);
        }
        13 => {
            // input
            let _ = p.g1()?;
            let _ = p.g1()?;
            let _ = p.g1()?;
            let _ = p.g2()?;
            if version >= 9 {
                for _ in 0..4 {
                    let _ = p.g1()?;
                }
            }
            if version >= 7 {
                let _ = p.g1()?;
            }
            let _ = p.g1()?;
            let _ = p.g4s()?;
            skip_sprite_part(&mut p, version, build)?;
            text_part = Some(read_text_part(&mut p, version, build)?);
            skip_scrollbar_part(&mut p, version, build)?;
        }
        15 => {
            // grid
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g1()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g1()?;
        }
        16 => {
            // list
            let _ = p.g1()?;
            let _ = p.g1()?;
            let _ = p.g1()?;
            let _ = p.g1()?;
            if version >= 9 {
                let _ = p.g1()?;
            }
            let _ = p.g1()?;
            let _ = p.g1()?;
            for _ in 0..p.g2()? {
                let _ = p.gjstr()?;
            }
            for _ in 0..p.g2()? {
                let _ = p.g4s()?;
            }
            for _ in 0..p.g2()? {
                let _ = p.g2()?;
            }
            if version >= 9 {
                for _ in 0..8 {
                    let _ = p.g1()?;
                }
            }
            let _ = p.g4s()?;
            skip_sprite_part(&mut p, version, build)?;
            skip_sprite_part(&mut p, version, build)?;
            skip_sprite_part(&mut p, version, build)?;
            text_part = Some(read_text_part(&mut p, version, build)?);
            skip_scrollbar_part(&mut p, version, build)?;
        }
        26 => {
            // crmview
            let _ = p.g1()?;
            for _ in 0..p.g2()? {
                let _ = p.g2()?;
            }
            let _ = p.gjstr()?;
            let _ = p.g1()?;
            for _ in 0..p.g2()? {
                let _ = p.gjstr()?;
            }
            for _ in 0..p.g2()? {
                let _ = p.g4s()?;
            }
        }
        other => {
            crate::cache_bail!("unsupported component type {other} (no 948 body decoder)");
        }
    }

    let body_end = p.pos();
    Ok(Segments {
        type_id,
        body_start,
        body_end,
        text_part,
    })
}

fn skip_layer_body(p: &mut Packet<'_>, version: i16, build: u32) -> Result<()> {
    let _ = p.g2()?;
    let _ = p.g2()?;
    if version == -1 && build >= 495 {
        let _ = p.g1()?;
    } else if version >= 9 {
        for _ in 0..4 {
            let _ = p.g1()?;
        }
    } else if version >= 6 {
        for _ in 0..4 {
            let _ = p.g2()?;
        }
    }
    Ok(())
}

fn skip_rect_body(p: &mut Packet<'_>) -> Result<()> {
    let _ = p.g4s()?;
    let _ = p.g1()?;
    let _ = p.g1()?;
    Ok(())
}

fn skip_line_body(p: &mut Packet<'_>, build: u32) -> Result<()> {
    let _ = p.g1()?;
    let _ = p.g4s()?;
    if build >= 493 {
        let _ = p.g1()?;
    }
    Ok(())
}

fn skip_model_body(
    p: &mut Packet<'_>,
    build: u32,
    has_width_mode: bool,
    has_height_mode: bool,
) -> Result<()> {
    let _model = if build < 681 {
        p.g2null()?
    } else {
        p.gsmart2or4null()?
    };
    if build < 619 {
        for _ in 0..6 {
            let _ = p.g2()?;
        }
        let _anim = if build < 681 {
            p.g2null()?
        } else {
            p.gsmart2or4null()?
        };
        let _ = p.g1()?;
        if build >= 493 {
            let _ = p.g2()?;
        }
        if build >= 501 {
            let _ = p.g2()?;
            let _ = p.g1()?;
        }
    } else {
        let model_flags = p.g1()?;
        let has_transform = (model_flags & 1) != 0;
        let has_precise_zoom = (model_flags & 2) != 0;
        if has_transform {
            let _ = p.g2s()?;
            let _ = p.g2s()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
        } else if has_precise_zoom {
            let _ = p.g2s()?;
            let _ = p.g2s()?;
            let _ = p.g2s()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
            let _ = p.g2()?;
        }
        let _anim = if build < 681 {
            p.g2null()?
        } else {
            p.gsmart2or4null()?
        };
    }
    if has_width_mode {
        let _ = p.g2()?;
    }
    if has_height_mode {
        let _ = p.g2()?;
    }
    Ok(())
}

fn read_text_part(p: &mut Packet<'_>, version: i16, build: u32) -> Result<TextPart> {
    let font = if build < 800 {
        if build < 681 {
            p.g2null()?
        } else {
            p.gsmart2or4null()?
        }
    } else {
        p.gsmart2or4null()?
    };
    let fontmono = if version >= 2 { p.g1()? == 1 } else { true };
    let text = p.gjstr()?;
    let line_height = p.g1()?;
    let align_h = p.g1()?;
    let align_v = p.g1()?;
    let shadow = p.g1()? == 1;
    let colour = p.g4s()?;
    let trans = if build >= 582 { p.g1()? } else { 0 };
    let maxlines = if version >= 0 { p.g1()? } else { 0 };
    Ok(TextPart {
        font,
        fontmono,
        text,
        line_height,
        align_h,
        align_v,
        shadow,
        colour,
        trans,
        maxlines,
    })
}

fn skip_sprite_part(p: &mut Packet<'_>, version: i16, build: u32) -> Result<()> {
    let _graphic = p.g4s()?;
    let _angle = p.g2()?;
    let _flags = p.g1()?;
    let _trans = p.g1()?;
    let _outline = p.g1()?;
    let _shadow = p.g4s()?;
    let _vflip = p.g1()?;
    let _hflip = p.g1()?;
    if build >= 537 {
        let _colour = p.g4s()?;
    }
    if version >= 3 {
        let _clickmask = p.g1()?;
    }
    if version >= 6 {
        for _ in 0..4 {
            let _ = p.g1()?;
        }
    }
    Ok(())
}

fn skip_scrollbar_part(p: &mut Packet<'_>, version: i16, build: u32) -> Result<()> {
    let _ = p.g1()?;
    let _ = p.g1()?;
    let _ = p.g1()?;
    let _ = p.g1()?;
    skip_sprite_part(p, version, build)?;
    skip_sprite_part(p, version, build)?;
    skip_sprite_part(p, version, build)?;
    Ok(())
}

// ── CLI orchestration (`interface transcode`) ────────────────────────────────

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Where to read the donor interface group from.
pub enum GroupSource<'a> {
    /// A raw group `.dat` (gzip JS5 container + 2-byte version trailer).
    RawDat(&'a Path),
    /// The runtime interfaces `.js5` pack root; the group is read by id.
    Pack(&'a Path),
}

/// Options for [`run`].
pub struct InterfaceTranscodeOptions<'a> {
    /// Interface group id (used for the pack lookup, output filename, messages).
    pub group: u32,
    /// Donor build (must be 948 — the only supported source today).
    pub from: u32,
    /// Target build (must be 910 — the only supported target today).
    pub to: u32,
    /// Build number the donor components decode at (the crate's 948/947 layout).
    pub decode_build: u32,
    /// Donor group byte source.
    pub source: GroupSource<'a>,
    /// Optional output dir; writes `<group>-948.dat`. READ-ONLY caches otherwise —
    /// never point this at a protected oracle dir.
    pub out_dir: Option<&'a Path>,
    /// Emit a JSON summary instead of the human report.
    pub json: bool,
}

/// One downcoded component, summarised for the report.
#[derive(serde::Serialize)]
struct RewriteSummary {
    component: u32,
    from_type: u8,
    to_type: &'static str,
    dropped_body_bytes: usize,
    ops: Vec<String>,
}

/// `pub(crate)` re-export of [`read_group_files`] for the port-layer interface
/// driver (shared donor group reader — raw `.dat` or runtime pack).
pub(crate) fn read_group_files_pub(
    source: &GroupSource<'_>,
    group: u32,
    decode_build: u32,
) -> Result<BTreeMap<u32, Vec<u8>>> {
    read_group_files(source, group, decode_build)
}

/// Read the donor group's component file map from the chosen source.
fn read_group_files(
    source: &GroupSource<'_>,
    group: u32,
    decode_build: u32,
) -> Result<BTreeMap<u32, Vec<u8>>> {
    match source {
        GroupSource::RawDat(path) => {
            let raw = std::fs::read(path)
                .with_context(|| format!("read raw interface group {}", path.display()))?;
            let decoded =
                crate::interface::component::decode_interface_group_raw(&raw, decode_build)?;
            Ok(decoded.files().clone())
        }
        GroupSource::Pack(root) => {
            let pack_path = root.join("client.interfaces.js5");
            let pack = crate::js5pack::PackArchive::open(&pack_path)
                .with_context(|| format!("open interfaces pack {}", pack_path.display()))?;
            pack.group_files(group)?.ok_or_else(|| {
                crate::error::CacheError::message(format!(
                    "interface {group} absent in {}",
                    pack_path.display()
                ))
            })
        }
    }
}

/// Run `interface transcode` — a THIN ALIAS over the typed interface port layer
/// ([`crate::port::interface`]). Reads the donor group, ports it through the IR
/// (decode → `lower::list_to_server_driven` → encode, validating every component
/// through the faithful 910 mirror), re-packs the group, optionally writes the
/// `<group>-948.dat`, and prints a summary. The mirror validation is the
/// in-process replacement for "run the client and see if it crashes".
///
/// The byte production is delegated to the port layer (proven byte-identical to
/// the legacy `transcode_and_pack` by the interface-port oracle); the legacy
/// `transcode_group` / `transcode_and_pack` API remains for the equivalence oracle
/// and any in-process caller. The `-948.dat` output name and the report format are
/// preserved so existing tooling/docs that call `interface transcode` are unchanged.
pub fn run(opts: &InterfaceTranscodeOptions<'_>) -> Result<()> {
    if opts.from != 948 || opts.to != 910 {
        crate::cache_bail!(
            "interface transcode currently supports only --from 948 --to 910 (got {} -> {})",
            opts.from,
            opts.to
        );
    }

    let target = crate::port::book::BuildDescriptor::load(
        &crate::cs2::lint::default_data_dir(),
        opts.to,
    )?;
    let files = read_group_files(&opts.source, opts.group, opts.decode_build)?;
    let component_count = files.len();
    let ported = crate::port::interface::port_interface_group(
        opts.group,
        &files,
        opts.decode_build,
        &target,
        u16::from(TARGET_VERSION),
    )?;
    // The port layer already validated every component through the 910 mirror.
    let validated = ported.group.components.len();

    let rewrites: Vec<RewriteSummary> = ported
        .downcodes
        .iter()
        .filter_map(|(id, d)| {
            let (from_type, to_type, dropped) = match d {
                crate::port::lower::interface::Downcoded::Kept => return None,
                crate::port::lower::interface::Downcoded::ToLayer { from, dropped } => {
                    (from.type_id(), "layer", *dropped)
                }
                crate::port::lower::interface::Downcoded::ToText { from, dropped } => {
                    (from.type_id(), "text", *dropped)
                }
            };
            let op_labels = ported
                .group
                .components
                .get(id)
                .and_then(|b| super::decode910::decode_component_910(b).ok())
                .map(|d| d.ops)
                .unwrap_or_default();
            Some(RewriteSummary {
                component: *id,
                from_type,
                to_type,
                dropped_body_bytes: dropped,
                ops: op_labels,
            })
        })
        .collect();

    if let Some(dir) = opts.out_dir {
        let iface_dir = dir.join("interfaces");
        std::fs::create_dir_all(&iface_dir)
            .with_context(|| format!("create {}", iface_dir.display()))?;
        let out_path = iface_dir.join(format!("{}-948.dat", opts.group));
        std::fs::write(&out_path, &ported.group.dat)
            .with_context(|| format!("write {}", out_path.display()))?;
    }

    if opts.json {
        let summary = serde_json::json!({
            "group": opts.group,
            "from": opts.from,
            "to": opts.to,
            "target_version": TARGET_VERSION,
            "components": component_count,
            "kept": component_count - rewrites.len(),
            "rewritten": rewrites,
            "validated_through_910_mirror": validated,
            "dat_bytes": ported.group.dat.len(),
            "wrote": opts.out_dir.is_some(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).context("encode transcode summary")?
        );
    } else {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "interface transcode — group {} ({} -> {}, version {})",
            opts.group, opts.from, opts.to, TARGET_VERSION
        );
        let _ = writeln!(
            s,
            "  {component_count} components: {} kept, {} downcoded; all {validated} validated through the 910 mirror",
            component_count - rewrites.len(),
            rewrites.len()
        );
        for r in &rewrites {
            let _ = writeln!(
                s,
                "    com{}: type {} -> {} (dropped {} body bytes; ops {:?})",
                r.component, r.from_type, r.to_type, r.dropped_body_bytes, r.ops
            );
        }
        if opts.out_dir.is_some() {
            let _ = writeln!(s, "  wrote interfaces/{}-948.dat", opts.group);
        }
        print!("{s}");
    }
    Ok(())
}

/// Default runtime pack root (matches `explain-interface`).
pub const DEFAULT_PACK_ROOT_STR: &str = "../../server/data/pack-910-base-948-overlay";

/// Default runtime pack root (matches `explain-interface`).
#[must_use]
pub fn default_pack_root() -> PathBuf {
    PathBuf::from(DEFAULT_PACK_ROOT_STR)
}
