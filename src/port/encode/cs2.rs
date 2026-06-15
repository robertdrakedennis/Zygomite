//! The CS2 back-end (plan §4.1 / §5): lower typed IR to a target build's bytecode
//! with **intrinsic** validation — the encoder cannot emit an invalid script.
//!
//! Validation folded in here as encoder invariants (plan §9 step 3):
//!  * **opcode availability** — every IR op must resolve to a target opcode id
//!    (an absent op is an unrepresentable construct surfaced by `represent`);
//!  * **proc-id allocation by identity** — a [`ProcIdentity`] is resolved to a
//!    target raw id by [`ProcAllocator`]: reuse a 910-base id iff the identity
//!    matches, else a free id (plan §6 `proc_alloc`). Collisions become
//!    structurally impossible;
//!  * **db-field id packing** — db-field constants re-pack through the target
//!    descriptor (the `>>4` is `decode∘encode`, not a rewrite);
//!  * **stack balance / arg arity** — the net-stack simulation (the same sound,
//!    conservative check the lint carried) runs on the IR before bytes are
//!    produced;
//!  * **byte fidelity** — the emitted bytes re-decode to an identical script.
//!
//! The actual byte writer is the existing [`crate::script::encode_script`]; this
//! module wraps it with the IR→`CompiledScript` lowering and the invariants, so
//! `assemble-script` and `cs2 port` share one validating back-end.

use std::collections::HashMap;

use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::port::book::BuildDescriptor;
use crate::port::ir::cs2::{Cs2Ir, Operand, ProcIdentity};
use crate::script::{CompiledScript, decode_script, encode_script, script_to_asm};

/// Resolves a [`ProcIdentity`] to a target raw script id (plan §6 `proc_alloc`).
///
/// The structural rule: a call to a proc reuses its id directly iff that id means
/// the *same* proc in the target build; when the target id is a *different* proc
/// (a collision) the call must be retargeted to a free id where the donor proc
/// was spliced. This allocator holds the resolution map. For a port whose free-id
/// assignment is pinned by a committed oracle, the map is seeded from data (the
/// assignment the oracle defines); for a fresh port the same map is *computed*
/// from the collision set, allocating contiguous free ids.
#[derive(Clone, Debug, Default)]
pub struct ProcAllocator {
    /// `source raw id → target raw id`. An id absent from the map resolves to
    /// itself (the proc means the same thing in both builds, or it is a donor-new
    /// id spliced at its native id).
    remap: HashMap<i32, i32>,
}

impl ProcAllocator {
    /// An allocator that resolves every call to its source id (identity port).
    #[must_use]
    pub fn identity() -> Self {
        Self::default()
    }

    /// Seed the allocator with an explicit `source → target` remap (the oracle's
    /// pinned free-id assignment, or a precomputed collision remap).
    #[must_use]
    pub fn with_remap(remap: HashMap<i32, i32>) -> Self {
        Self { remap }
    }

    /// Record that calls to `source` retarget to `target`.
    pub fn insert(&mut self, source: i32, target: i32) {
        self.remap.insert(source, target);
    }

    /// Resolve a proc identity to its target raw id.
    #[must_use]
    pub fn resolve(&self, identity: &ProcIdentity) -> i32 {
        self.remap
            .get(&identity.source_id)
            .copied()
            .unwrap_or(identity.source_id)
    }

    /// The underlying remap, for diagnostics / `port plan`.
    #[must_use]
    pub fn remap(&self) -> &HashMap<i32, i32> {
        &self.remap
    }
}

/// Lower IR to a [`CompiledScript`] against the target descriptor + allocator,
/// re-packing db-field constants and resolving call ids. No bytes yet — this is
/// the form `script_to_asm` serializes and `encode_script` consumes.
pub fn lower_to_compiled(
    ir: &Cs2Ir,
    target: &BuildDescriptor,
    alloc: &ProcAllocator,
) -> Result<CompiledScript> {
    ir.to_compiled(
        &|field| target.encode_db_field(field),
        &|identity| Ok(alloc.resolve(identity)),
    )
}

/// Serialize IR to the reversible `@cs2` asm body (the IR's textual form). This is
/// the byte-exact diff anchor against committed `.asm.ts` listings.
pub fn ir_to_asm(
    ir: &Cs2Ir,
    target: &BuildDescriptor,
    alloc: &ProcAllocator,
) -> Result<String> {
    let compiled = lower_to_compiled(ir, target, alloc)?;
    Ok(script_to_asm(&compiled))
}

/// Errors the intrinsic validator raises before any bytes are produced.
#[derive(Clone, Debug)]
pub struct EncodeError {
    /// 0-based instruction index (or `usize::MAX` for a whole-script error).
    pub instr: usize,
    /// A stable kind tag.
    pub kind: &'static str,
    /// Human detail.
    pub detail: String,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.instr == usize::MAX {
            write!(f, "[{}] {}", self.kind, self.detail)
        } else {
            write!(f, "[{}] instr {}: {}", self.kind, self.instr, self.detail)
        }
    }
}

/// Run the intrinsic IR-level invariants (opcode availability, canonical-name
/// fidelity, and net stack balance). Returns every violation; the encoder refuses
/// to emit bytes when this is non-empty.
pub fn validate_ir(ir: &Cs2Ir, target: &BuildDescriptor) -> Vec<EncodeError> {
    let mut errors = Vec::new();

    // 1) Every op must resolve to a target opcode id, by its CANONICAL name (a
    //    non-canonical alias trips the byte-fidelity gate).
    for (i, insn) in ir.code.iter().enumerate() {
        if insn.op == "switch" {
            continue; // handled structurally by the encoder
        }
        if !target.has_op(&insn.op) {
            errors.push(EncodeError {
                instr: i,
                kind: "missing_op",
                detail: format!("target build {} has no opcode `{}`", target.build, insn.op),
            });
        } else if !target.has_canonical_op(&insn.op) {
            let canonical = target.canonical_of(&insn.op).unwrap_or_default();
            errors.push(EncodeError {
                instr: i,
                kind: "non_canonical_op",
                detail: format!(
                    "`{}` is not canonical in build {} (canonical `{canonical}`); rename before encode",
                    insn.op, target.build
                ),
            });
        }
    }

    // 2) Net stack balance (sound; only fires on fully-resolvable bodies). Lifts
    //    the lint's `net_stack_findings` to the IR.
    errors.extend(net_stack_errors(ir, target));

    errors
}

/// The IR net-stack-balance check (sound, conservative). Mirrors
/// `cs2/lint.rs::net_stack_findings` but reads the typed IR. Skips any body
/// containing a call or a variadic op (unresolvable arity) — never a false
/// positive — and flags the first underflow on a fully-resolvable body.
fn net_stack_errors(ir: &Cs2Ir, target: &BuildDescriptor) -> Vec<EncodeError> {
    for insn in &ir.code {
        if matches!(insn.operand, Operand::Call(_)) || is_variadic_op(&insn.op) {
            return Vec::new();
        }
        if insn.op != "push_constant_string"
            && !is_control_op(&insn.op)
            && target.stack_effect(&insn.op).is_none()
        {
            // An op with no static effect and no special-case: unresolvable.
            return Vec::new();
        }
    }
    let (mut di, mut dobj, mut dl) = (
        i64::from(ir.header.arg_int),
        i64::from(ir.header.arg_obj),
        i64::from(ir.header.arg_long),
    );
    let mut errors = Vec::new();
    for (idx, insn) in ir.code.iter().enumerate() {
        if is_control_op(&insn.op) {
            continue;
        }
        let eff = if insn.op == "push_constant_string" {
            // Operand-typed push: int / str(obj) / long.
            match &insn.operand {
                Operand::LongConst(_) => (0, 0, 0, 0, 0, 1),
                Operand::StrConst(_) => (0, 0, 0, 0, 1, 0),
                _ => (0, 0, 0, 1, 0, 0),
            }
        } else {
            let Some(d) = target.stack_effect(&insn.op) else {
                return Vec::new();
            };
            (
                d.int_pops,
                d.obj_pops,
                d.long_pops,
                d.int_pushes,
                d.obj_pushes,
                d.long_pushes,
            )
        };
        di -= eff.0;
        dobj -= eff.1;
        dl -= eff.2;
        if di < 0 || dobj < 0 || dl < 0 {
            errors.push(EncodeError {
                instr: idx,
                kind: "net_stack_underflow",
                detail: format!(
                    "`{}` pops more than available (depth int={di} obj={dobj} long={dl}); \
                     the IR's net stack effect is wrong — the live client would throw AIOOBE \
                     off the CS2 stack",
                    insn.op
                ),
            });
            return errors;
        }
        di += eff.3;
        dobj += eff.4;
        dl += eff.5;
    }
    errors
}

/// Variadic / callee-dependent opcodes whose stack effect cannot be read off the
/// static table. Mirrors `cs2/lint.rs::is_variadic_op`.
fn is_variadic_op(op: &str) -> bool {
    matches!(
        op,
        "join_string"
            | "switch"
            | "db_getfield"
            | "db_getfieldcount"
            | "db_find"
            | "db_findnext"
            | "db_getrowtable"
            | "cc_setonvartransmit"
            | "cc_setonstattransmit"
            | "cc_setoninvtransmit"
            | "if_setonvartransmit"
            | "if_setonstattransmit"
            | "if_setoninvtransmit"
            | "cc_setonop"
            | "if_setonop"
    )
}

/// Control-flow / pseudo opcodes with no `stack-effects.txt` entry but neutral in
/// the linear simulation. Mirrors `cs2/lint.rs::is_control_op`.
fn is_control_op(op: &str) -> bool {
    op == "return" || op.starts_with("branch") || op == "switch" || op == "label"
}

/// Encode IR to target bytecode, running the intrinsic invariants first and a
/// byte-fidelity round-trip after. Returns the bytes the overlay would splice.
pub fn encode_ir(
    ir: &Cs2Ir,
    target: &BuildDescriptor,
    alloc: &ProcAllocator,
) -> Result<Vec<u8>> {
    let errors = validate_ir(ir, target);
    if !errors.is_empty() {
        let detail = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "encode refused: {} intrinsic invariant violation(s): {detail}",
            errors.len()
        );
    }
    let compiled = lower_to_compiled(ir, target, alloc)?;
    let bytes = encode_script(&compiled, &target.opcodes, target.build)
        .context("encoding IR to target bytecode")?;

    // Byte fidelity: the emitted bytes must re-decode to an identical script
    // (the same guard `verify_assembled_script` applies). A self-consistent
    // encoder bug still surfaces here.
    let decoded = decode_script(&bytes, &target.opcodes, target.build)
        .context("re-decoding emitted bytecode for fidelity check")?;
    if script_to_asm(&decoded) != script_to_asm(&compiled) {
        bail!("encode fidelity check failed: re-decoded bytecode does not match the IR");
    }
    Ok(bytes)
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

    fn ir(code: Vec<Insn>, header: Header) -> Cs2Ir {
        Cs2Ir {
            name: None,
            header,
            code,
        }
    }

    #[test]
    fn rejects_op_absent_from_target() {
        let code = vec![
            Insn::bare("sub"),
            Insn {
                op: "return".into(),
                operand: Operand::None,
            },
        ];
        let errors = validate_ir(&ir(code, Header::default()), &target_910());
        assert!(errors.iter().any(|e| e.kind == "missing_op"));
    }

    #[test]
    fn rejects_non_canonical_alias() {
        // `enum` resolves via alias to `_enum`; must be flagged non-canonical.
        let code = vec![Insn::bare("enum")];
        let errors = validate_ir(&ir(code, Header::default()), &target_910());
        assert!(errors.iter().any(|e| e.kind == "non_canonical_op"));
    }

    #[test]
    fn flags_net_stack_underflow() {
        // push ONE int, then `add` (pops two) → underflow.
        let code = vec![
            Insn {
                op: "push_constant_string".into(),
                operand: Operand::TypedIntConst(1),
            },
            Insn::bare("add"),
            Insn::bare("return"),
        ];
        let errors = validate_ir(&ir(code, Header::default()), &target_910());
        assert!(errors.iter().any(|e| e.kind == "net_stack_underflow"));
    }

    #[test]
    fn proc_allocator_resolves_remap_else_identity() {
        let mut alloc = ProcAllocator::identity();
        alloc.insert(5360, 20803);
        assert_eq!(alloc.resolve(&ProcIdentity::from_source_id(5360)), 20803);
        assert_eq!(alloc.resolve(&ProcIdentity::from_source_id(17503)), 17503);
    }

    #[test]
    fn encodes_balanced_body_and_round_trips() -> Result<()> {
        // A minimal valid body: push int local, return.
        let code = vec![
            Insn {
                op: "push_int_local".into(),
                operand: Operand::LocalRef(0),
            },
            Insn::bare("return"),
        ];
        let header = Header {
            arg_int: 1,
            ..Header::default()
        };
        let bytes = encode_ir(&ir(code, header), &target_910(), &ProcAllocator::identity())?;
        assert!(!bytes.is_empty());
        Ok(())
    }
}
