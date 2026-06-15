//! Named, opt-in IR→IR lowering passes for the interface IR (plan §6 / §9 step 5).
//!
//! The interface analogue of the CS2 [`super`] passes: a typed transform over the
//! [`InterfaceIr`] that bridges a 948→910 representability gap. The single pass
//! here is the *composite-widget downcode* — the relic/ritual pattern made a named
//! pass instead of a hand-stub:
//!
//!  * [`list_to_server_driven`] — every component whose [`ComponentKind`] is a
//!    composite widget the 910 client lacks a `Component.decode` body for (Button,
//!    Check, Input, List/dropdown, Grid, Panel, CrmView) is lowered to a
//!    910-decodable primitive: a [`ComponentKind::Text`] when a visible label
//!    survives (so it still renders), else an empty interactive
//!    [`ComponentKind::Layer`] (an invisible but fully-clickable hotspot). The
//!    component's ops / op-cursors / hooks / params live in the common TAIL, which
//!    is carried verbatim, so interactivity is intact and the server still drives
//!    the action (via `setComponentEvents`). The original widget body bytes are
//!    DROPPED — exactly what makes the stream re-alignable for the 910 decoder.
//!
//! This pass is gated on the *target descriptor's* capability: when the 910 client
//! gains the `cc_list` / composite-widget family (descriptor capability flips), the
//! lowering is a no-op and the widgets are directly representable (plan §8).

use crate::error::Result;
use crate::packet::ByteWriter;
use crate::port::book::BuildDescriptor;
use crate::port::ir::interface::{Body, Component, ComponentKind, InterfaceIr, TextPart};

/// The wire version the 910 decoder fully handles; downcoded bodies are written at
/// this version. Shared with [`crate::interface::transcode::TARGET_VERSION`].
pub const TARGET_VERSION: u8 = crate::interface::transcode::TARGET_VERSION;

/// What a single component downcode did, for the port report / oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Downcoded {
    /// A primitive type — left unchanged.
    Kept,
    /// A composite widget rewritten to a layer (no visible label survived).
    ToLayer {
        /// The original (composite) kind.
        from: ComponentKind,
        /// Dropped widget-body byte count.
        dropped: usize,
    },
    /// A composite widget rewritten to a text component (a label survived).
    ToText {
        /// The original (composite) kind.
        from: ComponentKind,
        /// Dropped widget-body byte count.
        dropped: usize,
    },
}

/// Lower every composite widget the *target* cannot represent to a 910-decodable
/// primitive. Returns the per-component disposition in id order (the same order as
/// `ir.components`). When the target's `cc_list` capability is set, this is a
/// no-op (every component is [`Downcoded::Kept`]).
///
/// The pass mutates `body` in place: a downcoded component's body becomes a
/// [`Body::Raw`] holding the synthesized v9 layer/text body, and its `kind` /
/// `version` are rewritten so the encoder emits a 910-decodable component.
pub fn list_to_server_driven(ir: &mut InterfaceIr, target: &BuildDescriptor) -> Result<Vec<Downcoded>> {
    let mut report = Vec::with_capacity(ir.components.len());
    for component in &mut ir.components {
        report.push(downcode_component(component, target)?);
    }
    Ok(report)
}

/// Downcode one component (if needed). The capability check is the seam with the
/// client (plan §8): a target that has the composite-widget family keeps the
/// widget; one that lacks it lowers to a primitive.
fn downcode_component(component: &mut Component, target: &BuildDescriptor) -> Result<Downcoded> {
    // A primitive type, or a target that can already represent the composite
    // family → keep unchanged. (Only the downcode rewrites the version/body; kept
    // components are re-versioned by the encoder.)
    if component.kind.is_primitive() || target.capabilities.cc_list {
        return Ok(Downcoded::Kept);
    }

    let from = component.kind;
    let (label, dropped) = match &component.body {
        Body::Raw(_) => (None, 0),
        Body::Composite { text, raw_len } => (text.clone(), *raw_len),
    };

    // Choose the target primitive by whether a visible label survives.
    if let Some(tp) = label.filter(|t| !t.text.is_empty()) {
        component.kind = ComponentKind::Text;
        component.body = Body::Raw(text_body_v9(&tp)?);
        Ok(Downcoded::ToText { from, dropped })
    } else {
        component.kind = ComponentKind::Layer;
        component.body = Body::Raw(layer_body_v9());
        Ok(Downcoded::ToLayer { from, dropped })
    }
}

/// Synthesize the v9 `type 0` (empty layer) body bytes — `scrollwidth=0`,
/// `scrollheight=0`, then the four zero `version >= 9` margin bytes. Delegates to
/// the transcoder's writer so the bytes are identical.
fn layer_body_v9() -> Vec<u8> {
    let mut w = ByteWriter::new();
    crate::interface::transcode::write_layer_body_v9(&mut w);
    w.data
}

/// Synthesize the v9 `type 4` (text) body bytes from a recovered [`TextPart`].
/// Delegates to the transcoder's writer (constructing its `pub(crate)` text-part
/// shape) so the bytes are byte-identical to `interface transcode`'s output.
fn text_body_v9(tp: &TextPart) -> Result<Vec<u8>> {
    let mut w = ByteWriter::new();
    let xc = crate::interface::transcode::TextPart {
        font: tp.font,
        fontmono: tp.fontmono,
        text: tp.text.clone(),
        line_height: tp.line_height,
        align_h: tp.align_h,
        align_v: tp.align_v,
        shadow: tp.shadow,
        colour: tp.colour,
        trans: tp.trans,
        maxlines: tp.maxlines,
    };
    crate::interface::transcode::write_text_body_v9(&mut w, &xc)?;
    Ok(w.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn target_910() -> BuildDescriptor {
        BuildDescriptor::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"), 910)
            .expect("load 910 descriptor")
    }

    #[test]
    fn button_with_no_label_downcodes_to_layer() {
        let mut ir = InterfaceIr {
            group: 0,
            components: vec![Component {
                version: 11,
                kind: ComponentKind::Button,
                name_bit: false,
                header_tail: vec![],
                body: Body::Composite { text: None, raw_len: 52 },
                tail: vec![],
            }],
        };
        let report = list_to_server_driven(&mut ir, &target_910()).expect("lower");
        assert_eq!(report[0], Downcoded::ToLayer { from: ComponentKind::Button, dropped: 52 });
        assert_eq!(ir.components[0].kind, ComponentKind::Layer);
        assert!(matches!(ir.components[0].body, Body::Raw(_)));
    }

    #[test]
    fn check_with_label_downcodes_to_text() {
        let tp = TextPart { text: "Show Locked".to_string(), ..TextPart::default() };
        let mut ir = InterfaceIr {
            group: 0,
            components: vec![Component {
                version: 11,
                kind: ComponentKind::Check,
                name_bit: false,
                header_tail: vec![],
                body: Body::Composite { text: Some(tp), raw_len: 58 },
                tail: vec![],
            }],
        };
        let report = list_to_server_driven(&mut ir, &target_910()).expect("lower");
        assert_eq!(report[0], Downcoded::ToText { from: ComponentKind::Check, dropped: 58 });
        assert_eq!(ir.components[0].kind, ComponentKind::Text);
    }

    #[test]
    fn primitive_text_is_kept() {
        let mut ir = InterfaceIr {
            group: 0,
            components: vec![Component {
                version: 11,
                kind: ComponentKind::Text,
                name_bit: false,
                header_tail: vec![],
                body: Body::Raw(vec![1, 2, 3]),
                tail: vec![],
            }],
        };
        let report = list_to_server_driven(&mut ir, &target_910()).expect("lower");
        assert_eq!(report[0], Downcoded::Kept);
        assert_eq!(ir.components[0].body, Body::Raw(vec![1, 2, 3]));
    }
}
