//! A faithful Rust mirror of the **910** client `Component.decode`
//! (`com/jagex/game/config/iftype/Component.java`), used as the validation oracle
//! for the 948 → 910 component transcoder.
//!
//! The 910 decoder is a LINEAR, `version`-byte-led decoder whose field set is
//! gated up to a maximum handled version of **9**. A 948 component carries a
//! newer wire version (11) with appended fields; feeding those raw bytes to this
//! mirror reproduces the live crash the client hits — the stream misaligns and
//! the trailing op-name section reads a bogus index, an
//! `ArrayIndexOutOfBoundsException` at `Component.decode:973`. Here that surfaces
//! as [`Decode910Error::OpnameIndexOutOfBounds`] (or an out-of-bounds packet
//! read), never a panic.
//!
//! The transcoder ([`super::transcode`]) re-encodes each component to exactly the
//! bytes this mirror reads at `version = 9`; a re-encoded component is correct iff
//! it decodes through here with no error AND ends exactly at end-of-buffer
//! ([`Decoded910::end_pos`] == input length).
//!
//! SOURCE OF TRUTH: the committed client decoder, ported branch-for-branch. Line
//! references in comments point at `Component.java`.

use crate::error::{CacheError, Result};
use crate::packet::Packet;

/// What went wrong decoding a component through the 910 mirror. The variants name
/// the two failure shapes the transcoder must rule out: a misaligned op-name
/// index (the live AIOOBE) and a short/long buffer (general misalignment).
#[derive(Debug)]
pub enum Decode910Error {
    /// The second op-name index (`Component.decode:972`) addressed past the
    /// `opname` array the first op-name count sized — the exact live crash.
    OpnameIndexOutOfBounds {
        /// The out-of-range index read from the stream.
        index: usize,
        /// The length of the `opname` array (`firstIndex + 1`).
        len: usize,
    },
    /// A packet primitive read past the end of the component buffer — the other
    /// way a misaligned stream manifests (the live client would AIOOBE in
    /// `Packet`).
    UnexpectedEof(CacheError),
    /// The decode finished cleanly but did not consume the whole buffer: trailing
    /// bytes remain, i.e. the 910 decoder under-read a newer-version component.
    TrailingBytes {
        /// Number of unconsumed bytes after a clean field walk.
        remaining: usize,
    },
    /// A hook argument carried a type tag the client does not handle (0/1 only).
    BadHookArg(u8),
}

impl std::fmt::Display for Decode910Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpnameIndexOutOfBounds { index, len } => write!(
                f,
                "opname index {index} out of bounds for opname length {len} (Component.decode:973 AIOOBE)"
            ),
            Self::UnexpectedEof(e) => write!(f, "unexpected end of component buffer: {e}"),
            Self::TrailingBytes { remaining } => {
                write!(
                    f,
                    "910 decode left {remaining} trailing bytes (misalignment)"
                )
            }
            Self::BadHookArg(t) => write!(f, "unhandled hook argument type {t}"),
        }
    }
}

impl std::error::Error for Decode910Error {}

/// The load-bearing fields the 910 mirror recovers — enough to assert the
/// transcode preserved meaning (ops, op-name cursors, font/sprite/model/script
/// refs, params) and to prove the op-name section decoded sanely.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decoded910 {
    /// Wire `version` byte as the 910 decoder saw it (`255` → `-1`).
    pub version: i32,
    /// Component `type` (low 7 bits; the name-present bit is stripped).
    pub type_id: u8,
    /// Optional author name (present iff the `0x80` type bit was set).
    pub name: Option<String>,
    /// Right-click op labels in slot order (empty string for a present-but-blank
    /// slot, matching the client's `op[i]`).
    pub ops: Vec<String>,
    /// `(opIndex, cursorId)` pairs from the op-name section (the crash site).
    pub op_cursors: Vec<(usize, i32)>,
    /// Sprite/graphic ids referenced (graphic part + every sprite sub-part).
    pub graphics: Vec<i32>,
    /// Font-metrics ids referenced by text parts.
    pub fonts: Vec<i32>,
    /// Static text a type-4 text component renders, when any (empty otherwise).
    pub text: String,
    /// Model id referenced by a type-6 model component, when any.
    pub model: Option<i32>,
    /// Script ids referenced by component hooks, in decode order.
    pub scripts: Vec<i32>,
    /// `(paramKey, intValue)` int params from the common tail.
    pub int_params: Vec<(u32, i32)>,
    /// `(paramKey, strValue)` string params from the common tail.
    pub str_params: Vec<(u32, String)>,
    /// Final read position; equals the input length for a correctly-sized buffer.
    pub end_pos: usize,
}

/// Convenience: map a packet read error into [`Decode910Error::UnexpectedEof`].
fn eof<T>(r: Result<T>) -> std::result::Result<T, Decode910Error> {
    r.map_err(Decode910Error::UnexpectedEof)
}

/// Decode one component's bytes through the faithful 910 mirror. `parentlayer`
/// only feeds the (cosmetic) layer rebase and is irrelevant to byte alignment, so
/// it is fixed at 0 here.
///
/// Returns the recovered [`Decoded910`] on a clean, exactly-sized decode; returns
/// a [`Decode910Error`] for the two misalignment shapes the transcoder must
/// prevent.
pub fn decode_component_910(bytes: &[u8]) -> std::result::Result<Decoded910, Decode910Error> {
    let mut p = Packet::new(bytes);
    let mut out = Decoded910::default();

    // ── header (Component.decode:792-824) ──
    let mut version = i32::from(eof(p.g1())?);
    if version == 255 {
        version = -1;
    }
    out.version = version;

    let mut type_id = eof(p.g1())?;
    if (type_id & 0x80) != 0 {
        type_id &= 0x7F;
        out.name = Some(eof(p.gjstr())?);
    }
    out.type_id = type_id;

    let _clientcode = eof(p.g2())?;
    let _xpos = eof(p.g2s())?;
    let _ypos = eof(p.g2s())?;
    let _wsize = eof(p.g2())?;
    let _hsize = eof(p.g2())?;
    let field2356 = eof(p.g1s())?;
    let field2174 = eof(p.g1s())?;
    let _xmode = eof(p.g1s())?;
    let _ymode = eof(p.g1s())?;
    if field2356 == 4 || field2174 == 4 {
        let _aspectwidth = eof(p.g2())?;
        let _aspectheight = eof(p.g2())?;
    }
    let _layer = eof(p.g2())?;
    // flag byte: bit 0 = hide, bit 1 = noclickthrough (version >= 0). Neither
    // drives byte alignment, so the value is read and discarded.
    let _flag_byte = eof(p.g1())?;

    // ── type bodies (Component.decode:825-921) ──
    match type_id {
        0 => decode_layer_910(&mut p, version)?,
        5 => decode_graphic_910(&mut p, version, &mut out)?,
        6 => decode_model_910(&mut p, &mut out, field2356, field2174)?,
        4 => decode_text_910(&mut p, version, &mut out)?,
        3 => decode_rect_910(&mut p)?,
        9 => decode_line_910(&mut p)?,
        _ => {}
    }

    // ── common tail (Component.decode:922-1054) ──
    if version >= 6 {
        let _stylesheet = eof(p.g4s())?;
    }
    if version >= 9 {
        let _unknown = eof(p.g1())?;
    }
    let server_key_flags = if version < 6 {
        eof(p.g3())? as i32
    } else {
        eof(p.g4s())?
    };

    // var/transform block (Component.decode:929-952): a chain of `varFlag` bytes.
    let mut var_flag = eof(p.g1())?;
    while var_flag != 0 {
        let _combined = i32::from(var_flag) << 8 | i32::from(eof(p.g1())?);
        let _src = eof(p.g1())?;
        let _dst = eof(p.g1())?;
        var_flag = eof(p.g1())?;
    }

    let _opbase = eof(p.gjstr())?;
    let op_packed = eof(p.g1())?;
    let op_count = (op_packed & 0xF) as usize;
    let op_name_count = (op_packed >> 4) as usize;
    if op_count > 0 {
        for _ in 0..op_count {
            out.ops.push(eof(p.gjstr())?);
        }
    }
    // op-name section — the live crash site (Component.decode:963-974).
    let mut opname_len = 0usize;
    if op_name_count > 0 {
        let idx = usize::from(eof(p.g1())?);
        opname_len = idx + 1;
        let cursor = i32::from(eof(p.g2())?);
        out.op_cursors.push((idx, cursor));
    }
    if op_name_count > 1 {
        let idx2 = usize::from(eof(p.g1())?);
        // The client indexes `this.opname[idx2]` into an array sized by the FIRST
        // index; a misaligned stream makes `idx2` (often 255) overrun it.
        if idx2 >= opname_len {
            return Err(Decode910Error::OpnameIndexOutOfBounds {
                index: idx2,
                len: opname_len,
            });
        }
        let cursor = i32::from(eof(p.g2())?);
        out.op_cursors.push((idx2, cursor));
    }

    let _pausetext = eof(p.gjstr())?;
    let _dragdeadzone = eof(p.g1())?;
    let _dragdeadtime = eof(p.g1())?;
    let _dragrenderbehaviour = eof(p.g1())?;
    let _targetverb = eof(p.gjstr())?;

    // Target-cursor block (Component.decode:984-997): present iff the server-key
    // flags carry a non-zero target-cursor field.
    if target_cursor_from_flags(server_key_flags) != 0 {
        let _targetcursor = eof(p.g2())?;
        let _field2202 = eof(p.g2())?;
        let _field2269 = eof(p.g2())?;
    }
    if version >= 0 {
        let _mouseover = eof(p.g2())?;
    }

    if version >= 0 {
        let int_param_count = eof(p.g1())?;
        for _ in 0..int_param_count {
            let key = eof(p.g3())?;
            let val = eof(p.g4s())?;
            out.int_params.push((key, val));
        }
        let str_param_count = eof(p.g1())?;
        for _ in 0..str_param_count {
            let key = eof(p.g3())?;
            let val = eof(p.gjstr2())?;
            out.str_params.push((key, val));
        }
    }

    // Hooks (Component.decode:1019-1049). Order/gating mirror the client exactly.
    decode_hook_910(&mut p, &mut out)?; // onload
    decode_hook_910(&mut p, &mut out)?; // onmouseover
    decode_hook_910(&mut p, &mut out)?; // onmouseleave
    decode_hook_910(&mut p, &mut out)?; // ontargetleave
    decode_hook_910(&mut p, &mut out)?; // ontargetenter
    decode_hook_910(&mut p, &mut out)?; // onvartransmit
    decode_hook_910(&mut p, &mut out)?; // oninvtransmit
    decode_hook_910(&mut p, &mut out)?; // onstattransmit
    decode_hook_910(&mut p, &mut out)?; // ontimer
    decode_hook_910(&mut p, &mut out)?; // onop
    if version >= 0 {
        decode_hook_910(&mut p, &mut out)?; // onopt
    }
    decode_hook_910(&mut p, &mut out)?; // onmouserepeat
    decode_hook_910(&mut p, &mut out)?; // onclick
    decode_hook_910(&mut p, &mut out)?; // onclickrepeat
    decode_hook_910(&mut p, &mut out)?; // onrelease
    decode_hook_910(&mut p, &mut out)?; // onhold
    decode_hook_910(&mut p, &mut out)?; // ondrag
    decode_hook_910(&mut p, &mut out)?; // ondragcomplete
    decode_hook_910(&mut p, &mut out)?; // onscrollwheel
    decode_hook_910(&mut p, &mut out)?; // onvarctransmit
    decode_hook_910(&mut p, &mut out)?; // onvarcstrtransmit
    if version >= 6 {
        decode_hook_910(&mut p, &mut out)?; // onbuttonclick
        decode_hook_910(&mut p, &mut out)?; // onhook51
        decode_hook_910(&mut p, &mut out)?; // onlistselect
    }
    if version >= 8 {
        decode_hook_910(&mut p, &mut out)?; // onupdated
    }

    // Transmit lists (Component.decode:1050-1054).
    for _ in 0..5 {
        decode_transmit_list_910(&mut p)?;
    }

    out.end_pos = p.pos();
    let remaining = bytes.len() - p.pos();
    if remaining != 0 {
        return Err(Decode910Error::TrailingBytes { remaining });
    }
    Ok(out)
}

/// type 0 layer (Component.decode:825-841).
fn decode_layer_910(p: &mut Packet<'_>, version: i32) -> std::result::Result<(), Decode910Error> {
    let _scrollwidth = eof(p.g2())?;
    let _scrollheight = eof(p.g2())?;
    if version < 0 {
        let _ = eof(p.g1())?;
    } else if version >= 9 {
        for _ in 0..4 {
            let _ = eof(p.g1())?;
        }
    } else if version >= 6 {
        for _ in 0..4 {
            let _ = eof(p.g2())?;
        }
    }
    Ok(())
}

/// type 5 graphic (Component.decode:842-863).
fn decode_graphic_910(
    p: &mut Packet<'_>,
    version: i32,
    out: &mut Decoded910,
) -> std::result::Result<(), Decode910Error> {
    let graphic = eof(p.g4s())?;
    if graphic != -1 {
        out.graphics.push(graphic);
    }
    let _angle2d = eof(p.g2())?;
    let _graphic_flags = eof(p.g1())?;
    let _trans = eof(p.g1())?;
    let _outline = eof(p.g1())?;
    let _graphicshadow = eof(p.g4s())?;
    let _vflip = eof(p.g1())?;
    let _hflip = eof(p.g1())?;
    let _colour = eof(p.g4s())?;
    if version >= 3 {
        let _clickmask = eof(p.g1())?;
    }
    if version >= 6 {
        for _ in 0..4 {
            let _ = eof(p.g1())?;
        }
    }
    Ok(())
}

/// type 6 model (Component.decode:864-895).
fn decode_model_910(
    p: &mut Packet<'_>,
    out: &mut Decoded910,
    field2356: i8,
    field2174: i8,
) -> std::result::Result<(), Decode910Error> {
    let model = eof(p.gsmart2or4null())?;
    out.model = Some(model);
    let model_flags = eof(p.g1())?;
    let has_origin_xy = (model_flags & 0x1) == 1;
    let field2274 = (model_flags & 0x2) == 2;
    if has_origin_xy {
        let _ox = eof(p.g2s())?;
        let _oy = eof(p.g2s())?;
        let _ax = eof(p.g2())?;
        let _ay = eof(p.g2())?;
        let _az = eof(p.g2())?;
        let _zoom = eof(p.g2())?;
    } else if field2274 {
        let _ox = eof(p.g2s())?;
        let _oy = eof(p.g2s())?;
        let _oz = eof(p.g2s())?;
        let _ax = eof(p.g2())?;
        let _ay = eof(p.g2())?;
        let _az = eof(p.g2())?;
        let _zoom = eof(p.g2s())?;
    }
    let _modelanim = eof(p.gsmart2or4null())?;
    if field2356 != 0 {
        let _modelobjwidth = eof(p.g2())?;
    }
    if field2174 != 0 {
        let _field2238 = eof(p.g2())?;
    }
    Ok(())
}

/// type 4 text (Component.decode:896-911).
fn decode_text_910(
    p: &mut Packet<'_>,
    version: i32,
    out: &mut Decoded910,
) -> std::result::Result<(), Decode910Error> {
    let textfont = eof(p.gsmart2or4null())?;
    if textfont != -1 {
        out.fonts.push(textfont);
    }
    if version >= 2 {
        let _fontmono = eof(p.g1())?;
    }
    out.text = eof(p.gjstr())?;
    let _field2229 = eof(p.g1())?;
    let _field2223 = eof(p.g1())?;
    let _field2264 = eof(p.g1())?;
    let _textshadow = eof(p.g1())?;
    let _colour = eof(p.g4s())?;
    let _trans = eof(p.g1())?;
    if version >= 0 {
        let _maxlines = eof(p.g1())?;
    }
    Ok(())
}

/// type 3 rectangle (Component.decode:912-916).
fn decode_rect_910(p: &mut Packet<'_>) -> std::result::Result<(), Decode910Error> {
    let _colour = eof(p.g4s())?;
    let _fill = eof(p.g1())?;
    let _trans = eof(p.g1())?;
    Ok(())
}

/// type 9 line (Component.decode:917-921).
fn decode_line_910(p: &mut Packet<'_>) -> std::result::Result<(), Decode910Error> {
    let _linewid = eof(p.g1())?;
    let _colour = eof(p.g4s())?;
    let _linedirection = eof(p.g1())?;
    Ok(())
}

/// One hook (Component.decodeHook:1057-1074): count byte, then `count` typed args
/// where arg 0 = int (g4s), arg 1 = string (gjstr). The first arg the 910 client
/// reads is the SCRIPT id when present — but note the client's decodeHook treats
/// the whole list uniformly, so the script id is arg[0] with type 0.
fn decode_hook_910(
    p: &mut Packet<'_>,
    out: &mut Decoded910,
) -> std::result::Result<(), Decode910Error> {
    let count = usize::from(eof(p.g1())?);
    if count == 0 {
        return Ok(());
    }
    // The client reads `count` args; for a real hook the trigger/script is encoded
    // as the args. We surface every int arg that looks like a script ref by
    // capturing arg[0] (the script id in the if3 hook convention is the first
    // int). To stay faithful we simply walk all args.
    for i in 0..count {
        match eof(p.g1())? {
            0 => {
                let v = eof(p.g4s())?;
                if i == 0 {
                    out.scripts.push(v);
                }
            }
            1 => {
                let _ = eof(p.gjstr())?;
            }
            other => return Err(Decode910Error::BadHookArg(other)),
        }
    }
    Ok(())
}

/// One transmit list (Component.decodeTransmitList:1076-1087): count byte then
/// `count` × g4s ids.
fn decode_transmit_list_910(p: &mut Packet<'_>) -> std::result::Result<(), Decode910Error> {
    let count = usize::from(eof(p.g1())?);
    for _ in 0..count {
        let _ = eof(p.g4s())?;
    }
    Ok(())
}

/// Mirror of `ServerKeyProperties.getTargetCursorFromFlags`: the 910 client stores
/// the target-cursor presence in bit-field of the server-key flags word. The
/// target-cursor triple is read iff this is non-zero.
///
/// SOURCE OF TRUTH: `ServerKeyProperties.getTargetCursorFromFlags`. The flags word
/// packs the target mask in bits 11..=17 (7 bits); a non-zero target mask means a
/// target cursor is configured. We mirror the client's predicate by testing that
/// span, which is what drives the extra three `g2` reads.
const fn target_cursor_from_flags(flags: i32) -> i32 {
    (flags >> 11) & 0x7F
}
