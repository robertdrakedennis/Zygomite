//! Named, opt-in IR→IR lowering passes (plan §6).
//!
//! Each pass is a typed transform over the [`Cs2Ir`] that bridges one 948→910
//! delta the target build cannot represent directly. They are declared once, as
//! book-level op-equivalences, and applied by name — the inversion of the old
//! blind text rewrites: a construct with no registered lowering fails loud at
//! encode time (plan §6) rather than crashing three frames into a render.
//!
//! The routine passes (plan §9 step 1):
//!  * [`sub_to_add`] — 910 has no `sub`; `a - N` (constant RHS) → `a + (-N)`
//!    zero-shift, `a - b` (variable RHS) → `a + (-1 * b)` (expands, renumbered);
//!  * [`tostring_drop_radix`] — 948's `tostring(v, radix)` → 910's `tostring(v)`;
//!    drop the redundant `push int:10` radix (shrinks, renumbered);
//!  * [`tostring_localised_long_neutralise`] — 948-only long→string localiser →
//!    drop int+long (stack-neutral `pop_int_discard; pop_long_discard`);
//!  * [`dbfind_drop_tuple`] — 948's `db_find(field,key,tuple)` → 910's
//!    `db_find(field,key)`; the dangling tuple-index push → a zero-shift branch;
//!  * [`enum_rename`] — bare `enum` → canonical `_enum`;
//!  * [`unmapped_pop1_neutralise`] — 948-only single-int-pop component/stylesheet
//!    opcodes → stack-neutral `pop_int_discard`;
//!  * [`dbfield_repack`] — db-field constants re-pack through the target packing
//!    (handled structurally by the IR's [`crate::port::ir::cs2::Operand::DbFieldConst`]
//!    at encode; this records the pass for `represent`).
//!
//! The structural pass [`proc_alloc`] lives in [`crate::port::encode::cs2::ProcAllocator`].

pub mod interface;
pub mod renumber;

use crate::cache_bail as bail;
use crate::error::Result;
use crate::port::ir::cs2::{Cs2Ir, Insn, Operand};

use renumber::Rebuilder;

/// 948-only single-int-pop component / stylesheet opcodes 910 has no equivalent
/// for. They set cosmetic component state (toggle / enabled / feedback /
/// stylesheet) absent from the 910 component model, so they neutralise to a
/// stack-neutral `pop_int_discard`. Mirrors `UNMAPPED_POP1_INT_OPS` in the Python.
pub const UNMAPPED_POP1_INT_OPS: &[&str] = &[
    "cc_setstylesheet",
    "cc_setenabled",
    "cc_setfeedbackmode",
    "cc_button_setcantoggle",
    "cc_button_settoggled",
    "cc_check_set",
];

/// `enum` → `_enum` (910 names the lookup `_enum`). Zero-shift, in place.
pub fn enum_rename(ir: &mut Cs2Ir) {
    for insn in &mut ir.code {
        if insn.op == "enum" {
            insn.op = "_enum".to_string();
        }
    }
}

/// 948-only single-int-pop opcodes → `pop_int_discard`. Zero-shift, in place.
pub fn unmapped_pop1_neutralise(ir: &mut Cs2Ir) {
    for insn in &mut ir.code {
        if UNMAPPED_POP1_INT_OPS.contains(&insn.op.as_str()) {
            insn.op = "pop_int_discard".to_string();
            insn.operand = Operand::None;
        }
    }
}

/// 948-only `tostring_localised_long` → drop int + long (stack-neutral). The donor
/// sequence is `push str:"~"; push long:V; push int:1; tostring_localised_long`;
/// neutralising it to `pop_int_discard; pop_long_discard` leaves the separator
/// string as the degraded result (net effect `… string → string`). Replaces ONE
/// instruction with TWO → renumbered. Mirrors the Python `tostring_localised_long`
/// branch in `apply_common_rewrites`.
pub fn tostring_localised_long_neutralise(ir: &mut Cs2Ir) {
    let mut rb = Rebuilder::new();
    for insn in &ir.code {
        rb.begin();
        if insn.op == "tostring_localised_long" {
            rb.push(Insn::bare("pop_int_discard"));
            rb.push(Insn::bare("pop_long_discard"));
        } else {
            rb.push(insn.clone());
        }
    }
    ir.code = rb.finish();
}

/// The fused "common rewrites" rebuild pass: applies `sub`→`add`, the 948-only
/// single-int-pop neutralisations, `tostring_localised_long`, and `tostring`
/// radix-drop in ONE rebuild over a single old→new index map — byte-identical to
/// `build-ritual-scripts.py::apply_common_rewrites`'s rebuild loop (lines 491–546).
///
/// Fusing them (rather than running the individual passes in sequence) guarantees
/// the renumbering matches the Python exactly even where two triggers are adjacent.
/// The in-place, length-preserving rewrites (`enum`, db-field repack, `db_find`
/// arity) are applied by the caller BEFORE this, mirroring the Python order.
pub fn common_rebuild(ir: &mut Cs2Ir) -> Result<()> {
    let mut rb = Rebuilder::new();
    for (i, insn) in ir.code.iter().enumerate() {
        rb.begin();
        if insn.op == "sub" {
            // RHS is the just-emitted instruction (the original predecessor).
            let const_rhs = ir.code.get(i.wrapping_sub(1)).and_then(|p| {
                if i >= 1 {
                    typed_int_const_value(p)
                } else {
                    None
                }
            });
            if let Some(n) = const_rhs {
                rb.replace_last(Insn {
                    op: "push_constant_string".to_string(),
                    operand: Operand::TypedIntConst(n.wrapping_neg()),
                });
            } else {
                rb.push(Insn {
                    op: "push_constant_string".to_string(),
                    operand: Operand::TypedIntConst(-1),
                });
                rb.push(Insn::bare("multiply"));
            }
            // Both lowerings finish by replacing `sub` with `add`.
            rb.push(Insn::bare("add"));
        } else if UNMAPPED_POP1_INT_OPS.contains(&insn.op.as_str()) {
            rb.push(Insn::bare("pop_int_discard"));
        } else if insn.op == "tostring_localised_long" {
            rb.push(Insn::bare("pop_int_discard"));
            rb.push(Insn::bare("pop_long_discard"));
        } else if insn.op == "tostring" {
            match rb.last() {
                Some(prev) if is_typed_int_const(prev, 10) => {
                    rb.drop_prev_remap_current();
                    rb.push(insn.clone());
                }
                other => {
                    bail!(
                        "tostring not preceded by a `push int:10` radix: {:?}",
                        other.map(|p| p.op.clone())
                    );
                }
            }
        } else {
            rb.push(insn.clone());
        }
    }
    ir.code = rb.finish();
    Ok(())
}

/// `db_find` arity: 948's `db_find` pops `(field, key, tuple)`; 910's pops only
/// `(field, key)`. The dangling tuple-index push (`push_constant_string int:0`)
/// immediately before a `db_find` becomes a zero-shift `branch <db_find_index>`
/// fall-through. Mirrors the Python db_find loop. Errors if a `db_find` is not so
/// preceded (an un-handled arity form).
pub fn dbfind_drop_tuple(ir: &mut Cs2Ir) -> Result<()> {
    let len = ir.code.len();
    for i in 0..len {
        if ir.code[i].op == "db_find" {
            let Some(prev) = i.checked_sub(1) else {
                bail!("db_find at instr 0 has no tuple-index push to drop");
            };
            if !is_typed_int_const(&ir.code[prev], 0) {
                bail!(
                    "db_find at instr {i} not preceded by a tuple-index `push int:0`: {:?}",
                    ir.code[prev].op
                );
            }
            // Zero-shift: rewrite the push to a fall-through branch to db_find.
            ir.code[prev] = Insn {
                op: "branch".to_string(),
                operand: Operand::Jump(i as i32),
            };
        }
    }
    Ok(())
}

/// `tostring` radix drop: 948's `tostring(value, radix)` → 910's `tostring(value)`
/// (assumes base 10). Every ritual call pushes `int:10` as the radix immediately
/// before; drop it. Removing one instruction shifts later targets → renumbered.
/// Mirrors the Python `tostring` branch. Errors if a `tostring` is not preceded by
/// the `int:10` radix push.
pub fn tostring_drop_radix(ir: &mut Cs2Ir) -> Result<()> {
    let mut rb = Rebuilder::new();
    for insn in &ir.code {
        rb.begin();
        if insn.op == "tostring" {
            match rb.last() {
                Some(prev) if is_typed_int_const(prev, 10) => {
                    rb.drop_prev_remap_current();
                    rb.push(insn.clone());
                }
                other => {
                    bail!(
                        "tostring not preceded by a `push int:10` radix: {:?}",
                        other.map(|p| p.op.clone())
                    );
                }
            }
        } else {
            rb.push(insn.clone());
        }
    }
    ir.code = rb.finish();
    Ok(())
}

/// `sub` → 910 (which has no `sub`). A constant RHS (`push int:N; sub`) becomes
/// `push int:-N; add` (zero-shift). A variable / computed RHS (`a - b`) expands to
/// `a + (-1 * b)` via `push int:-1; multiply; add` (+2 instructions → renumbered).
/// Mirrors the Python `sub` branch in `apply_common_rewrites`. Errors on a `sub`
/// form with no representable lowering.
pub fn sub_to_add(ir: &mut Cs2Ir) -> Result<()> {
    let mut rb = Rebuilder::new();
    for insn in &ir.code {
        rb.begin();
        if insn.op == "sub" {
            // Inspect the just-emitted instruction (the RHS operand source).
            let const_rhs = match rb.last() {
                Some(prev) => typed_int_const_value(prev),
                None => None,
            };
            if let Some(n) = const_rhs {
                // Constant RHS: negate the preceding push, sub → add (zero-shift).
                rb.replace_last(Insn {
                    op: "push_constant_string".to_string(),
                    operand: Operand::TypedIntConst(n.wrapping_neg()),
                });
            } else {
                // Variable / computed RHS: a - b → a + (-1 * b) (expands 1 → 3).
                rb.push(Insn {
                    op: "push_constant_string".to_string(),
                    operand: Operand::TypedIntConst(-1),
                });
                rb.push(Insn::bare("multiply"));
            }
            // Both lowerings finish by replacing `sub` with `add`.
            rb.push(Insn::bare("add"));
        } else {
            rb.push(insn.clone());
        }
    }
    ir.code = rb.finish();
    Ok(())
}

/// Whether `insn` is `push_constant_string` carrying the typed int constant `n`.
fn is_typed_int_const(insn: &Insn, n: i32) -> bool {
    typed_int_const_value(insn) == Some(n)
}

/// The typed-int value of a `push_constant_string int:N`, or `None`.
fn typed_int_const_value(insn: &Insn) -> Option<i32> {
    if insn.op != "push_constant_string" {
        return None;
    }
    match &insn.operand {
        Operand::TypedIntConst(v) => Some(*v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::ir::cs2::Header;

    fn ir(code: Vec<Insn>) -> Cs2Ir {
        Cs2Ir {
            name: None,
            header: Header::default(),
            code,
        }
    }

    fn int_const(n: i32) -> Insn {
        Insn {
            op: "push_constant_string".into(),
            operand: Operand::TypedIntConst(n),
        }
    }

    #[test]
    fn enum_renames_in_place() {
        let mut x = ir(vec![Insn::bare("enum")]);
        enum_rename(&mut x);
        assert_eq!(x.code[0].op, "_enum");
    }

    #[test]
    fn sub_constant_rhs_is_zero_shift() -> Result<()> {
        // push int:5; sub → push int:-5; add. Length unchanged; later branch holds.
        let mut x = ir(vec![
            int_const(5),
            Insn::bare("sub"),
            Insn {
                op: "branch".into(),
                operand: Operand::Jump(2),
            },
        ]);
        sub_to_add(&mut x)?;
        assert_eq!(x.code.len(), 3);
        assert_eq!(x.code[0].operand, Operand::TypedIntConst(-5));
        assert_eq!(x.code[1].op, "add");
        // Branch target 2 unchanged (zero-shift).
        assert_eq!(x.code[2].operand, Operand::Jump(2));
        Ok(())
    }

    #[test]
    fn sub_variable_rhs_expands_and_renumbers_branch() -> Result<()> {
        // [0] push_int_local; [1] push_int_local; [2] sub; [3] branch 2 → after
        // expansion sub becomes 3 instrs at indices 2,3,4 and branch (now at 5)
        // retargets to instr 2.
        let mut x = ir(vec![
            Insn {
                op: "push_int_local".into(),
                operand: Operand::LocalRef(0),
            },
            Insn {
                op: "push_int_local".into(),
                operand: Operand::LocalRef(1),
            },
            Insn::bare("sub"),
            Insn {
                op: "branch".into(),
                operand: Operand::Jump(2),
            },
        ]);
        sub_to_add(&mut x)?;
        // 2 locals + (push -1, multiply, add) + branch = 6 instrs.
        assert_eq!(x.code.len(), 6);
        assert_eq!(x.code[2].operand, Operand::TypedIntConst(-1));
        assert_eq!(x.code[3].op, "multiply");
        assert_eq!(x.code[4].op, "add");
        // branch is now instr 5; its old target 2 maps to new index 2 (the start
        // of the expanded sub).
        assert_eq!(x.code[5].operand, Operand::Jump(2));
        Ok(())
    }

    #[test]
    fn tostring_drops_radix_and_renumbers() -> Result<()> {
        // [0] push value; [1] push int:10; [2] tostring; [3] branch 2 → after drop,
        // tostring is instr 1, branch (instr 2) retargets to 1.
        let mut x = ir(vec![
            Insn {
                op: "push_int_local".into(),
                operand: Operand::LocalRef(0),
            },
            int_const(10),
            Insn::bare("tostring"),
            Insn {
                op: "branch".into(),
                operand: Operand::Jump(2),
            },
        ]);
        tostring_drop_radix(&mut x)?;
        assert_eq!(x.code.len(), 3);
        assert_eq!(x.code[1].op, "tostring");
        assert_eq!(x.code[2].operand, Operand::Jump(1));
        Ok(())
    }

    #[test]
    fn dbfind_drops_tuple_push_zero_shift() -> Result<()> {
        let mut x = ir(vec![
            int_const(60163),
            Insn {
                op: "push_int_local".into(),
                operand: Operand::LocalRef(0),
            },
            int_const(0),
            Insn::bare("db_find"),
        ]);
        dbfind_drop_tuple(&mut x)?;
        assert_eq!(x.code.len(), 4);
        assert_eq!(x.code[2].op, "branch");
        assert_eq!(x.code[2].operand, Operand::Jump(3));
        Ok(())
    }

    #[test]
    fn tostring_localised_long_expands_to_two_discards() {
        let mut x = ir(vec![Insn::bare("tostring_localised_long")]);
        tostring_localised_long_neutralise(&mut x);
        assert_eq!(x.code.len(), 2);
        assert_eq!(x.code[0].op, "pop_int_discard");
        assert_eq!(x.code[1].op, "pop_long_discard");
    }
}
