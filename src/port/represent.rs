//! Representability dry-run (plan §6): enumerate every IR construct the target
//! build cannot encode directly, classified, with the named lowering that handles
//! each (or `Unhandled` → a hard build-time failure).
//!
//! This is the "enumerate the delta up front" the old flow lacked. `port plan`
//! runs it over an interface's whole closure; `cs2 lint-splice` becomes a thin
//! `decode→represent` dry-run wrapper (plan §9 step 3).

use serde::Serialize;

use crate::port::book::BuildDescriptor;
use crate::port::ir::cs2::{Cs2Ir, Operand};

/// The classification of one unrepresentable (or representable-via-lowering)
/// construct (plan §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// An opcode the target book lacks entirely (`cc_setondropdownselect`,
    /// `cc_radiogroup_*`, `sub`, the stylesheet opcode family, …).
    MissingOp,
    /// An opcode whose arity differs between builds (`tostring`, `db_find`).
    ArityDrift,
    /// A non-canonical opcode mnemonic the target renames (`enum` → `_enum`).
    OpRename,
    /// A db-field constant whose bit-packing differs between builds (handled
    /// silently by the encoder when both packings are known).
    IdPackingDiff,
    /// A call to a proc id present in the target but as a DIFFERENT proc
    /// (different arg signature) — a proc-id collision that must be remapped.
    ProcCollision,
    /// A call to a proc absent from the target build (donor-new id).
    MissingProc,
    /// A target capability gap not tied to a single opcode (cc_list component,
    /// modern fonts, …) — reported by `port plan` from the descriptor.
    CapabilityGap,
    /// A component whose [`crate::port::ir::interface::ComponentKind`] the target
    /// build has no `Component.decode` body for (type 16/list, button, check, …) —
    /// the interface analogue of `MissingOp` (plan §6).
    MissingComponentKind,
    /// A component addressed in the donor's sparse cc-id space onto a target whose
    /// cc-model is dense (plan §6 `CcModelMismatch`). Handled silently when the
    /// target descriptor declares `cc_model_sparse = true`.
    CcModelMismatch,
}

/// The named lowering that bridges a finding, or that none is registered (→ the
/// port fails loud at encode unless the construct is independently neutralised).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bridge {
    /// `lower::sub_to_add`.
    SubToAdd,
    /// `lower::tostring_drop_radix`.
    TostringDropRadix,
    /// `tostring_localised_long` neutralisation.
    TostringLocalisedLong,
    /// `lower::dbfind_drop_tuple`.
    DbfindDropTuple,
    /// `lower::enum_rename`.
    EnumRename,
    /// `lower::dbfield_repack` (encoder-intrinsic).
    DbfieldRepack,
    /// The 948-only single-int-pop component/stylesheet opcode neutralisation.
    UnmappedPop1Neutralise,
    /// `lower::proc_alloc` (the structural proc allocator).
    ProcAlloc,
    /// `lower::list_to_server_driven` (the list/dropdown → server-driven layer).
    ListToServerDriven,
    /// No registered lowering — the construct is unrepresentable and the port
    /// must either register one, splice the family, or extend the client.
    None,
}

/// One representability finding.
#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    /// The script id the construct appears in (`None` for whole-closure / config
    /// findings).
    pub script: Option<i32>,
    /// 0-based instruction index, when applicable.
    pub instr: Option<usize>,
    /// The classification.
    pub kind: FindingKind,
    /// The opcode / construct name.
    pub construct: String,
    /// The named lowering that handles it (or `None`).
    pub bridge: Bridge,
    /// Human detail.
    pub detail: String,
}

impl Finding {
    /// Whether this finding is unhandled (no registered lowering AND not silently
    /// encodable) — i.e. a hard build-time failure if the port proceeds.
    #[must_use]
    pub fn is_unhandled(&self) -> bool {
        self.bridge == Bridge::None
    }
}

/// Variadic opcodes whose arity differs from 910 in the ritual port (the typed-
/// constant-radix drift). Mirrors the cases the Python rewrites.
fn arity_drift_bridge(op: &str) -> Option<(Bridge, &'static str)> {
    match op {
        "tostring" => Some((
            Bridge::TostringDropRadix,
            "948 `tostring(value, radix)` → 910 `tostring(value)`; drop the radix push",
        )),
        "db_find" => Some((
            Bridge::DbfindDropTuple,
            "948 `db_find(field, key, tuple)` → 910 `db_find(field, key)`; drop the tuple push",
        )),
        "tostring_localised_long" => Some((
            Bridge::TostringLocalisedLong,
            "948-only long→string localiser; neutralise (drop int + long)",
        )),
        _ => None,
    }
}

/// The named lowering for a 948-only opcode the target lacks, or `None` if no
/// lowering is registered for it.
fn missing_op_bridge(op: &str) -> (Bridge, &'static str) {
    if op == "sub" {
        return (Bridge::SubToAdd, "910 has no `sub`; rewrite `a - b` to `a + (-b)`");
    }
    if crate::port::lower::UNMAPPED_POP1_INT_OPS.contains(&op) {
        return (
            Bridge::UnmappedPop1Neutralise,
            "948-only single-int-pop component/stylesheet opcode; neutralise to pop_int_discard",
        );
    }
    if op.starts_with("cc_radiogroup") || op == "cc_setondropdownselect" || op.starts_with("cc_list")
    {
        return (
            Bridge::ListToServerDriven,
            "948-only list/dropdown/radiogroup opcode; lower to a server-driven layer or extend the client",
        );
    }
    (
        Bridge::None,
        "948-only opcode with no registered lowering — register one, splice the family, or extend the client",
    )
}

/// Run the representability dry-run over a single decoded IR script against the
/// target descriptor. `script_id` labels the findings.
#[must_use]
pub fn represent_script(ir: &Cs2Ir, script_id: Option<i32>, target: &BuildDescriptor) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (i, insn) in ir.code.iter().enumerate() {
        let op = insn.op.as_str();
        if op == "switch" {
            continue;
        }

        // db-field packing difference (silently handled by the encoder).
        if let Operand::DbFieldConst(field) = &insn.operand {
            findings.push(Finding {
                script: script_id,
                instr: Some(i),
                kind: FindingKind::IdPackingDiff,
                construct: format!("db_field t{} c{} tup{}", field.table, field.column, field.tuple),
                bridge: Bridge::DbfieldRepack,
                detail: format!(
                    "db-field re-packs from the 948 layout to the {} layout (encoder-intrinsic)",
                    target.build
                ),
            });
            continue;
        }

        // Arity drift (tostring/db_find/...). These are opcodes the target DOES
        // have, but with a different arity in the donor's usage.
        if let Some((bridge, detail)) = arity_drift_bridge(op) {
            findings.push(Finding {
                script: script_id,
                instr: Some(i),
                kind: FindingKind::ArityDrift,
                construct: op.to_string(),
                bridge,
                detail: detail.to_string(),
            });
            continue;
        }

        if !target.has_op(op) {
            // 948-only opcode absent from the target book.
            let (bridge, detail) = missing_op_bridge(op);
            findings.push(Finding {
                script: script_id,
                instr: Some(i),
                kind: FindingKind::MissingOp,
                construct: op.to_string(),
                bridge,
                detail: detail.to_string(),
            });
        } else if !target.has_canonical_op(op) {
            // Present only via an alias (e.g. `enum` → `_enum`).
            let canonical = target.canonical_of(op).unwrap_or_default();
            findings.push(Finding {
                script: script_id,
                instr: Some(i),
                kind: FindingKind::OpRename,
                construct: op.to_string(),
                bridge: Bridge::EnumRename,
                detail: format!("non-canonical mnemonic; canonical target is `{canonical}`"),
            });
        }
    }
    findings
}

/// Capability-gap findings read straight off the target descriptor (plan §8) —
/// the modern-font and component-family gaps `port plan` reports even when no
/// single opcode triggers them.
#[must_use]
pub fn capability_findings(target: &BuildDescriptor) -> Vec<Finding> {
    let mut findings = Vec::new();
    let caps = &target.capabilities;
    if !caps.cc_list {
        findings.push(Finding {
            script: None,
            instr: None,
            kind: FindingKind::CapabilityGap,
            construct: "cc_list".to_string(),
            bridge: Bridge::ListToServerDriven,
            detail: format!(
                "build {} has no cc_list component family (type 16); the recipe list mounts \
                 empty / server-driven until the client gains it (descriptor.cc_list = true)",
                target.build
            ),
        });
    }
    if !caps.cc_radiogroup {
        findings.push(Finding {
            script: None,
            instr: None,
            kind: FindingKind::CapabilityGap,
            construct: "cc_radiogroup".to_string(),
            bridge: Bridge::ListToServerDriven,
            detail: format!(
                "build {} has no cc_radiogroup component family; the multi-focus selector is \
                 stubbed until the client gains it (descriptor.cc_radiogroup = true)",
                target.build
            ),
        });
    }
    if !caps.stylesheet {
        findings.push(Finding {
            script: None,
            instr: None,
            kind: FindingKind::CapabilityGap,
            construct: "stylesheet".to_string(),
            bridge: Bridge::UnmappedPop1Neutralise,
            detail: format!(
                "build {} has no stylesheet text-colour system (cc_setstylesheet + the skin/text \
                 db tables); the chain is stubbed to constant colours",
                target.build
            ),
        });
    }
    if !caps.modern_fonts {
        findings.push(Finding {
            script: None,
            instr: None,
            kind: FindingKind::CapabilityGap,
            construct: "modern_fonts".to_string(),
            bridge: Bridge::None,
            detail: format!(
                "build {} has no modern (fontmetrics2/ttf) font decoder; modern fonts must be \
                 pre-rasterized to bitmap with `font rasterize` (out of this layer's scope)",
                target.build
            ),
        });
    }
    findings
}

/// Run the representability dry-run over a decoded interface group (plan §4.2 /
/// §6): every composite-widget component the target build cannot represent
/// directly is classified [`FindingKind::MissingComponentKind`] with its
/// [`Bridge::ListToServerDriven`] lowering; the cc-model gap is reported once. A
/// target whose `cc_list` capability is set yields no component findings (the
/// widgets are directly representable — the client-engine seam, plan §8).
#[must_use]
pub fn represent_interface(
    ir: &crate::port::ir::interface::InterfaceIr,
    target: &BuildDescriptor,
) -> Vec<Finding> {
    use crate::port::ir::interface::Body;

    let mut findings = Vec::new();
    for (id, component) in ir.components.iter().enumerate() {
        if component.needs_downcode() && !target.capabilities.cc_list {
            let label = match &component.body {
                Body::Composite { text: Some(t), .. } if !t.text.is_empty() => {
                    format!("→ text (label \"{}\" survives)", t.text)
                }
                _ => "→ layer (server-driven hotspot)".to_string(),
            };
            findings.push(Finding {
                script: None,
                instr: Some(id),
                kind: FindingKind::MissingComponentKind,
                construct: format!("{:?} (type {})", component.kind, component.kind.type_id()),
                bridge: Bridge::ListToServerDriven,
                detail: format!(
                    "component {id} is a composite widget with no build-{} Component.decode body; \
                     lower::list_to_server_driven downcodes it {label}",
                    target.build
                ),
            });
        }
    }
    // The cc-model: the donor addresses subcomponents in a sparse id space. On a
    // target whose cc-model is dense this is a CcModelMismatch; post the client
    // change 910 is sparse, so report it as silently handled.
    if !target.capabilities.cc_model_sparse {
        findings.push(Finding {
            script: None,
            instr: None,
            kind: FindingKind::CcModelMismatch,
            construct: "cc_model".to_string(),
            bridge: Bridge::None,
            detail: format!(
                "build {} addresses subcomponents by DENSE ids; the donor's sparse cc-ids need a \
                 client change (descriptor.cc_model_sparse = true)",
                target.build
            ),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::ir::cs2::{Header, Insn};
    use std::path::PathBuf;

    fn target_910() -> BuildDescriptor {
        BuildDescriptor::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"), 910)
            .expect("load 910 descriptor")
    }

    fn ir(code: Vec<Insn>) -> Cs2Ir {
        Cs2Ir {
            name: None,
            header: Header::default(),
            code,
        }
    }

    #[test]
    fn flags_sub_as_missing_op_with_bridge() {
        let f = represent_script(&ir(vec![Insn::bare("sub")]), Some(1), &target_910());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FindingKind::MissingOp);
        assert_eq!(f[0].bridge, Bridge::SubToAdd);
    }

    #[test]
    fn flags_dropdown_opcode_as_list_bridge() {
        let f = represent_script(
            &ir(vec![Insn::bare("cc_setondropdownselect")]),
            Some(17790),
            &target_910(),
        );
        assert_eq!(f[0].kind, FindingKind::MissingOp);
        assert_eq!(f[0].bridge, Bridge::ListToServerDriven);
    }

    #[test]
    fn flags_enum_as_rename() {
        let f = represent_script(&ir(vec![Insn::bare("enum")]), None, &target_910());
        assert_eq!(f[0].kind, FindingKind::OpRename);
        assert_eq!(f[0].bridge, Bridge::EnumRename);
    }

    #[test]
    fn flags_tostring_as_arity_drift() {
        let f = represent_script(&ir(vec![Insn::bare("tostring")]), None, &target_910());
        assert_eq!(f[0].kind, FindingKind::ArityDrift);
    }

    #[test]
    fn capability_gaps_include_cc_list_and_fonts() {
        let f = capability_findings(&target_910());
        assert!(f.iter().any(|x| x.construct == "cc_list"));
        assert!(f.iter().any(|x| x.construct == "modern_fonts"));
    }

    #[test]
    fn flags_composite_widget_as_missing_component_kind() {
        use crate::port::ir::interface::{Body, Component, ComponentKind, InterfaceIr, TextPart};
        let ir = InterfaceIr {
            group: 1224,
            components: vec![
                // a primitive text (representable) ...
                Component {
                    version: 11,
                    kind: ComponentKind::Text,
                    name_bit: false,
                    header_tail: vec![],
                    body: Body::Raw(vec![]),
                    tail: vec![],
                },
                // ... and a labelled check (NOT representable on 910).
                Component {
                    version: 11,
                    kind: ComponentKind::Check,
                    name_bit: false,
                    header_tail: vec![],
                    body: Body::Composite {
                        text: Some(TextPart { text: "Show Locked".into(), ..TextPart::default() }),
                        raw_len: 58,
                    },
                    tail: vec![],
                },
            ],
        };
        let f = represent_interface(&ir, &target_910());
        // Only the check is flagged; 910 cc_model is sparse so no CcModelMismatch.
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FindingKind::MissingComponentKind);
        assert_eq!(f[0].bridge, Bridge::ListToServerDriven);
        assert_eq!(f[0].instr, Some(1));
        assert!(f[0].detail.contains("Show Locked"));
    }
}
