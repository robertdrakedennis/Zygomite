//! Faithful round-trip codec for interface (if3) component groups.
//!
//! [`crate::interface`] decodes components into human-readable text and never
//! reconstructs the original bytes. This module models the subset of component
//! types we need to *edit and re-pack* — `layer` (0), `rectangle` (3) and
//! `graphic` (5) — as fully typed structs that re-encode byte-for-byte.
//!
//! The decode path mirrors `parse_component` exactly so that
//! `encode_raw(decode_raw(bytes)) == bytes` for every supported component. The
//! `interface_1218_slot_template_roundtrips` integration test enforces this
//! against the real 910 cache before any clone is generated.
//!
//! Scope is deliberate: only the three container/leaf types that make up a
//! skill-guide grid slot are modelled. Other types (`text`, `button`, …) return
//! [`CodecError::UnsupportedType`] so callers can skip them.

use crate::error::Result;
use crate::packet::{ByteWriter, Packet};

/// Error raised when a component cannot be decoded into the typed model.
#[derive(Debug)]
pub enum CodecError {
    /// Component type is outside the supported {layer, rectangle, graphic} set.
    UnsupportedType(u8),
    /// Legacy (pre-566) non-versioned component — not modelled.
    LegacyUnversioned,
    /// Structural problem (e.g. >2 opcursors) that the model cannot represent.
    Malformed(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(t) => write!(f, "unsupported component type {t}"),
            Self::LegacyUnversioned => write!(f, "legacy unversioned component"),
            Self::Malformed(m) => write!(f, "malformed component: {m}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// A single hook argument: either a 32-bit int or a CP1252 string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookArg {
    Int(i32),
    Str(String),
}

/// A client-script hook (`onload`, `onop`, …). `None` slots are stored as
/// absent in [`Component::hooks`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    /// Trigger byte the client reads after the arg count (unused by us, kept raw).
    pub unknown: u8,
    /// Client-script id (`-1` == none).
    pub script: i32,
    pub args: Vec<HookArg>,
}

/// Version/build-dependent trailer of a `layer` body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerExtra {
    /// `version == -1 && build >= 495`: single noclickthrough byte.
    Noclick(u8),
    /// `version >= 9`: four 1-byte margins.
    Margin1([u8; 4]),
    /// `version >= 6`: four 2-byte margins.
    Margin2([u16; 4]),
    /// Older layouts: nothing.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerBody {
    pub scroll_width: u16,
    pub scroll_height: u16,
    pub extra: LayerExtra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RectBody {
    pub colour: i32,
    pub fill: u8,
    pub trans: u8,
}

/// A `graphic` body / sprite sub-part. Gated fields are `Some` iff the
/// version/build gate that read them was satisfied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpritePart {
    pub graphic: i32,
    pub angle: u16,
    pub flags: u8,
    pub trans: u8,
    pub outline: u8,
    pub graphic_shadow: i32,
    pub vflip: u8,
    pub hflip: u8,
    /// `build >= 537`
    pub colour: Option<i32>,
    /// `version >= 3`
    pub clickmask: Option<u8>,
    /// `version >= 6`
    pub edge: Option<[u8; 4]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Layer(LayerBody),
    Rectangle(RectBody),
    Graphic(SpritePart),
}

/// Fully typed, re-encodable interface component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    // ── header ──
    /// `-1` (stored as `0xFF`) or `0..=254`.
    pub version: i16,
    /// Component type with the name bit stripped (0/3/5).
    pub if_type: u8,
    /// `Some` iff the name bit (`0x80`) was set; inner value may be empty.
    pub name: Option<String>,
    pub contenttype: u16,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub width_mode: i8,
    pub height_mode: i8,
    pub x_mode: i8,
    pub y_mode: i8,
    /// `Some` iff `width_mode == 4 || height_mode == 4`.
    pub aspect: Option<(u16, u16)>,
    /// Parent layer id (`-1` == none).
    pub layer: i32,
    pub flags: u8,

    // ── body ──
    pub body: Body,

    // ── common tail ──
    /// `version >= 6`
    pub stylesheet: Option<i32>,
    /// `version >= 9`
    pub unknown10: Option<u8>,
    pub events: u32,
    /// Raw bytes of the opkey section (passthrough; never edited).
    pub opkey_raw: Vec<u8>,
    pub opbase: String,
    /// All `opcount` op strings, including empties (which decode hides).
    pub ops: Vec<String>,
    /// `(index, cursor)` pairs; at most 2.
    pub opcursors: Vec<(u8, u16)>,
    /// `build >= 537`
    pub pausetext: Option<String>,
    pub dragdeadzone: u8,
    pub dragdeadtime: u8,
    pub dragrenderbehaviour: u8,
    pub targetverb: String,
    /// `build >= 530 && targetmask != 0`
    pub targetcursors: Option<[i32; 3]>,
    /// `version >= 0`
    pub mouseover: Option<i32>,
    /// `version >= 0`; `(param_id, value)`.
    pub int_params: Vec<(u32, i32)>,
    /// `version >= 0`; `(param_id, value)`.
    pub str_params: Vec<(u32, String)>,
    /// Hooks for the gated slots that are present, in decode order.
    pub hooks: Vec<Option<Hook>>,
    /// Transmit lists for the gated slots that are present, in decode order.
    pub transmit_lists: Vec<Vec<i32>>,
}

/// Ordered presence gates for the hook slots in `decode_common_tail`.
/// Single source of truth shared by decode and encode.
fn hook_gates(version: i16, build: u32) -> [bool; 26] {
    [
        true,         // onload
        true,         // onmouseover
        true,         // onmouseleave
        true,         // ontargetleave
        true,         // ontargetenter
        true,         // onvartransmit
        true,         // oninvtransmit
        true,         // onstattransmit
        true,         // ontimer
        true,         // onop
        version >= 0, // onopt
        true,         // onmouserepeat
        true,         // onclick
        true,         // onclickrepeat
        true,         // onrelease
        true,         // onhold
        true,         // ondrag
        true,         // ondragcomplete
        build < 459,  // ondragcancel
        true,         // onscrollwheel
        build >= 506, // onvarctransmit
        build >= 506, // onvarcstrtransmit
        version >= 6, // onbuttonclick
        version >= 6, // onhook51
        version >= 6, // onlistselect
        version >= 8, // onupdated
    ]
}

/// Ordered presence gates for the transmit-list slots.
fn transmit_gates(build: u32) -> [bool; 5] {
    [
        build >= 459, // onvartransmitlist
        build >= 459, // oninvtransmitlist
        build >= 459, // onstattransmitlist
        build >= 506, // onvarctransmitlist
        build >= 506, // onvarcstrtransmitlist
    ]
}

fn malformed(msg: impl Into<String>) -> crate::error::CacheError {
    crate::error::CacheError::message(CodecError::Malformed(msg.into()).to_string())
}

// ── decode ──────────────────────────────────────────────────────────────────

pub fn decode_raw(data: &[u8], build: u32) -> Result<Component> {
    let mut p = Packet::new(data);

    let mut version = i16::from(p.g1()?);
    if build < 566 && version != i16::from(u8::MAX) {
        return Err(crate::error::CacheError::message(
            CodecError::LegacyUnversioned.to_string(),
        ));
    }
    if version == i16::from(u8::MAX) {
        version = -1;
    }

    let if_type_byte = p.g1()?;
    let if_type = if_type_byte & 0x7F;
    let name = if (if_type_byte & 0x80) != 0 {
        Some(p.gjstr()?)
    } else {
        None
    };

    let contenttype = p.g2()?;
    let x = p.g2s()?;
    let y = p.g2s()?;
    let width = p.g2()?;
    let height = p.g2()?;

    let (mut width_mode, mut height_mode, mut x_mode, mut y_mode) = (0_i8, 0_i8, 0_i8, 0_i8);
    if build >= 493 {
        width_mode = p.g1s()?;
        height_mode = p.g1s()?;
        x_mode = p.g1s()?;
        y_mode = p.g1s()?;
    }

    let aspect = if width_mode == 4 || height_mode == 4 {
        Some((p.g2()?, p.g2()?))
    } else {
        None
    };

    let layer = p.g2null()?;
    let flags = p.g1()?;

    let body = match if_type {
        0 => Body::Layer(decode_layer_body(&mut p, version, build)?),
        3 => Body::Rectangle(RectBody {
            colour: p.g4s()?,
            fill: p.g1()?,
            trans: p.g1()?,
        }),
        5 => Body::Graphic(decode_sprite_part(&mut p, version, build)?),
        other => {
            return Err(crate::error::CacheError::message(
                CodecError::UnsupportedType(other).to_string(),
            ));
        }
    };

    // ── common tail ──
    let stylesheet = if version >= 6 { Some(p.g4s()?) } else { None };
    let unknown10 = if version >= 9 { Some(p.g1()?) } else { None };

    let events = if version < 6 {
        p.g3()?
    } else {
        p.g4s()? as u32
    };
    let targetmask = (events >> 11) & 0x7F;

    let opkey_start = p.pos();
    skip_opkeys(&mut p, build)?;
    let opkey_raw = data[opkey_start..p.pos()].to_vec();

    let opbase = p.gjstr()?;

    let opinfo = p.g1()?;
    let opcount = opinfo & 15;
    let opcursor_count = opinfo >> 4;
    if opcursor_count > 2 {
        return Err(malformed(format!(
            "opcursor count {opcursor_count} exceeds modelled maximum of 2"
        )));
    }

    let mut ops = Vec::with_capacity(usize::from(opcount));
    for _ in 0..opcount {
        ops.push(p.gjstr()?);
    }

    let mut opcursors = Vec::with_capacity(usize::from(opcursor_count));
    for _ in 0..opcursor_count {
        opcursors.push((p.g1()?, p.g2()?));
    }

    let pausetext = if build >= 537 { Some(p.gjstr()?) } else { None };

    let dragdeadzone = p.g1()?;
    let dragdeadtime = p.g1()?;
    let dragrenderbehaviour = p.g1()?;
    let targetverb = p.gjstr()?;

    let targetcursors = if build >= 530 && targetmask != 0 {
        Some([p.g2null()?, p.g2null()?, p.g2null()?])
    } else {
        None
    };

    let mouseover = if version >= 0 {
        Some(p.g2null()?)
    } else {
        None
    };

    let (mut int_params, mut str_params) = (Vec::new(), Vec::new());
    if version >= 0 {
        let int_count = p.g1()?;
        for _ in 0..int_count {
            int_params.push((p.g3()?, p.g4s()?));
        }
        let str_count = p.g1()?;
        for _ in 0..str_count {
            str_params.push((p.g3()?, p.gjstr2()?));
        }
    }

    let mut hooks = Vec::new();
    for gate in hook_gates(version, build) {
        if gate {
            hooks.push(decode_hook(&mut p)?);
        }
    }

    let mut transmit_lists = Vec::new();
    for gate in transmit_gates(build) {
        if gate {
            transmit_lists.push(decode_transmit_list(&mut p)?);
        }
    }

    let remaining = p.len().saturating_sub(p.pos());
    if remaining != 0 {
        return Err(malformed(format!(
            "end of component not reached: {remaining} bytes remaining"
        )));
    }

    Ok(Component {
        version,
        if_type,
        name,
        contenttype,
        x,
        y,
        width,
        height,
        width_mode,
        height_mode,
        x_mode,
        y_mode,
        aspect,
        layer,
        flags,
        body,
        stylesheet,
        unknown10,
        events,
        opkey_raw,
        opbase,
        ops,
        opcursors,
        pausetext,
        dragdeadzone,
        dragdeadtime,
        dragrenderbehaviour,
        targetverb,
        targetcursors,
        mouseover,
        int_params,
        str_params,
        hooks,
        transmit_lists,
    })
}

fn decode_layer_body(p: &mut Packet<'_>, version: i16, build: u32) -> Result<LayerBody> {
    let scroll_width = p.g2()?;
    let scroll_height = p.g2()?;
    let extra = if version == -1 && build >= 495 {
        LayerExtra::Noclick(p.g1()?)
    } else if version >= 9 {
        LayerExtra::Margin1([p.g1()?, p.g1()?, p.g1()?, p.g1()?])
    } else if version >= 6 {
        LayerExtra::Margin2([p.g2()?, p.g2()?, p.g2()?, p.g2()?])
    } else {
        LayerExtra::None
    };
    Ok(LayerBody {
        scroll_width,
        scroll_height,
        extra,
    })
}

fn decode_sprite_part(p: &mut Packet<'_>, version: i16, build: u32) -> Result<SpritePart> {
    let graphic = p.g4s()?;
    let angle = p.g2()?;
    let flags = p.g1()?;
    let trans = p.g1()?;
    let outline = p.g1()?;
    let graphic_shadow = p.g4s()?;
    let vflip = p.g1()?;
    let hflip = p.g1()?;
    let colour = if build >= 537 { Some(p.g4s()?) } else { None };
    let clickmask = if version >= 3 { Some(p.g1()?) } else { None };
    let edge = if version >= 6 {
        Some([p.g1()?, p.g1()?, p.g1()?, p.g1()?])
    } else {
        None
    };
    Ok(SpritePart {
        graphic,
        angle,
        flags,
        trans,
        outline,
        graphic_shadow,
        vflip,
        hflip,
        colour,
        clickmask,
        edge,
    })
}

/// Advance past the opkey section (3 build ranges) without interpreting it.
fn skip_opkeys(p: &mut Packet<'_>, build: u32) -> Result<()> {
    if build < 499 {
        // nothing
    } else if build < 530 {
        let keycount = p.g1()?;
        for _ in 0..keycount {
            p.g1s()?;
        }
        if keycount > 0 && build >= 509 {
            let modcount = p.g1()?;
            for _ in 0..modcount {
                p.g1s()?;
            }
        }
    } else {
        let mut value = p.g1()?;
        while value != 0 {
            p.g1()?; // rate low byte
            p.g1s()?;
            p.g1s()?;
            value = p.g1()?;
        }
    }
    Ok(())
}

fn decode_hook(p: &mut Packet<'_>) -> Result<Option<Hook>> {
    let count = usize::from(p.g1()?);
    if count == 0 {
        return Ok(None);
    }
    let unknown = p.g1()?;
    let script = p.g4s()?;
    let mut args = Vec::with_capacity(count - 1);
    for _ in 0..(count - 1) {
        match p.g1()? {
            0 => args.push(HookArg::Int(p.g4s()?)),
            1 => args.push(HookArg::Str(p.gjstr()?)),
            other => return Err(malformed(format!("unexpected hook argument type {other}"))),
        }
    }
    Ok(Some(Hook {
        unknown,
        script,
        args,
    }))
}

fn decode_transmit_list(p: &mut Packet<'_>) -> Result<Vec<i32>> {
    let count = usize::from(p.g1()?);
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(p.g4s()?);
    }
    Ok(ids)
}

// ── encode ──────────────────────────────────────────────────────────────────

pub fn encode_raw(c: &Component, build: u32) -> Result<Vec<u8>> {
    let mut w = ByteWriter::new();

    // ── header ──
    w.p1(if c.version < 0 {
        u8::MAX
    } else {
        c.version as u8
    });
    let name_bit = if c.name.is_some() { 0x80 } else { 0 };
    w.p1((c.if_type & 0x7F) | name_bit);
    if let Some(name) = &c.name {
        w.pjstr(name)?;
    }
    w.p2(c.contenttype);
    w.p2(c.x as u16);
    w.p2(c.y as u16);
    w.p2(c.width);
    w.p2(c.height);
    if build >= 493 {
        w.p1(c.width_mode as u8);
        w.p1(c.height_mode as u8);
        w.p1(c.x_mode as u8);
        w.p1(c.y_mode as u8);
    }
    if c.width_mode == 4 || c.height_mode == 4 {
        let (aw, ah) = c
            .aspect
            .ok_or_else(|| malformed("aspect mode set but aspect values missing"))?;
        w.p2(aw);
        w.p2(ah);
    }
    p_g2null(&mut w, c.layer);
    w.p1(c.flags);

    // ── body ──
    match &c.body {
        Body::Layer(b) => {
            w.p2(b.scroll_width);
            w.p2(b.scroll_height);
            match &b.extra {
                LayerExtra::Noclick(v) => w.p1(*v),
                LayerExtra::Margin1(m) => {
                    for v in m {
                        w.p1(*v);
                    }
                }
                LayerExtra::Margin2(m) => {
                    for v in m {
                        w.p2(*v);
                    }
                }
                LayerExtra::None => {}
            }
        }
        Body::Rectangle(b) => {
            w.p4s(b.colour);
            w.p1(b.fill);
            w.p1(b.trans);
        }
        Body::Graphic(b) => encode_sprite_part(&mut w, b, c.version, build)?,
    }

    // ── common tail ──
    if c.version >= 6 {
        w.p4s(
            c.stylesheet
                .ok_or_else(|| malformed("stylesheet gate set but value missing"))?,
        );
    }
    if c.version >= 9 {
        w.p1(c
            .unknown10
            .ok_or_else(|| malformed("unknown10 gate set but value missing"))?);
    }

    if c.version < 6 {
        w.p3(c.events);
    } else {
        w.p4s(c.events as i32);
    }
    let targetmask = (c.events >> 11) & 0x7F;

    w.pdata(&c.opkey_raw);
    w.pjstr(&c.opbase)?;

    if c.ops.len() > 15 {
        return Err(malformed(format!("op count {} exceeds 15", c.ops.len())));
    }
    if c.opcursors.len() > 2 {
        return Err(malformed(format!(
            "opcursor count {} exceeds 2",
            c.opcursors.len()
        )));
    }
    let opinfo = (c.ops.len() as u8 & 15) | ((c.opcursors.len() as u8 & 15) << 4);
    w.p1(opinfo);
    for op in &c.ops {
        w.pjstr(op)?;
    }
    for (index, cursor) in &c.opcursors {
        w.p1(*index);
        w.p2(*cursor);
    }

    if build >= 537 {
        w.pjstr(
            c.pausetext
                .as_deref()
                .ok_or_else(|| malformed("pausetext gate set but value missing"))?,
        )?;
    }

    w.p1(c.dragdeadzone);
    w.p1(c.dragdeadtime);
    w.p1(c.dragrenderbehaviour);
    w.pjstr(&c.targetverb)?;

    if build >= 530 && targetmask != 0 {
        let tc = c
            .targetcursors
            .ok_or_else(|| malformed("targetcursor gate set but values missing"))?;
        for v in tc {
            p_g2null(&mut w, v);
        }
    }

    if c.version >= 0 {
        p_g2null(
            &mut w,
            c.mouseover
                .ok_or_else(|| malformed("mouseover gate set but value missing"))?,
        );
    }

    if c.version >= 0 {
        if c.int_params.len() > 255 || c.str_params.len() > 255 {
            return Err(malformed("param count exceeds 255"));
        }
        w.p1(c.int_params.len() as u8);
        for (param, value) in &c.int_params {
            w.p3(*param);
            w.p4s(*value);
        }
        w.p1(c.str_params.len() as u8);
        for (param, value) in &c.str_params {
            w.p3(*param);
            p_jstr2(&mut w, value)?;
        }
    }

    let mut hook_iter = c.hooks.iter();
    for gate in hook_gates(c.version, build) {
        if gate {
            let hook = hook_iter
                .next()
                .ok_or_else(|| malformed("fewer hooks stored than gates require"))?;
            encode_hook(&mut w, hook.as_ref())?;
        }
    }
    if hook_iter.next().is_some() {
        return Err(malformed("more hooks stored than gates allow"));
    }

    let mut list_iter = c.transmit_lists.iter();
    for gate in transmit_gates(build) {
        if gate {
            let list = list_iter
                .next()
                .ok_or_else(|| malformed("fewer transmit lists stored than gates require"))?;
            encode_transmit_list(&mut w, list)?;
        }
    }
    if list_iter.next().is_some() {
        return Err(malformed("more transmit lists stored than gates allow"));
    }

    Ok(w.data)
}

fn encode_sprite_part(w: &mut ByteWriter, b: &SpritePart, version: i16, build: u32) -> Result<()> {
    w.p4s(b.graphic);
    w.p2(b.angle);
    w.p1(b.flags);
    w.p1(b.trans);
    w.p1(b.outline);
    w.p4s(b.graphic_shadow);
    w.p1(b.vflip);
    w.p1(b.hflip);
    if build >= 537 {
        w.p4s(
            b.colour
                .ok_or_else(|| malformed("sprite colour gate set but value missing"))?,
        );
    }
    if version >= 3 {
        w.p1(b
            .clickmask
            .ok_or_else(|| malformed("sprite clickmask gate set but value missing"))?);
    }
    if version >= 6 {
        let edge = b
            .edge
            .ok_or_else(|| malformed("sprite edge gate set but values missing"))?;
        for v in edge {
            w.p1(v);
        }
    }
    Ok(())
}

fn encode_hook(w: &mut ByteWriter, hook: Option<&Hook>) -> Result<()> {
    match hook {
        None => w.p1(0),
        Some(h) => {
            let count = h.args.len() + 1;
            if count > 255 {
                return Err(malformed("hook argument count exceeds 254"));
            }
            w.p1(count as u8);
            w.p1(h.unknown);
            w.p4s(h.script);
            for arg in &h.args {
                match arg {
                    HookArg::Int(v) => {
                        w.p1(0);
                        w.p4s(*v);
                    }
                    HookArg::Str(s) => {
                        w.p1(1);
                        w.pjstr(s)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn encode_transmit_list(w: &mut ByteWriter, list: &[i32]) -> Result<()> {
    if list.len() > 255 {
        return Err(malformed("transmit list length exceeds 255"));
    }
    w.p1(list.len() as u8);
    for id in list {
        w.p4s(*id);
    }
    Ok(())
}

/// Encode an `i32` the way `Packet::g2null` decodes it (`-1` => `0xFFFF`).
fn p_g2null(w: &mut ByteWriter, value: i32) {
    let encoded = if value < 0 {
        0xFFFF
    } else {
        (value as u32 & 0xFFFF) as u16
    };
    w.p2(encoded);
}

/// Encode a string the way `Packet::gjstr2` decodes it: a leading `0` marker
/// byte then a NUL-terminated CP1252 string.
fn p_jstr2(w: &mut ByteWriter, value: &str) -> Result<()> {
    w.p1(0);
    w.pjstr(value)
}
