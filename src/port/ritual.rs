//! Milestone-1 driver (plan §9 steps 1–2 / §11): re-port the City of Um "Ritual
//! selection" (interface 1224) CS2 scripts through the port layer — the BYTE-EXACT
//! oracle against the 99 committed `server/cache-patches/ritual-pedestal-948/
//! scripts/*.asm.ts`.
//!
//! This reproduces `build-ritual-scripts.py` as a typed pipeline over the
//! [`crate::port`] layer: decode each donor (948) script to [`Cs2Ir`], apply the
//! routine 948→910 lowerings ([`crate::port::lower`]) + the three surgical
//! composite-opcode neutralisations (17785 / 17523 / 17804), resolve proc ids
//! through the pinned [`ProcAllocator`] (the oracle's free-id assignment, declared
//! here as data — plan §6 `proc_alloc`), and emit the reversible `@cs2` asm with
//! the ritual header. The output must equal the committed listings byte-for-byte.
//!
//! The closure (which scripts are spliced, which are skill-29-owned, the free-id
//! remap, the stylesheet/colour/runeday stubs, the type-4 override) is the same
//! data the Python carried — the layer makes the *transforms* typed and the
//! validation intrinsic; the *port-specific facts* (which db constants are field
//! ids, the pinned free-id assignment, the stub bodies) remain declared data, as
//! the plan prescribes.

use std::collections::HashMap;

use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::port::book::BuildDescriptor;
use crate::port::encode::ProcAllocator;
use crate::port::ir::cs2::{Cs2Ir, DbField, Header, Insn, Operand};
use crate::port::lower;
use crate::port::lower::renumber::{Rebuilder, renumber_with_map};
use crate::script::script_to_asm;

// ── Port config (mirrors build-ritual-scripts.py) ────────────────────────────

/// 948-format db-field constants the ritual splice touches (dbtables 234/235/236).
/// A `push_constant_string int:N` equal to one of these is the donor
/// `table<<12|column<<4|tuple` packing → re-packed to the 910 `table<<8|column`
/// layout (the `>>4`). Exactly `RITUAL_DB_FIELDS` in the Python.
const RITUAL_DB_FIELDS: &[i32] = &[
    958_480, 958_545, 958_592, // table 234 (cols 1/5/8)
    962_560, 962_576, 962_592, 962_608, 962_609, 962_610, 962_611, 962_640, // table 235
    962_656, 962_672, 962_688, 962_704, 962_720, 962_736, 962_768, 962_784, // table 235
    962_800, 962_832, // table 235
    966_674, 966_704, 966_736, 966_768, 966_784, 966_800, 966_816, // table 236
];

/// The 12 closure scripts owned by `SKILL_29_GUIDE_CORE_SCRIPT_PATCHES` (a cache
/// group has one patch). Excluded from the ritual splice — the skills-29 copies
/// are the single source. Mirrors `SKILL_29_SHARED_IDS`.
const SKILL_29_SHARED_IDS: &[i32] = &[
    17495, 17498, 17503, 17504, 17505, 17507, 17510, 17511, 17512, 17513, 17522, 17527,
];

/// The full missing-from-910 transitive closure of interface 1224 (the SPLICE set,
/// before excluding the skill-29-shared ids), in the Python's order. Each is
/// re-emitted from its reversible transpile at its NATIVE id.
const SPLICE_IDS_RAW: &[i32] = &[
    14441, 14488, 16387, 17495, 17496, 17497, 17498, 17499, 17500, 17501, 17502, 17503, 17504,
    17505, 17506, 17507, 17508, 17509, 17510, 17511, 17512, 17513, 17514, 17515, 17516, 17517,
    17518, 17522, 17523, 17524, 17525, 17527, 17528, 17529, 17530, 17531, 17532, 17533, 17534,
    17535, 17536, 17537, 17538, 17539, 17540, 17541, 17542, 17634, 17784, 17785, 17786, 17787,
    17788, 17789, 17793, 17794, 17795, 17796, 17797, 17798, 17799, 17800, 17801, 17802, 17803,
    17804, 17806, 17807, 17808, 17809, 17810, 17811, 17812, 17816, 17828, 17829, 17830, 17831,
    18409, 18411, 18412, 18417, 18418, 18420, 18423, 18424, 18437, 18520, 18985, 19066, 19657,
    16851, 16861,
];

/// Proc-id COLLISION remap: `donor id → free 910 id`. The render helpers (and the
/// three same-arity detail collisions) are 948-only procs colliding with unrelated
/// 910-base ids; spliced at free ids absent from both books. Mirrors
/// `RENDER_HELPER_REMAP` exactly (the pinned free-id assignment the oracle defines).
const RENDER_HELPER_REMAP: &[(i32, i32)] = &[
    (1212, 20817),
    (1526, 20800),
    (2995, 20801),
    (3686, 20802),
    (5360, 20803),
    (7775, 20804),
    (7852, 20805),
    (7853, 20806),
    (7872, 20807),
    (7886, 20808),
    (7889, 20809),
    (9731, 20810),
    (10643, 20811),
    (10644, 20812),
    (10684, 20813),
    (15709, 20814),
    (16748, 20815),
    (17063, 20816),
    (18419, 20818),
    (1296, 20819),
    (11205, 20821),
    (13033, 20822),
];

/// Donor stylesheet helpers spliced as signature-matching no-op stubs (the
/// 948-only stylesheet db table NPEs on 910). Keyed by OLD donor id. Mirrors
/// `STYLESHEET_STUB_OLD_IDS`.
const STYLESHEET_STUB_OLD_IDS: &[i32] = &[5360, 10684, 16748];

/// 948-only Runeday/seasonal helper → `return 0` stub. Mirrors
/// `RUNEDAY_STUB_OLD_IDS`.
const RUNEDAY_STUB_OLD_IDS: &[i32] = &[11205];

/// The donor `text_body_default` colour (dbrow 344/2100): 0xE3D7CF.
const RITUAL_TEXT_COLOUR: i32 = 14_931_919;
/// The rasterized body font present in the 1224 overlay (26 is absent).
const RITUAL_TEXT_FONT: i32 = 58;

// ── The driver ───────────────────────────────────────────────────────────────

/// A re-ported script ready to write: the export id it lands at, and the full
/// `.asm.ts` text (header comment + reversible asm body).
pub struct PortedScript {
    /// The id the listing is written under (`script<id>.asm.ts`).
    pub out_id: i32,
    /// The full file text.
    pub text: String,
}

/// Decodes a 948 clientscript group id to its [`Cs2Ir`]-ready [`CompiledScript`].
/// Both the CLI (cache-backed) and the oracle test supply one; the driver never
/// touches the filesystem directly, so it is fast (one cache load) and testable.
pub type ScriptSource<'a> = dyn Fn(i32) -> Result<crate::script::CompiledScript> + 'a;

/// Re-port the whole ritual closure through the port layer, producing the set of
/// `.asm.ts` listings. `source` decodes a 948 clientscript group id to its
/// `CompiledScript` (the donor side).
pub fn port_ritual_scripts(
    source: &ScriptSource<'_>,
    descriptor_948: &BuildDescriptor,
    descriptor_910: &BuildDescriptor,
) -> Result<Vec<PortedScript>> {
    let mut out = Vec::new();
    let alloc = ritual_allocator();
    let field_set: std::collections::HashSet<i32> = RITUAL_DB_FIELDS.iter().copied().collect();

    // The native-id splice set, skill-29-shared excluded.
    let splice_ids: Vec<i32> = SPLICE_IDS_RAW
        .iter()
        .copied()
        .filter(|id| !SKILL_29_SHARED_IDS.contains(id))
        .collect();

    for sid in splice_ids {
        let mut ir = decode_ritual_ir(source, sid, descriptor_948, &field_set)?;
        if sid == 17785 {
            // _fix_17785 needs one extra int local (the SCRATCH slot).
            if ir.header.local_int == 15 {
                ir.header.local_int = 16;
            }
        }
        apply_common_rewrites(sid, &mut ir, descriptor_910)?;
        apply_surgical_rewrites(sid, &mut ir)?;
        let body = render_asm(&ir, descriptor_910, &alloc)?;
        out.push(PortedScript {
            out_id: sid,
            text: format!("{}\n{}", faithful_header(sid), body),
        });
    }

    // The colliding render-helper closure, emitted at free ids.
    for &(old_id, new_id) in RENDER_HELPER_REMAP {
        let headers_ir = decode_ritual_ir(source, old_id, descriptor_948, &field_set)?;
        if STYLESHEET_STUB_OLD_IDS.contains(&old_id) {
            let body = build_stylesheet_stub(&headers_ir);
            out.push(PortedScript {
                out_id: new_id,
                text: format!(
                    "{}\n{}",
                    stylesheet_stub_header(old_id, new_id),
                    asm_from_header_and_lines(&headers_ir.header, &body)
                ),
            });
            continue;
        }
        if let Some(body) = colour_stub_listing(old_id) {
            out.push(PortedScript {
                out_id: new_id,
                text: format!(
                    "{}\n{}",
                    colour_helper_header(old_id, new_id),
                    asm_from_header_and_lines(&headers_ir.header, &body)
                ),
            });
            continue;
        }
        if RUNEDAY_STUB_OLD_IDS.contains(&old_id) {
            let body = vec![
                "// @cs2 push_constant_string int:0".to_string(),
                "// @cs2 return 0".to_string(),
            ];
            out.push(PortedScript {
                out_id: new_id,
                text: format!(
                    "{}\n{}",
                    runeday_stub_header(old_id, new_id),
                    asm_from_header_and_lines(&headers_ir.header, &body)
                ),
            });
            continue;
        }
        // Faithful helper: routine rewrites + the cc_create type override + remap.
        let mut ir = headers_ir;
        apply_common_rewrites(old_id, &mut ir, descriptor_910)?;
        apply_cc_create_type_override(old_id, &mut ir)?;
        let body = render_asm(&ir, descriptor_910, &alloc)?;
        out.push(PortedScript {
            out_id: new_id,
            text: format!("{}\n{}", helper_header(old_id, new_id), body),
        });
    }

    // The dropdown onload stub (17790).
    out.push(PortedScript {
        out_id: 17790,
        text: format!(
            "{}\n{}",
            stub_header(17790),
            "// @cs2 locals int=1 obj=0 long=0\n\
             // @cs2 args int=1 obj=0 long=0\n\
             // @cs2 return 0\n"
        ),
    });

    Ok(out)
}

/// A [`ScriptSource`] backed by a flat 948 cache: decode the single-file
/// clientscript group at `group_id` with the 948 opcode book. Every ritual id is
/// a single-file group keyed by its id (file 0). The index is decoded once and
/// borrowed by the returned closure.
pub fn cache_source<'a>(
    cache: &'a crate::cache::FlatCache,
    index: &'a crate::js5::ArchiveIndex,
    book_948: &'a crate::script::OpcodeBook,
) -> impl Fn(i32) -> Result<crate::script::CompiledScript> + 'a {
    flat_cache_source(cache, index, book_948, 948)
}

/// A [`ScriptSource`] backed by a flat cache at an explicit `build` (948 donor or
/// 910 base). The single-file group's bytes are decoded with `book` at `build`.
pub fn flat_cache_source<'a>(
    cache: &'a crate::cache::FlatCache,
    index: &'a crate::js5::ArchiveIndex,
    book: &'a crate::script::OpcodeBook,
    build: u32,
) -> impl Fn(i32) -> Result<crate::script::CompiledScript> + 'a {
    move |group_id: i32| {
        let gid = u32::try_from(group_id)
            .with_context(|| format!("negative clientscript group id {group_id}"))?;
        let files =
            cache.group_files_with_index(index, crate::constants::ARCHIVE_CLIENTSCRIPTS, gid)?;
        let (_, bytes) = files
            .into_iter()
            .min_by_key(|(file, _)| *file)
            .with_context(|| format!("clientscript group {group_id} is empty"))?;
        crate::script::decode_script(&bytes, book, build)
            .with_context(|| format!("decode clientscript group {group_id} (build {build})"))
    }
}

/// The pinned proc allocator: the oracle's free-id assignment (`RENDER_HELPER_REMAP`).
fn ritual_allocator() -> ProcAllocator {
    let mut remap = HashMap::new();
    for &(old, new) in RENDER_HELPER_REMAP {
        remap.insert(old, new);
    }
    ProcAllocator::with_remap(remap)
}

/// Decode one 948 clientscript group to [`Cs2Ir`], lifting the ritual db-field
/// constants through the 948 packing (recognising only the `RITUAL_DB_FIELDS`).
fn decode_ritual_ir(
    source: &ScriptSource<'_>,
    group_id: i32,
    descriptor_948: &BuildDescriptor,
    field_set: &std::collections::HashSet<i32>,
) -> Result<Cs2Ir> {
    let compiled = source(group_id)
        .with_context(|| format!("decode donor (948) clientscript group {group_id}"))?;
    let db_decode = |v: i32| -> Option<DbField> {
        if field_set.contains(&v) {
            Some(descriptor_948.decode_db_field(v))
        } else {
            None
        }
    };
    Ok(Cs2Ir::from_compiled(&compiled, &db_decode))
}

/// The routine 948→910 rewrites every faithful listing gets, in the Python order:
/// in-place length-preserving (enum, db-field repack, db_find arity) first, then
/// the fused `sub`/unmapped/tostring rebuild.
fn apply_common_rewrites(_sid: i32, ir: &mut Cs2Ir, _target: &BuildDescriptor) -> Result<()> {
    // enum → _enum (in place).
    lower::enum_rename(ir);
    // db-field repack happens structurally at encode (the IR carries DbFieldConst).
    // db_find arity (in place, zero-shift).
    lower::dbfind_drop_tuple(ir)?;
    // Fused sub/unmapped/tostring rebuild (renumbered).
    lower::common_rebuild(ir)?;
    Ok(())
}

/// Render the IR to the reversible `@cs2` asm body (header + instruction lines),
/// resolving db-field packing + call ids through the target descriptor/allocator.
/// This is the exact `script_to_asm` codepath, so the lines match byte-for-byte.
fn render_asm(ir: &Cs2Ir, target: &BuildDescriptor, alloc: &ProcAllocator) -> Result<String> {
    let compiled = ir.to_compiled(&|field| target.encode_db_field(field), &|identity| {
        Ok(alloc.resolve(identity))
    })?;
    Ok(script_to_asm(&compiled))
}

/// Build an asm body from a header and pre-formatted `// @cs2` lines (for stubs).
fn asm_from_header_and_lines(header: &Header, lines: &[String]) -> String {
    let mut out = format!(
        "// @cs2 locals int={} obj={} long={}\n// @cs2 args int={} obj={} long={}\n",
        header.local_int,
        header.local_obj,
        header.local_long,
        header.arg_int,
        header.arg_obj,
        header.arg_long,
    );
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

// ── Header comment strings (verbatim from the Python) ────────────────────────

fn faithful_header(sid: i32) -> String {
    format!(
        "// ritual-pedestal-948: 948 donor script {sid} re-emitted from its reversible transpile \
         for the \"Ritual selection\" interface (1224) recipe list/detail (948-only id; routine \
         948->910 rewrites + composite-opcode stubs — see build-ritual-scripts.py / \
         plans/ritual-selection-full/README.md)."
    )
}

fn stub_header(sid: i32) -> String {
    format!(
        "// ritual-pedestal-948: 948 donor script {sid} spliced as a no-op mount stub for the \
         \"Ritual selection\" interface (1224) — the sort/filter DROPDOWN opcode family is \
         948-only (910 has none); native dropdown is Phase B. See build-ritual-scripts.py / \
         plans/ritual-selection-full/README.md."
    )
}

fn helper_header(old_id: i32, new_id: i32) -> String {
    format!(
        "// ritual-pedestal-948: 948 donor render helper script {old_id} re-emitted at FREE 910 id \
         {new_id} (proc-id COLLISION remap — donor {old_id} and 910-base {old_id} are different \
         procs with different arg counts; see build-ritual-scripts.py RENDER_HELPER_REMAP). routine \
         948->910 rewrites applied; internal gosubs remapped."
    )
}

fn stylesheet_stub_header(old_id: i32, new_id: i32) -> String {
    format!(
        "// ritual-pedestal-948: 948 donor STYLESHEET helper script {old_id} spliced as a \
         signature-matching no-op STUB at FREE 910 id {new_id}. The stylesheet system \
         (cc_setstylesheet opcode + db table 132 / field 540672) is 948-ONLY — on 910 the table is \
         absent (db_getfield NPEs 'columnTypes is null'). Components keep their cache-default \
         bounds/fonts. See build-ritual-scripts.py STYLESHEET_STUB_OLD_IDS."
    )
}

fn colour_helper_header(old_id: i32, new_id: i32) -> String {
    format!(
        "// ritual-pedestal-948: 948 donor stylesheet TEXT helper script {old_id} spliced at FREE \
         910 id {new_id} as the donor's CONSTANT text defaults (font {RITUAL_TEXT_FONT} + colour \
         {RITUAL_TEXT_COLOUR}=0x{RITUAL_TEXT_COLOUR:06X}, dbrow 344/2100 `text_body_default`), \
         replacing the no-op stub that left ritual rows/detail DARK-on-dark. Drops only the 948-only \
         db-table read + live re-skin. See build-ritual-scripts.py COLOUR_STUB_LISTINGS."
    )
}

fn runeday_stub_header(old_id: i32, new_id: i32) -> String {
    format!(
        "// ritual-pedestal-948: 948 donor Runeday/seasonal-bonus helper script {old_id} spliced as \
         a `return 0` STUB at FREE 910 id {new_id} (its 910-base namesake is an unrelated TIMER proc \
         — a SAME-ARITY collision that mis-ran and crashed 17812). The donor body reads a 948-only db \
         table (77, Runeday events; absent from 910) and only adds a cosmetic +20% soul-rate to the \
         Output panel. See build-ritual-scripts.py RUNEDAY_STUB_OLD_IDS."
    )
}

// ── Surgical rewrites + stubs (filled in next) ───────────────────────────────

/// The three composite-opcode blockers, neutralised zero-shift (17785/17523/17804).
fn apply_surgical_rewrites(sid: i32, ir: &mut Cs2Ir) -> Result<()> {
    match sid {
        17785 => surgical_17785(ir),
        17523 => surgical_17523(ir),
        17804 => surgical_17804(ir),
        _ => Ok(()),
    }
}

/// The cc_create type override for the list-row button helper (7852: type 10→4).
fn apply_cc_create_type_override(old_id: i32, ir: &mut Cs2Ir) -> Result<()> {
    if old_id != 7852 {
        return Ok(());
    }
    // Find the `push_constant_string int:10` two instructions before a `cc_create`.
    let mut swapped = 0;
    for i in 0..ir.code.len() {
        if ir.code[i].op == "cc_create"
            && i >= 2
            && matches!(ir.code[i - 2].operand, Operand::TypedIntConst(10))
            && ir.code[i - 2].op == "push_constant_string"
        {
            ir.code[i - 2].operand = Operand::TypedIntConst(4);
            swapped += 1;
        }
    }
    if swapped != 1 {
        bail!("helper 7852: expected exactly one cc_create type 10 to override, found {swapped}");
    }
    Ok(())
}

/// The stylesheet no-op stub body: returns the donor's int-return arity as the
/// "absent" sentinel (-1 then 0s).
fn build_stylesheet_stub(ir: &Cs2Ir) -> Vec<String> {
    let n = return_int_count(ir);
    let mut body = Vec::new();
    for k in 0..n {
        body.push(format!(
            "// @cs2 push_constant_string int:{}",
            if k == 0 { -1 } else { 0 }
        ));
    }
    body.push("// @cs2 return 0".to_string());
    body
}

/// The colour stub listings (10643/10644) — the donor's constant text defaults.
fn colour_stub_listing(old_id: i32) -> Option<Vec<String>> {
    match old_id {
        10644 => Some(vec![
            format!("// @cs2 push_constant_string int:{RITUAL_TEXT_FONT}"),
            "// @cs2 cc_settextfont 0".to_string(),
            "// @cs2 push_constant_string int:1".to_string(),
            "// @cs2 cc_settextshadow 0".to_string(),
            "// @cs2 push_int_local 1".to_string(),
            "// @cs2 push_constant_string int:1".to_string(),
            "// @cs2 branch_equals 8".to_string(),
            "// @cs2 branch 10".to_string(),
            format!("// @cs2 push_constant_string int:{RITUAL_TEXT_COLOUR}"),
            "// @cs2 cc_setcolour 0".to_string(),
            "// @cs2 return 0".to_string(),
        ]),
        10643 => Some(vec![
            format!("// @cs2 push_constant_string int:{RITUAL_TEXT_FONT}"),
            "// @cs2 cc_settextfont 0".to_string(),
            format!("// @cs2 push_constant_string int:{RITUAL_TEXT_COLOUR}"),
            "// @cs2 cc_setcolour 0".to_string(),
            "// @cs2 return 0".to_string(),
        ]),
        _ => None,
    }
}

/// How many ints the donor body leaves on the stack at its first `return` (the
/// int-push run immediately preceding it). Mirrors `_return_int_count`.
fn return_int_count(ir: &Cs2Ir) -> usize {
    let Some(first_ret) = ir.code.iter().position(|i| i.op == "return") else {
        return 0;
    };
    let mut count = 0;
    let mut j = first_ret as isize - 1;
    while j >= 0 {
        let insn = &ir.code[j as usize];
        if insn.op == "push_constant_string"
            && matches!(
                insn.operand,
                Operand::TypedIntConst(_) | Operand::DbFieldConst(_)
            )
        {
            count += 1;
            j -= 1;
        } else {
            break;
        }
    }
    count
}

// Surgical-edit bodies (17785/17523/17804) are implemented in `surgical.rs`-style
// functions below.
include!("ritual_surgical.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ritual_db_field_set_repacks_to_shift() {
        let d948 = BuildDescriptor::load(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"),
            948,
        )
        .unwrap();
        let d910 = BuildDescriptor::load(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"),
            910,
        )
        .unwrap();
        for &v in RITUAL_DB_FIELDS {
            let f = d948.decode_db_field(v);
            assert_eq!(d910.encode_db_field(&f), v >> 4);
        }
    }
}
