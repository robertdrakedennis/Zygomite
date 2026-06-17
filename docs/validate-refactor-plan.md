# `validate.rs` decomposition — architecture refactor plan

**Goal:** split the 3,234-LOC flat `src/validate.rs` into `src/validate/mod.rs` (the validator core)
+ `stack_effect.rs` (the giant per-opcode stack-effect table) + `tests.rs` (the inline test module).
**Strictly behavior-preserving** — pure code movement, no validation-logic changes.

## Why (evidence, measured 2026-06-15)
`src/validate.rs` = **3,234 LOC**, and the map shows two oversized, separable lumps:
- **`stack_effect_for` — ~826 LOC** (lines ~449–1274): one enormous `match` computing each CS2
  opcode's stack effect. Plus `varp_stack_effect_for` (~30) and `gosub_stack_effect` (~53). This is
  the gnarliest, most self-contained part of the engine.
- **The inline test module — ~1,768 LOC** (lines ~1466–3234): `test_data_dir`, `build_ctx`, and ~14
  large `#[cfg(test)]` test fns. **More than half the file is tests.**
- The actual validator core is only ~640 LOC: the types (`ValidationError`, `ValidationReport`,
  `TypedStacks`, `StackEffect`, `Cs2Validator`, `BatchReport`), the entry methods (`validate`,
  `validate_compiled`), the small passes (`pass_structural`, `pass_stack`, `pass_cross_ref`,
  `apply_*_pop`), the catalog (`build/extend_validation_catalog`, `missing_script_report`), and
  `validate_scripts`.

Note: the cli command wrapper for `validate-script` already lives in `src/commands/validate.rs`
(from the cli refactor) — **do not touch it**; this is purely the engine in `src/validate.rs`.

## API + coupling notes
- **Public API is small:** `ValidationError` (8 uses), `Cs2Validator` (2), `ValidationReport`,
  `BatchReport`, `build_validation_catalog`, `extend_validation_catalog`, `validate_scripts`,
  `is_valid`. Keep them defined/re-exported in `mod.rs` so `crate::validate::X` resolves unchanged.
- **The passes/stack methods are inherent methods on `Cs2Validator<'a>`.** Rust allows **multiple
  inherent `impl` blocks for the same type across child modules** of the same crate, and **a child
  module can access the parent type's private fields**. So `stack_effect.rs` can hold
  `impl<'a> Cs2Validator<'a> { fn stack_effect_for(..) .. }` and freely read `Cs2Validator`'s private
  fields — **no field-visibility widening needed**. (If clippy's `redundant_pub_crate` complains
  about any genuinely-needed `pub(crate)`, follow the overlay_plan precedent: plain `pub` inside a
  `pub(crate) mod` stays crate-internal.)

## Target architecture
- `src/validate/mod.rs` — the validator core: all the types, the `Cs2Validator` struct + its entry
  `impl` block (`new`/`validate`/`validate_compiled`/`pass_structural`/`pass_stack`/`pass_cross_ref`/
  `apply_*_pop`), the catalog fns (`build/extend_validation_catalog`, `missing_script_report`),
  `validate_scripts`, `is_valid`, the `pub mod` declarations, and the re-exports preserving the API.
- `src/validate/stack_effect.rs` — a second `impl<'a> Cs2Validator<'a>` block holding
  `stack_effect_for` (~826), `varp_stack_effect_for`, `gosub_stack_effect` (the opcode stack-effect
  computation). This is the big, self-contained extraction.
- `src/validate/tests.rs` — `#[cfg(test)] mod` (declared as `#[cfg(test)] mod tests;` in mod.rs)
  holding the entire test module **verbatim** (`test_data_dir`, `build_ctx`, all ~14 test fns), with
  `use super::*;` (+ whatever else it currently imports) so it sees the engine.

(If `pass_stack`/`apply_*_pop` read more naturally next to `stack_effect_for`, they may move to
`stack_effect.rs` too — group by cohesion; the goal is `mod.rs` ≈ the ~640-LOC core, `stack_effect.rs`
the opcode-table concern, `tests.rs` the tests.)

## This refactor is exceptionally well-gated
The ~1,768 LOC of tests you're relocating ARE the behavior gate (they're part of `cargo test`). After
moving, they must still pass — that's the proof the engine logic is untouched. Also reconstruct the
original from `git show HEAD:src/validate.rs` and diff the moved blocks → byte-identical (modulo
module headers / `use` lines / visibility). Report both.

## Execution discipline — incremental, ALWAYS GREEN
`git mv validate.rs validate/mod.rs` FIRST (Rust forbids both coexisting; keeps every step
compiling). Then, one extraction at a time with `cargo build --release` after each:
1. Move the test module → `tests.rs` (biggest, cleanest, lowest-risk; shrinks the file by half).
   Run `cargo test --release` — all tests pass from the new location.
2. Move `stack_effect_for`/`varp_stack_effect_for`/`gosub_stack_effect` → `stack_effect.rs` (2nd impl
   block). Build + test.
3. Trim `mod.rs` imports; confirm green.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No validation-logic changes. Public names/signatures unchanged.
- **Tests:** the full suite (**518**) must pass after every step and at the end — the relocated
  validate tests are the gate. Do NOT edit any test body (move them verbatim).
- **clippy:** `cargo clippy --release --all-targets` stays at **0**.
- **Scope:** only `src/validate.rs` → `src/validate/**`. Do NOT touch `src/commands/validate.rs`,
  `server/`, generated files, or anything else. You should NOT need to edit `lib.rs` (the
  `validate.rs` → `validate/mod.rs` rename via `git mv` suffices). Tree is clean.
- Downstream calls only the public items — no consumer edits expected.
- Use Edit/Write; `python3` to search (a hook blocks grep/rg on repo files — grep works on `/tmp`).
  Large verbatim block moves via a `python3` script writing the file is fine — moved code
  byte-identical.
- Do **not** commit or push.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed (incl. the relocated validate tests)
cargo clippy --release --all-targets        # 0
wc -l src/validate/*.rs ; git diff --stat | tail
# + the byte-identity diff result
```

## Done criteria
- `validate.rs` → `validate/` = `mod.rs` (~640, the core) + `stack_effect.rs` (~900) + `tests.rs`
  (~1,768). Public API identical; build green, 518 tests pass, clippy 0, byte-identity confirmed.

## Budget valve
If you near limits before all extractions, **stop at a green state** (compiles + all tests pass) and
report what remains. Partial-but-green is a success; a broken build is not.
