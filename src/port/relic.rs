//! Relic-system (interface 691) port driver (plan §9 step 4): re-port the donor
//! (948) Relic Powers CS2 closure through the port layer — the byte-exact oracle
//! against the committed `server/cache-patches/relic-system-948/scripts/*.asm.ts`
//! (a PROTECTED oracle; reproduced, never edited).
//!
//! Relic is the canonical donor-port shape (it is the source-of-truth the
//! `cs2/lint.rs` rules were derived from): the routine lowerings (`sub`→`add`,
//! `enum`→`_enum`, db-field `>>4`, `db_find` arity) + a fixed proc relocate
//! (948's `script7924` cc-icon-draw → free id 24924) + three per-script
//! signature-drift rewrites (14611/14620/14587) + one fully-authored currency
//! listing (14964). All on the same [`crate::port::lower`] passes + validating
//! [`crate::port::encode`] back-end the ritual driver uses.

use std::collections::HashMap;

use crate::cache_bail as bail;
use crate::error::Result;
use crate::port::book::BuildDescriptor;
use crate::port::encode::ProcAllocator;
use crate::port::ir::cs2::{Cs2Ir, DbField, Insn, Operand};
use crate::port::lower;
use crate::port::ritual::{PortedScript, ScriptSource};
use crate::script::script_to_asm;

/// 948-format db-field constants used by the relic splice set (tables
/// 66/90/92/94/287). Same set as `cs2/lint.rs::DB_FIELDS_948` and the Python's
/// `DB_FIELDS_948`. A `push_constant_string int:N` equal to one re-packs `>>4`.
const RELIC_DB_FIELDS: &[i32] = &[
    270_352, 270_400, 368_640, 376_896, 376_912, 385_024, 385_040, 385_056, 385_072, 385_088,
    385_104, 385_120, 385_136, 385_152, 385_168, 1_175_552,
];

/// The relic splice set: donor 948 clientscript group ids re-emitted at their
/// native id, EXCEPT 7924 which relocates to 24924 (the proc relocate). Mirrors
/// the Python `SPLICE` (14964 is the authored listing below; 14603/14610 are
/// skill-29-owned and excluded). The list is the SPLICE keys sans 7924/14964.
const RELIC_SPLICE_IDS: &[i32] = &[
    14459, 14584, 14587, 14596, 14606, 14611, 14612, 14614, 14619, 14620, 14621, 14622, 14623,
    14624, 14629, 14630, 14844, 14847, 14848, 14849, 14850, 14852, 14853, 14854, 14855, 14856,
    14857, 14858, 14859, 14860, 14861, 14862, 14863, 14864, 14865, 14866, 14867, 14869, 14870,
    14879, 17567, 18965, 18966, 18976, 19769, 19771, 19772, 19773, 19774, 19819, 19820, 19821,
    19822,
];

/// The relocated shared cc-icon-draw: donor `script7924` → free 910 id 24924.
const RELOCATE_7924_FROM: i32 = 7924;
const RELOCATE_7924_TO: i32 = 24924;

/// The fully-authored currency_total (group 14964) — chronotes = invtotal(889
/// currency pouch, 49430) + invtotal(93 backpack, 49430). Verbatim from the
/// Python `CURRENCY_TOTAL_LISTING` (the committed listing).
const CURRENCY_TOTAL_LISTING: &str = "\
// relic-system-948: 948 currency_total (group 14964) REWRITTEN — chronote count
// = invtotal(889 currency pouch, 49430) + invtotal(93 backpack, 49430). See
// build-relic-scripts.py header for why the dbtable-66 path is not portable.
// @cs2 locals int=1 obj=0 long=0
// @cs2 args int=1 obj=0 long=0
// @cs2 push_constant_string int:889
// @cs2 push_constant_string int:49430
// @cs2 inv_total 0
// @cs2 push_constant_string int:93
// @cs2 push_constant_string int:49430
// @cs2 inv_total 0
// @cs2 add 0
// @cs2 return 0
";

/// Re-port the relic CS2 closure through the port layer. `source` decodes a 948
/// clientscript group id to its `CompiledScript`.
pub fn port_relic_scripts(
    source: &ScriptSource<'_>,
    descriptor_948: &BuildDescriptor,
    descriptor_910: &BuildDescriptor,
) -> Result<Vec<PortedScript>> {
    let alloc = relic_allocator();
    let field_set: std::collections::HashSet<i32> = RELIC_DB_FIELDS.iter().copied().collect();
    let mut out = Vec::new();

    // The native-id splice set + 7924→24924.
    let mut ported_ids: Vec<i32> = RELIC_SPLICE_IDS.to_vec();
    ported_ids.push(RELOCATE_7924_FROM); // ported, but written at 24924.

    for sid in ported_ids {
        let mut ir = decode_relic_ir(source, sid, descriptor_948, &field_set)?;
        apply_relic_rewrites(sid, &mut ir, descriptor_910)?;
        let body = render_asm(&ir, descriptor_910, &alloc)?;
        let out_id = if sid == RELOCATE_7924_FROM {
            RELOCATE_7924_TO
        } else {
            sid
        };
        out.push(PortedScript {
            out_id,
            text: format!("{}\n{}", relic_header(sid), body),
        });
    }

    // The authored currency listing.
    out.push(PortedScript {
        out_id: 14964,
        text: CURRENCY_TOTAL_LISTING.to_string(),
    });

    Ok(out)
}

/// The proc allocator: 7924 → 24924 (the only relic relocate).
fn relic_allocator() -> ProcAllocator {
    let mut remap = HashMap::new();
    remap.insert(RELOCATE_7924_FROM, RELOCATE_7924_TO);
    ProcAllocator::with_remap(remap)
}

/// Decode a 948 relic clientscript group to [`Cs2Ir`], lifting the relic db-field
/// constants through the 948 packing (recognising only `RELIC_DB_FIELDS`).
fn decode_relic_ir(
    source: &ScriptSource<'_>,
    group_id: i32,
    descriptor_948: &BuildDescriptor,
    field_set: &std::collections::HashSet<i32>,
) -> Result<Cs2Ir> {
    let compiled = source(group_id)?;
    let db_decode = |v: i32| -> Option<DbField> {
        if field_set.contains(&v) {
            Some(descriptor_948.decode_db_field(v))
        } else {
            None
        }
    };
    Ok(Cs2Ir::from_compiled(&compiled, &db_decode))
}

/// The relic 948→910 rewrites: the routine lowerings (in the Python order) + the
/// three per-script signature-drift rewrites (zero-shift line replacements).
fn apply_relic_rewrites(sid: i32, ir: &mut Cs2Ir, _target: &BuildDescriptor) -> Result<()> {
    // enum → _enum (in place).
    lower::enum_rename(ir);
    // db-field repack is structural at encode (the IR carries DbFieldConst).
    // db_find arity (in place, zero-shift).
    lower::dbfind_drop_tuple(ir)?;
    // sub → add. Relic subtractions are ALL constant-RHS; the fused rebuild
    // handles them zero-shift (no variable-RHS, no tostring in this set).
    lower::common_rebuild(ir)?;

    // Per-script signature-drift rewrites (zero-shift; mirror the Python).
    match sid {
        14611 => {
            // 948 script3092 (chronote-discount flag) != 910 script3092; push 0.
            replace_unique_call(
                ir,
                3092,
                Insn {
                    op: "push_constant_string".into(),
                    operand: Operand::TypedIntConst(0),
                },
            )?;
        }
        14620 => {
            // 948 script1858 (fresh-start check) != 910; push int:6→0, gosub→bitcount.
            replace_unique_int_const(ir, 6, 0)?;
            replace_unique_call(ir, 1858, Insn::bare("bitcount"))?;
        }
        14587 => {
            // 948 script13022 != 910; gosub→bitcount (stack-shape stand-in).
            replace_unique_call(ir, 13022, Insn::bare("bitcount"))?;
        }
        _ => {}
    }
    Ok(())
}

/// Replace the unique `gosub_with_params <source_id>` with `replacement` (zero-shift).
fn replace_unique_call(ir: &mut Cs2Ir, source_id: i32, replacement: Insn) -> Result<()> {
    let hits: Vec<usize> = ir
        .code
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.op == "gosub_with_params"
                && matches!(&l.operand, Operand::Call(p) if p.source_id == source_id)
        })
        .map(|(i, _)| i)
        .collect();
    if hits.len() != 1 {
        bail!(
            "relic: expected exactly one `gosub_with_params {source_id}`, found {}",
            hits.len()
        );
    }
    ir.code[hits[0]] = replacement;
    Ok(())
}

/// Replace the unique `push_constant_string int:<from>` with `int:<to>` (zero-shift).
fn replace_unique_int_const(ir: &mut Cs2Ir, from: i32, to: i32) -> Result<()> {
    let hits: Vec<usize> = ir
        .code
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.op == "push_constant_string" && matches!(l.operand, Operand::TypedIntConst(v) if v == from)
        })
        .map(|(i, _)| i)
        .collect();
    if hits.len() != 1 {
        bail!(
            "relic: expected exactly one `push_constant_string int:{from}`, found {}",
            hits.len()
        );
    }
    ir.code[hits[0]].operand = Operand::TypedIntConst(to);
    Ok(())
}

/// Render IR → reversible asm body through the target descriptor + allocator.
fn render_asm(ir: &Cs2Ir, target: &BuildDescriptor, alloc: &ProcAllocator) -> Result<String> {
    let compiled = ir.to_compiled(
        &|field| target.encode_db_field(field),
        &|identity| Ok(alloc.resolve(identity)),
    )?;
    Ok(script_to_asm(&compiled))
}

/// The relic header comment (verbatim from the Python `emit` description, with the
/// `7924 relocated to 24924` special case).
fn relic_header(sid: i32) -> String {
    let label = if sid == RELOCATE_7924_FROM {
        "7924 relocated to 24924".to_string()
    } else {
        sid.to_string()
    };
    format!(
        "// relic-system-948: 948 donor script {label} re-emitted from its reversible transpile \
         (948-only id; shared-callee signature drift handled by zero-shift rewrites — see \
         build-relic-scripts.py)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn relic_db_field_set_repacks_to_shift() {
        let d948 = BuildDescriptor::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"),
            948,
        )
        .unwrap();
        let d910 = BuildDescriptor::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"),
            910,
        )
        .unwrap();
        for &v in RELIC_DB_FIELDS {
            let f = d948.decode_db_field(v);
            assert_eq!(d910.encode_db_field(&f), v >> 4);
        }
    }
}
