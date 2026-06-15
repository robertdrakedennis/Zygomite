// Surgical composite-opcode neutralisations for the ritual port (17785/17523/
// 17804). Included into `ritual.rs` (shares its imports + module scope). Each
// reproduces the corresponding branch of `build-ritual-scripts.py::
// apply_surgical_rewrites` (and `_fix_17785_focus_id_overreturn`) as a typed
// IR transform.

/// Whether `insn` is `push_constant_string` carrying the typed int `n`.
fn is_int_const(insn: &Insn, n: i32) -> bool {
    insn.op == "push_constant_string" && matches!(insn.operand, Operand::TypedIntConst(v) if v == n)
}

/// Whether `insn` is `push_constant_string str:"<s>"`.
fn is_str_const(insn: &Insn, s: &str) -> bool {
    insn.op == "push_constant_string" && matches!(&insn.operand, Operand::StrConst(v) if v == s)
}

/// Whether `insn` is `push_int_local <slot>`.
fn is_push_int_local(insn: &Insn, slot: i32) -> bool {
    insn.op == "push_int_local" && matches!(insn.operand, Operand::LocalRef(v) if v == slot)
}

/// Whether `insn` is a `gosub_with_params <id>` (call to source id `id`).
fn is_call(insn: &Insn, id: i32) -> bool {
    insn.op == "gosub_with_params" && matches!(&insn.operand, Operand::Call(p) if p.source_id == id)
}

/// 17785 — drop the per-row `cc_setonbuttonclick` (948-only) and the focus-id
/// over-return fix. Mirrors the `sid == 17785` branch + `_fix_17785_focus_id_overreturn`.
fn surgical_17785(ir: &mut Cs2Ir) -> Result<()> {
    // (1) Neutralise the 5-instruction cc_setonbuttonclick callback block
    //     zero-shift to a stack-neutral no-op (push 0 / pop / push 0 / pop /
    //     branch-to-next), preserving the instruction count.
    let hits: Vec<usize> = ir
        .code
        .iter()
        .enumerate()
        .filter(|(_, l)| l.op == "cc_setonbuttonclick")
        .map(|(i, _)| i)
        .collect();
    if hits.len() != 1 {
        bail!(
            "script 17785: expected exactly one cc_setonbuttonclick, found {}",
            hits.len()
        );
    }
    let k = hits[0];
    if k < 4 {
        bail!("script 17785: cc_setonbuttonclick too early to carry its callback block");
    }
    // Expected shape (k-4 .. k):
    //   push int:17792; push_int_local 11; push_int_local 8; push str:"ii";
    //   cc_setonbuttonclick
    let shape_ok = is_int_const(&ir.code[k - 4], 17792)
        && is_push_int_local(&ir.code[k - 3], 11)
        && is_push_int_local(&ir.code[k - 2], 8)
        && is_str_const(&ir.code[k - 1], "ii")
        && ir.code[k].op == "cc_setonbuttonclick";
    if !shape_ok {
        bail!("script 17785: cc_setonbuttonclick block shape changed");
    }
    ir.code[k - 4] = Insn {
        op: "push_constant_string".into(),
        operand: Operand::TypedIntConst(0),
    };
    ir.code[k - 3] = Insn::bare("pop_int_discard");
    ir.code[k - 2] = Insn {
        op: "push_constant_string".into(),
        operand: Operand::TypedIntConst(0),
    };
    ir.code[k - 1] = Insn::bare("pop_int_discard");
    ir.code[k] = Insn {
        op: "branch".into(),
        operand: Operand::Jump((k + 1) as i32),
    };

    // (2) focus-id over-return fix.
    fix_17785_focus_id_overreturn(ir)
}

/// The db_getfield multi-tuple OVER-RETURN fix for 17785 (the list re-layout
/// `cc_find` crash). Mirrors `_fix_17785_focus_id_overreturn`.
fn fix_17785_focus_id_overreturn(ir: &mut Cs2Ir) -> Result<()> {
    const SCRATCH: i32 = 15; // fresh int local (header bumped 15→16 in the driver).
    let gosubs: Vec<usize> = ir
        .code
        .iter()
        .enumerate()
        .filter(|(_, l)| is_call(l, 17503))
        .map(|(i, _)| i)
        .collect();
    if gosubs.len() != 3 {
        bail!(
            "script 17785: expected 3 gosub 17503 sites, found {}",
            gosubs.len()
        );
    }
    let cc_find_sites: Vec<usize> = gosubs
        .iter()
        .copied()
        .filter(|&i| i + 1 < ir.code.len() && ir.code[i + 1].op == "cc_find")
        .collect();
    let pop_sites: Vec<usize> = gosubs
        .iter()
        .copied()
        .filter(|&i| i + 1 < ir.code.len() && ir.code[i + 1].op == "pop_int_local")
        .collect();
    if cc_find_sites.len() != 1 || pop_sites.len() != 2 {
        bail!(
            "script 17785: unexpected 17503 site shapes (cc_find={cc_find_sites:?}, pop={pop_sites:?})"
        );
    }
    let relayout_site = cc_find_sites[0];
    let build_site = *pop_sites.iter().min().expect("two pop sites");
    let consume = build_site + 1; // the build site's own pop_int_local, re-emitted.

    let mut rb = Rebuilder::new();
    for (i, insn) in ir.code.iter().enumerate() {
        rb.begin();
        if i == consume {
            // Skipped (re-emitted next to its gosub at build_site). The Python
            // `continue`s, recording old_to_new[i] = current out len.
            continue;
        }
        rb.push(insn.clone());
        if i == relayout_site {
            rb.push(Insn {
                op: "pop_int_local".into(),
                operand: Operand::LocalRef(SCRATCH),
            }); // focusIdx (top)
            rb.push(Insn::bare("pop_int_discard")); // qty
            rb.push(Insn::bare("pop_int_discard")); // item
            rb.push(Insn {
                op: "push_int_local".into(),
                operand: Operand::LocalRef(SCRATCH),
            }); // re-push focusIdx
        } else if i == build_site {
            rb.push(ir.code[build_site + 1].clone()); // original pop_int_local
            rb.push(Insn::bare("pop_int_discard")); // qty
            rb.push(Insn::bare("pop_int_discard")); // item
        }
    }
    ir.code = rb.finish();
    Ok(())
}

/// 17523 — show-locked bypass (list every ritual when varbit 53836 == 1). Mirrors
/// the `sid == 17523` branch. Inserts a 5-instruction bypass after the `recipe ==
/// -1` guard (instr 0..5), renumbering every later target by +5, then resolves the
/// bypass's own synthetic `branch_not` to the post-renumber gate position.
fn surgical_17523(ir: &mut Cs2Ir) -> Result<()> {
    const GUARD_END: usize = 6;
    const BYPASS: usize = 5;
    // Validate the recipe==-1 guard shape (instr 0..5).
    if ir.code.len() < GUARD_END {
        bail!("script 17523: too short to carry the recipe==-1 guard");
    }
    let guard_ok = is_push_int_local(&ir.code[0], 0)
        && is_int_const(&ir.code[1], -1)
        && ir.code[2].op == "branch_equals"
        && matches!(ir.code[2].operand, Operand::Jump(4))
        && ir.code[3].op == "branch"
        && matches!(ir.code[3].operand, Operand::Jump(6))
        && is_int_const(&ir.code[4], 0)
        && ir.code[5].op == "return";
    if !guard_ok {
        bail!("script 17523: unexpected recipe==-1 guard shape");
    }

    // Rebuild with the bypass spliced at GUARD_END. We track the old→new map
    // ourselves (mirroring the Python) so the synthetic branch can be resolved
    // after renumber. The bypass's `branch_not` target is a placeholder
    // (i32::MIN) excluded from renumbering, resolved at the end.
    const PLACEHOLDER: i32 = i32::MIN;
    let mut rebuilt: Vec<Insn> = Vec::new();
    let mut old_to_new: Vec<usize> = Vec::new();
    let mut gate_pos: Option<usize> = None;
    for (i, insn) in ir.code.iter().enumerate() {
        old_to_new.push(rebuilt.len());
        if i == GUARD_END {
            gate_pos = Some(rebuilt.len() + BYPASS);
            rebuilt.push(Insn {
                op: "push_varbit".into(),
                operand: Operand::VarBitRef(crate::script::VarBitRef {
                    id: 53836,
                    transmog: false,
                }),
            });
            rebuilt.push(Insn {
                op: "push_constant_string".into(),
                operand: Operand::TypedIntConst(1),
            });
            rebuilt.push(Insn {
                op: "branch_not".into(),
                operand: Operand::Jump(PLACEHOLDER),
            });
            rebuilt.push(Insn {
                op: "push_constant_string".into(),
                operand: Operand::TypedIntConst(1),
            });
            rebuilt.push(Insn::bare("return"));
        }
        rebuilt.push(insn.clone());
    }
    old_to_new.push(rebuilt.len()); // one-past-end sentinel.
    let gate_pos = gate_pos.context("script 17523: guard end not reached")? as i32;

    // Renumber every REAL target (skip our placeholder).
    renumber_skipping_placeholder(&mut rebuilt, &old_to_new, PLACEHOLDER);

    // Resolve the bypass placeholder to the (now-stable) gate position.
    for insn in &mut rebuilt {
        if insn.op == "branch_not" && matches!(insn.operand, Operand::Jump(PLACEHOLDER)) {
            insn.operand = Operand::Jump(gate_pos);
        }
    }
    ir.code = rebuilt;
    Ok(())
}

/// Renumber all `Jump`/switch targets through `old_to_new`, except a target equal
/// to `placeholder` (left for post-renumber resolution).
fn renumber_skipping_placeholder(code: &mut [Insn], old_to_new: &[usize], placeholder: i32) {
    // Temporarily neutralise placeholder jumps so the generic remap skips them by
    // range, then restore. Simpler: do the remap manually here.
    let map = |t: i32| -> i32 {
        if t == placeholder {
            t
        } else if t >= 0 && (t as usize) < old_to_new.len() {
            old_to_new[t as usize] as i32
        } else {
            t
        }
    };
    for insn in code.iter_mut() {
        // Renumber all jump families incl. long_branch_* (see renumber.rs::remap_targets
        // — the layer is the sole producer and emits the correct renumbering).
        match &mut insn.operand {
            Operand::Jump(t) => *t = map(*t),
            Operand::Switch(cases) => {
                for case in cases.iter_mut() {
                    case.target = map(case.target);
                }
            }
            _ => {}
        }
    }
    let _ = renumber_with_map; // keep the import used in the non-placeholder path.
}

/// 17804 — stub the multi-focus radiogroup branch. Mirrors the `sid == 17804`
/// branch: retarget the entry guard to the branch exit (unreachable body) and make
/// the 3 now-dead unmapped ops a valid 910 op. Zero-shift.
fn surgical_17804(ir: &mut Cs2Ir) -> Result<()> {
    let add: Vec<usize> = ir
        .code
        .iter()
        .enumerate()
        .filter(|(_, l)| l.op == "cc_radiogroup_addoption")
        .map(|(i, _)| i)
        .collect();
    let sel: Vec<usize> = ir
        .code
        .iter()
        .enumerate()
        .filter(|(_, l)| l.op == "cc_radiogroup_setoptionselected")
        .map(|(i, _)| i)
        .collect();
    if add.len() != 1 || sel.len() != 2 {
        bail!(
            "script 17804: radiogroup op count changed (add={add:?}, sel={sel:?})"
        );
    }
    let k = add[0];
    if k < 4 {
        bail!("script 17804: radiogroup op too early");
    }
    let body_start = k - 2;
    let skip = k - 3;
    let guard = k - 4;
    // The skip is `branch <exit>`.
    let Operand::Jump(exit_target) = ir.code[skip].operand else {
        bail!("script 17804: expected the radiogroup skip `branch <exit>` at instr {skip}");
    };
    if ir.code[skip].op != "branch" {
        bail!("script 17804: expected `branch` at instr {skip}");
    }
    // The guard is `branch_equals <body_start>`.
    if !(ir.code[guard].op == "branch_equals"
        && matches!(ir.code[guard].operand, Operand::Jump(t) if t == body_start as i32))
    {
        bail!("script 17804: expected radiogroup entry guard `branch_equals {body_start}` at instr {guard}");
    }
    // Retarget the guard to the exit (skip the dead body).
    ir.code[guard].operand = Operand::Jump(exit_target);
    // Neutralise the dead radiogroup ops to a valid 910 op.
    for &j in add.iter().chain(sel.iter()) {
        ir.code[j] = Insn::bare("pop_int_discard");
    }
    Ok(())
}
