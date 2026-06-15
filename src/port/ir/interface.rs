//! The typed interface-component intermediate representation (plan §4.2).
//!
//! A build-neutral, in-memory model of one interface group's components, the
//! interface analogue of [`super::cs2`]. Each [`Component`] carries a semantic
//! [`ComponentKind`] (`Layer|Rect|Text|Graphic|Model|Line|Button|Check|Input|
//! List|…`) plus its geometry/ops/hooks, modelled so that the 948→910 downcode is
//! a *typed IR pass* (a [`ComponentKind::Button`] lowered to a
//! [`ComponentKind::Layer`]) rather than a blind byte rewrite.
//!
//! # Byte-exactness contract
//! The 910 `Component.decode` is a `version`-led linear decoder whose HEADER and
//! common TAIL are byte-identical across the donor (948, wire version 11) and the
//! 910 target at version 9 — proven empirically by the transcode oracle (all 225
//! components of interface 691 and the 47 primitive components of 1224 round-trip
//! unchanged). The ONLY bytes the 910 decoder cannot consume are the *body* of an
//! unsupported composite-widget `type`.
//!
//! So the IR carries the header (sans the version + type bytes, which it models
//! typed) and the tail as VERBATIM raw bytes — the encoder re-emits them
//! unchanged — and models the body semantically: for a primitive type the raw
//! body bytes are kept ([`Body::Raw`]); for a composite widget the recoverable
//! text part is lifted ([`Body::Composite`]) so a downcode can re-synthesize a
//! 910-decodable body that preserves the label. This is what lets the
//! [`super::super::encode::interface`] back-end reproduce the committed
//! `1224-910.dat` byte-for-byte while the lowering stays typed.

/// The semantic identity of a component, independent of the numeric `type` byte
/// any build assigns it (plan §4.2). The primitive kinds {Layer, Rect, Text,
/// Graphic, Model, Line} have a 910 `Component.decode` body; the composite kinds
/// {Button, Check, Input, List, Grid, Panel, CrmView} do NOT (the 910 decoder
/// skips their body, misaligning the stream → the `Component.decode:973` AIOOBE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentKind {
    /// type 0 — a layer / container (an interactive hotspot when it carries ops).
    Layer,
    /// type 3 — a filled rectangle.
    Rect,
    /// type 4 — static text.
    Text,
    /// type 5 — a sprite / graphic.
    Graphic,
    /// type 6 — a 3D model.
    Model,
    /// type 9 — a line.
    Line,
    /// type 10 — a button (composite; sprite + text parts).
    Button,
    /// type 11 — a panel (composite).
    Panel,
    /// type 12 — a checkbox (composite; sprite + text parts).
    Check,
    /// type 13 — a text-input box (composite; sprite + text + scrollbar).
    Input,
    /// type 15 — an item grid (composite).
    Grid,
    /// type 16 — a list / dropdown (composite; the `cc_list` family).
    List,
    /// type 26 — a CRM (content) view (composite).
    CrmView,
}

impl ComponentKind {
    /// The numeric `type` id this kind is encoded as on the wire. Inverse of
    /// [`Self::from_type_id`].
    #[must_use]
    pub const fn type_id(self) -> u8 {
        match self {
            Self::Layer => 0,
            Self::Rect => 3,
            Self::Text => 4,
            Self::Graphic => 5,
            Self::Model => 6,
            Self::Line => 9,
            Self::Button => 10,
            Self::Panel => 11,
            Self::Check => 12,
            Self::Input => 13,
            Self::Grid => 15,
            Self::List => 16,
            Self::CrmView => 26,
        }
    }

    /// Classify a numeric `type` byte (low 7 bits) into a [`ComponentKind`], or
    /// `None` for a type no decoder in the crate handles.
    #[must_use]
    pub const fn from_type_id(type_id: u8) -> Option<Self> {
        Some(match type_id {
            0 => Self::Layer,
            3 => Self::Rect,
            4 => Self::Text,
            5 => Self::Graphic,
            6 => Self::Model,
            9 => Self::Line,
            10 => Self::Button,
            11 => Self::Panel,
            12 => Self::Check,
            13 => Self::Input,
            15 => Self::Grid,
            16 => Self::List,
            26 => Self::CrmView,
            _ => return None,
        })
    }

    /// Whether the 910 `Component.decode` implements a body for this kind. The
    /// composite kinds return `false` — they must be lowered before encode for a
    /// target whose `cc_list` / composite-widget capability is absent.
    #[must_use]
    pub const fn is_primitive(self) -> bool {
        matches!(
            self,
            Self::Layer | Self::Rect | Self::Text | Self::Graphic | Self::Model | Self::Line
        )
    }
}

/// The text-part fields recovered from a component (a `type 4` text body or the
/// text part embedded in a composite widget). Enough to re-emit a 910 `type 4`
/// body so a downcoded label still renders. Field order matches the 910
/// `Component.decode` text reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextPart {
    /// Font-metrics id (`gSmart2or4null`; `-1` = default font).
    pub font: i32,
    /// Whether the font is monospaced (`version >= 2`).
    pub fontmono: bool,
    /// The static text.
    pub text: String,
    /// `field2229` — line height.
    pub line_height: u8,
    /// `field2223` — horizontal align.
    pub align_h: u8,
    /// `field2264` — vertical align.
    pub align_v: u8,
    /// Whether a text shadow is drawn.
    pub shadow: bool,
    /// RGB colour (`g4s`).
    pub colour: i32,
    /// Transparency.
    pub trans: u8,
    /// Max wrapped lines (`version >= 0`).
    pub maxlines: u8,
}

/// A component's body, modelled either as VERBATIM raw bytes (a primitive type
/// the target decodes directly) or as a recovered composite-widget body (with its
/// lifted text part + the raw body length, which the lowering drops).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// A primitive type's body, carried byte-verbatim (re-emitted unchanged).
    Raw(Vec<u8>),
    /// A composite widget's body: the recovered text part (when it had a non-empty
    /// label, for the text downcode) plus the original body byte length (which the
    /// lowering drops). The raw body bytes are NOT retained — a composite body is
    /// never re-emitted as-is on a target that lacks the widget.
    Composite {
        /// The recovered text part, if the widget had one with a non-empty label.
        text: Option<TextPart>,
        /// Number of original widget-body bytes (dropped by the lowering).
        raw_len: usize,
    },
}

/// One interface component as typed IR.
///
/// `header_tail` and `tail` are VERBATIM raw bytes (version-independent layout) —
/// the encoder re-emits them unchanged, which is what preserves byte-exactness.
/// `version`, `kind`, `name_bit` and `body` are the typed facets the lowering and
/// encoder operate on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// The wire `version` byte (`-1` is encoded as `255`). The donor is 11; the
    /// 910 encoder forces it to 9.
    pub version: i16,
    /// The component's semantic kind.
    pub kind: ComponentKind,
    /// Whether the `0x80` name-present bit was set on the type byte (the author
    /// name then leads `header_tail`). Preserved so the encoder re-sets it.
    pub name_bit: bool,
    /// The header bytes AFTER the version + type bytes (the optional name string,
    /// clientcode, position, sizes, modes, aspect, layer, flags) — VERBATIM.
    pub header_tail: Vec<u8>,
    /// The component body (raw for primitives, recovered for composites).
    pub body: Body,
    /// The common-tail bytes (var/transform block, ops, hooks, params, transmit
    /// lists) — VERBATIM.
    pub tail: Vec<u8>,
}

impl Component {
    /// Whether this component needs a downcode to be representable on a target
    /// whose composite-widget capability is absent (a non-primitive kind).
    #[must_use]
    pub const fn needs_downcode(&self) -> bool {
        !self.kind.is_primitive()
    }
}

/// A whole interface group as typed IR: the dense `0..n` component roster in id
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceIr {
    /// The interface group id (carried for diagnostics).
    pub group: u32,
    /// Components in ascending id order. The id is the dense index `0..n` the
    /// interface archive uses.
    pub components: Vec<Component>,
}

impl Component {
    /// Lift one donor (948/947 layout) component's raw bytes into typed IR.
    ///
    /// Reuses the single donor field-walk in
    /// [`crate::interface::transcode::segment`] (the decode source of truth) to
    /// find the header / body / tail boundaries, then splits the raw bytes into the
    /// verbatim header-tail + tail and the typed body. A primitive type keeps its
    /// body bytes verbatim ([`Body::Raw`]); a composite widget lifts its recovered
    /// text part ([`Body::Composite`]).
    pub fn from_donor_bytes(bytes: &[u8], build: u32) -> crate::error::Result<Self> {
        use crate::interface::transcode::segment;

        let seg = segment(bytes, build)?;
        // The version + type bytes lead the header; everything after them is the
        // verbatim header-tail. The type byte may carry the 0x80 name bit.
        let mut version = i16::from(bytes[0]);
        if version == i16::from(u8::MAX) {
            version = -1;
        }
        let name_bit = (bytes[1] & 0x80) != 0;
        let kind = ComponentKind::from_type_id(seg.type_id).ok_or_else(|| {
            crate::error::CacheError::message(format!(
                "component type {} has no semantic ComponentKind",
                seg.type_id
            ))
        })?;

        let header_tail = bytes[2..seg.body_start].to_vec();
        let raw_body = &bytes[seg.body_start..seg.body_end];
        let tail = bytes[seg.body_end..].to_vec();

        let body = if kind.is_primitive() {
            Body::Raw(raw_body.to_vec())
        } else {
            Body::Composite {
                text: seg.text_part.map(|tp| TextPart {
                    font: tp.font,
                    fontmono: tp.fontmono,
                    text: tp.text,
                    line_height: tp.line_height,
                    align_h: tp.align_h,
                    align_v: tp.align_v,
                    shadow: tp.shadow,
                    colour: tp.colour,
                    trans: tp.trans,
                    maxlines: tp.maxlines,
                }),
                raw_len: raw_body.len(),
            }
        };

        Ok(Self {
            version,
            kind,
            name_bit,
            header_tail,
            body,
            tail,
        })
    }
}

impl InterfaceIr {
    /// Lift a whole interface group's component file map (dense `0..n` ids) into
    /// typed IR. `build` is the layout the donor components decode at (947/948).
    pub fn from_donor_files(
        group: u32,
        files: &std::collections::BTreeMap<u32, Vec<u8>>,
        build: u32,
    ) -> crate::error::Result<Self> {
        let mut components = Vec::with_capacity(files.len());
        for (&id, bytes) in files {
            let c = Component::from_donor_bytes(bytes, build).map_err(|e| {
                crate::error::CacheError::message(format!("decode component {id}: {e}"))
            })?;
            components.push(c);
        }
        Ok(Self { group, components })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_kind_type_id_round_trips() {
        for t in [0u8, 3, 4, 5, 6, 9, 10, 11, 12, 13, 15, 16, 26] {
            let kind = ComponentKind::from_type_id(t).expect("known type");
            assert_eq!(kind.type_id(), t, "type {t} round-trips");
        }
        assert!(ComponentKind::from_type_id(99).is_none());
    }

    #[test]
    fn primitive_classification_matches_910_body_set() {
        for k in [
            ComponentKind::Layer,
            ComponentKind::Rect,
            ComponentKind::Text,
            ComponentKind::Graphic,
            ComponentKind::Model,
            ComponentKind::Line,
        ] {
            assert!(k.is_primitive(), "{k:?} is a primitive 910 type");
        }
        for k in [
            ComponentKind::Button,
            ComponentKind::Check,
            ComponentKind::Input,
            ComponentKind::List,
            ComponentKind::Grid,
            ComponentKind::Panel,
            ComponentKind::CrmView,
        ] {
            assert!(!k.is_primitive(), "{k:?} is a composite (no 910 body)");
        }
    }
}
