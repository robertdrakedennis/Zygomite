//! Material-storage (interface 660) port driver (plan §9 step 4): retire
//! `build-material-storage-scripts.py`, gated on byte-exact reproduction of the
//! committed `server/cache-patches/material-storage-948/scripts/*.asm.ts`.
//!
//! This porter has BOTH shapes the layer must cover:
//!  * **donor splices** (14683/14710/14907/14909/14911) — five 948-only scripts,
//!    decoded from the 948 donor and lowered (only the `sub`→`add` routine
//!    rewrite applies). The pure decode→lower→encode port path.
//!  * **base augmentation** (9239) — 910's shared grid-slot op builder gains the
//!    948 switch cases for the interface-660 grids; the 948 locked-slot + op-set
//!    case is translated into 910's frame (locals shifted −1, branch targets
//!    rebased, the background-layer arg constant-folded). The base script is
//!    decoded to IR, then AUTHORED IR (the translated block + two switch cases) is
//!    appended and the whole is re-encoded with the validating back-end — the
//!    encoder makes the authored block correct-by-construction (stack-balanced,
//!    valid opcodes), which the old text-append could not guarantee.

use crate::cache_bail as bail;
use crate::error::Result;
use crate::port::book::BuildDescriptor;
use crate::port::encode::ProcAllocator;
use crate::port::ir::cs2::{Cs2Ir, DbField, Insn, Operand};
use crate::port::lower;
use crate::port::ritual::{PortedScript, ScriptSource};
use crate::script::{SwitchCase, script_to_asm};

// Interface 660 component uids (the 948 packed ids).
const STORAGE_GRID: i32 = 43_253_786; // com26 — container contents
const BACKPACK_GRID: i32 = 43_253_779; // com19 — backpack mirror
const STORAGE_GRID_BACKGROUND: i32 = 43_253_785; // com25 — locked-slot cc host

/// The five 948-only donor splices: group id → (nothing extra; sub-only). Mirrors
/// the Python `DONOR_SPLICES`.
const DONOR_SPLICE_IDS: &[i32] = &[14683, 14710, 14907, 14909, 14911];

/// 948 script 9239 instructions 313..419 — the material-storage component case —
/// captured verbatim from the 948 reversible transpile (the Python `BLOCK_948`).
/// Branch targets are 948 instruction indexes; locals are 948's frame.
/// [`translate_block`] rebases both into 910's frame.
const BLOCK_948: &str = "\
313 push_int_local 1
314 push_constant_string int:43253786
315 branch_equals 317
316 branch 337
317 push_int_local 6
318 gosub_with_params 14683
319 branch_greater_than_or_equals 321
320 branch 337
321 push_int_local 0
322 push_int_local 6
323 cc_find 1
324 push_constant_string int:1
325 branch_equals 327
326 branch 336
327 push_int_local 1
328 push_int_local 7
329 push_int_local 6
330 cc_getx 1
331 cc_gety 1
332 push_constant_string int:26605
333 push_constant_string str:\"Info\"
334 push_constant_string str:\"Requires a higher material storage capacity unlock from the Archaeology Guild.\"
335 gosub_with_params 14710
336 branch 419
337 push_int_local 11
338 push_constant_string int:5
339 branch_greater_than 341
340 branch 370
341 push_int_local 1
342 push_int_local 7
343 push_int_local 2
344 push_int_local 6
345 push_int_local 9
346 push_int_local 10
347 push_int_local 5
348 push_string_local 0
349 push_constant_string str:\"-1\"
350 join_string 2
351 push_string_local 0
352 push_constant_string str:\"-5\"
353 join_string 2
354 push_string_local 0
355 push_constant_string str:\"-10\"
356 join_string 2
357 push_string_local 0
358 push_constant_string str:\"-All\"
359 join_string 2
360 push_string_local 0
361 push_constant_string str:\"-X\"
362 join_string 2
363 push_constant_string str:\"\"
364 push_constant_string str:\"\"
365 push_constant_string str:\"\"
366 push_constant_string str:\"\"
367 push_constant_string str:\"Examine\"
368 gosub_with_params 12092
369 branch 419
370 push_int_local 11
371 push_constant_string int:1
372 branch_greater_than 374
373 branch 401
374 push_int_local 1
375 push_int_local 7
376 push_int_local 2
377 push_int_local 6
378 push_int_local 9
379 push_int_local 10
380 push_int_local 5
381 push_string_local 0
382 push_constant_string str:\"-1\"
383 join_string 2
384 push_string_local 0
385 push_constant_string str:\"-5\"
386 join_string 2
387 push_constant_string str:\"\"
388 push_string_local 0
389 push_constant_string str:\"-All\"
390 join_string 2
391 push_string_local 0
392 push_constant_string str:\"-X\"
393 join_string 2
394 push_constant_string str:\"\"
395 push_constant_string str:\"\"
396 push_constant_string str:\"\"
397 push_constant_string str:\"\"
398 push_constant_string str:\"Examine\"
399 gosub_with_params 12092
400 branch 419
401 push_int_local 1
402 push_int_local 7
403 push_int_local 2
404 push_int_local 6
405 push_int_local 9
406 push_int_local 10
407 push_int_local 5
408 push_string_local 0
409 push_constant_string str:\"\"
410 push_constant_string str:\"\"
411 push_constant_string str:\"\"
412 push_constant_string str:\"\"
413 push_constant_string str:\"\"
414 push_constant_string str:\"\"
415 push_constant_string str:\"\"
416 push_constant_string str:\"\"
417 push_constant_string str:\"Examine\"
418 gosub_with_params 12092
419 branch 670";

const BLOCK_948_START: i32 = 313;
const BLOCK_948_EPILOGUE: i32 = 670;

/// Re-port the material-storage scripts. `donor_source` decodes 948 groups;
/// `base_source` decodes 910-base groups (for the 9239 augmentation).
pub fn port_material_storage_scripts(
    donor_source: &ScriptSource<'_>,
    base_source: &ScriptSource<'_>,
    _descriptor_948: &BuildDescriptor,
    descriptor_910: &BuildDescriptor,
) -> Result<Vec<PortedScript>> {
    let alloc = ProcAllocator::identity();
    let mut out = Vec::new();

    // 1) The donor splices (sub-only routine rewrite).
    for &sid in DONOR_SPLICE_IDS {
        // The donor splices touch no ritual/relic db-fields; recognise none.
        let no_db = |_v: i32| -> Option<DbField> { None };
        let compiled = donor_source(sid)?;
        let mut ir = Cs2Ir::from_compiled(&compiled, &no_db);
        // Only `sub`→`add` (constant RHS) applies; the fused rebuild handles it.
        lower::common_rebuild(&mut ir)?;
        let body = render_asm(&ir, descriptor_910, &alloc)?;
        out.push(PortedScript {
            out_id: sid,
            text: format!("{}\n{}", donor_header(sid), body),
        });
    }

    // 2) The 9239 base augmentation.
    out.push(port_9239(base_source, descriptor_910, &alloc)?);

    Ok(out)
}

/// Augment the 910-base script 9239 with the material-storage grid cases + block.
fn port_9239(
    base_source: &ScriptSource<'_>,
    descriptor_910: &BuildDescriptor,
    alloc: &ProcAllocator,
) -> Result<PortedScript> {
    let no_db = |_v: i32| -> Option<DbField> { None };
    let base = base_source(9239)?;
    let mut ir = Cs2Ir::from_compiled(&base, &no_db);
    let base_len = ir.code.len();
    if base_len != 305 {
        bail!("unexpected 910 script9239 instruction count {base_len} (expected 305)");
    }
    // The shared per-case epilogue: `branch 299` (slot++ → loop head) at instr 271.
    let epilogue = 271;
    if !(ir.code[epilogue].op == "branch"
        && matches!(ir.code[epilogue].operand, Operand::Jump(299)))
    {
        bail!("instr {epilogue} is not the case epilogue (expected `branch 299`)");
    }
    // Find the switch carrying `case 43384837` and append the two grid cases →
    // the appended block start (base_len).
    let switch_idx = ir
        .code
        .iter()
        .position(|i| {
            i.op == "switch"
                && matches!(&i.operand, Operand::Switch(cases) if cases.iter().any(|c| c.value == 43_384_837))
        })
        .ok_or_else(|| crate::error::CacheError::message("no switch with case 43384837 in 9239"))?;
    if let Operand::Switch(cases) = &mut ir.code[switch_idx].operand {
        cases.push(SwitchCase {
            value: STORAGE_GRID,
            target: base_len as i32,
        });
        cases.push(SwitchCase {
            value: BACKPACK_GRID,
            target: base_len as i32,
        });
    }
    // Append the translated block.
    let block = translate_block(base_len as i32, epilogue as i32)?;
    ir.code.extend(block);

    let body = render_asm(&ir, descriptor_910, alloc)?;
    Ok(PortedScript {
        out_id: 9239,
        text: format!("{}\n{}", header_9239(base_len), body),
    })
}

/// Translate `BLOCK_948` into 910's frame: rebase branch targets (the epilogue
/// `670` → `epilogue_910`; others by `base + (t - BLOCK_948_START)`), shift int
/// locals by −1 (948's 9239 grew a leading background-layer arg), and fold local 0
/// into the background-layer constant. Mirrors the Python `translate_block`.
fn translate_block(base: i32, epilogue_910: i32) -> Result<Vec<Insn>> {
    let mut out = Vec::new();
    let mut expected = 0;
    for raw in BLOCK_948.lines() {
        expected += 1;
        let (_idx_str, rest) = raw
            .split_once(' ')
            .ok_or_else(|| crate::error::CacheError::message("malformed block line"))?;
        let (op, operand_text) = match rest.split_once(' ') {
            Some((op, rest)) => (op, rest),
            None => (rest, ""),
        };
        let insn = if is_branch_op(op) {
            let target: i32 = operand_text
                .rsplit(' ')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| crate::error::CacheError::message("bad branch target in block"))?;
            let rebased = if target == BLOCK_948_EPILOGUE {
                epilogue_910
            } else {
                base + (target - BLOCK_948_START)
            };
            Insn {
                op: op.to_string(),
                operand: Operand::Jump(rebased),
            }
        } else if op == "push_int_local" {
            let local: i32 = operand_text
                .parse()
                .map_err(|_| crate::error::CacheError::message("bad local index in block"))?;
            if local == 0 {
                // 948's leading background-layer arg → the constant 660:25.
                Insn {
                    op: "push_constant_string".to_string(),
                    operand: Operand::TypedIntConst(STORAGE_GRID_BACKGROUND),
                }
            } else {
                Insn {
                    op: "push_int_local".to_string(),
                    operand: Operand::LocalRef(local - 1),
                }
            }
        } else {
            parse_block_insn(op, operand_text)?
        };
        out.push(insn);
    }
    if out.len() != expected {
        bail!("translated block has {} instructions, expected {expected}", out.len());
    }
    Ok(out)
}

/// Parse a non-branch, non-push_int_local block instruction into IR (the authored
/// block uses a small, fixed opcode vocabulary).
fn parse_block_insn(op: &str, operand_text: &str) -> Result<Insn> {
    let operand = match op {
        "push_constant_string" => {
            if let Some(rest) = operand_text.strip_prefix("int:") {
                Operand::TypedIntConst(
                    rest.parse()
                        .map_err(|_| crate::error::CacheError::message("bad int const in block"))?,
                )
            } else if let Some(rest) = operand_text.strip_prefix("str:") {
                Operand::StrConst(unquote(rest))
            } else {
                bail!("unsupported push_constant_string operand in block: {operand_text}");
            }
        }
        "push_string_local" => Operand::LocalRef(
            operand_text
                .parse()
                .map_err(|_| crate::error::CacheError::message("bad string-local index in block"))?,
        ),
        "gosub_with_params" => Operand::Call(crate::port::ir::cs2::ProcIdentity::from_source_id(
            operand_text
                .parse()
                .map_err(|_| crate::error::CacheError::message("bad gosub id in block"))?,
        )),
        "join_string" => Operand::Count(
            operand_text
                .parse()
                .map_err(|_| crate::error::CacheError::message("bad join_string count in block"))?,
        ),
        // cc_find / cc_getx / cc_gety carry a 1-byte operand in this block.
        _ => {
            if operand_text.is_empty() {
                Operand::None
            } else if let Ok(v) = operand_text.parse::<i32>() {
                Operand::Byte(u8::try_from(v).map_err(|_| {
                    crate::error::CacheError::message("byte operand out of range in block")
                })?)
            } else {
                bail!("unsupported block instruction `{op} {operand_text}`");
            }
        }
    };
    Ok(Insn {
        op: op.to_string(),
        operand,
    })
}

/// Strip surrounding quotes from a `str:"…"` operand and unescape the minimal
/// set the block uses (`\"` only; the block has no `\n`/`\\`).
fn unquote(s: &str) -> String {
    let trimmed = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    trimmed.replace("\\\"", "\"")
}

fn is_branch_op(op: &str) -> bool {
    matches!(
        op,
        "branch"
            | "branch_equals"
            | "branch_not"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals"
    )
}

fn render_asm(ir: &Cs2Ir, target: &BuildDescriptor, alloc: &ProcAllocator) -> Result<String> {
    let compiled = ir.to_compiled(
        &|field| target.encode_db_field(field),
        &|identity| Ok(alloc.resolve(identity)),
    )?;
    Ok(script_to_asm(&compiled))
}

fn donor_header(sid: i32) -> String {
    format!(
        "// material-storage-948: 948 donor script {sid} re-emitted from its reversible transpile \
         (948-only group; all referenced ids are 910-native or imported by this patch family). \
         Generated by build-material-storage-scripts.py."
    )
}

fn header_9239(base: usize) -> String {
    format!(
        "// material-storage-948: switch cases for the interface 660 grids (43253786 storage / \
         43253779 backpack -> appended block at instr {base}; zero-shift). 948's locked-slot + \
         op-set case translated to 910's frame (locals shifted by -1; background layer constant \
         43253785). Generated by build-material-storage-scripts.py."
    )
}
