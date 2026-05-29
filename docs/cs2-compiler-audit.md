# CS2 Compiler Audit — Fix Plan & Tracker

Branch: `fix/cs2-compiler-audit`. Scope: the compile direction (TS/ASM → CS2 bytecode):
`script.rs` (encode/assemble), `transpile/{ts_parse,ts_lower,sema,scope,structured}.rs`,
`validate.rs` wiring, `vars.rs`, `packet.rs`.

Bar: the emitted bytes must decode byte-for-byte in the 910 client and execute under
`switch(opcode.index)`. Verified-correct already: trailer layout, switch tables, operand
widths, and the `push_constant_string` discriminator (wire `0=int,1=long,2=string`, confirmed
against the client's `BaseVarType` serialIds). Gates after every batch: `cargo clippy
--all-targets` (pedantic+nursery+cargo denied), `cargo fmt --check`, `cargo test`.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done · `[-]` deferred (with reason)

---

## Batch A — Tier 1: prevents producing broken/unrunnable scripts ✅ DONE (commit pending)

- [x] **C1 — Post-compile verification in `run_assemble_script`.** Added `verify_assembled_script`:
  (a) decode emitted bytes and structurally compare to the compiled script (opcode placeholder
  normalized out), (b) `Cs2Validator::validate_compiled` against the target build catalog;
  `bail!` on errors. Added `--no-verify`. Verified on real 910 + 947 via `reversible_ts` (5/5).
- [x] **C2 — Book-resolve emitted opcode names in the lowerer.** Fixed `Mod→"modulo"`; added a
  `finish()` pass that `bail!`s on any emitted command absent from the target build's opcode book
  (`ctx.has_command`), so `sub` on 910 fails cleanly and `--strict-structured` is a real guarantee.
  (Also delivers C10's clean-rejection.)
- [x] **C3 — Arity-check `emit_ui_call`** — `create`/`deleteAll`/`getText` now use slice-pattern
  destructuring with `bail!` on arity mismatch (no more panic on `UI.create()`).

## Batch B — Tier 2: turn silent corruption/hidden errors into hard errors ✅ DONE (commit pending)

- [x] **C4 — silent zero/placeholder fallbacks → `bail!`** (3 sites): encode_operand catch-all,
  parse_cs2_asm unknown-command (empty still = byte 0; garbage now errors), parse_counts
  (`=`-less + unknown keys now error).
- [x] **C5 — `VarDomain::from_id` unknown id** now `bail!`s (0..=10 valid for 910/947) instead of
  silently remapping to `Player(0)`.
- [x] **C6 — `pjstr` non-Cp1252** now `bail!`s (pjstr/pjstrnull return `Result`; 3 callers `?`'d)
  instead of emitting raw UTF-8.
- Verified: 910 + 947 byte-perfect roundtrips still pass (100 scripts each), 118 lib tests pass.

## Batch C — Tier 3: robustness & fidelity ✅ DONE (commit pending)

- [x] **C7 — Validate gosub arg count/type vs callee signature** — `check_call_arity` bails on a
  total-count mismatch always, and on a per-type (int/obj/long) mismatch when all arg kinds are
  concrete (skips when any is `Unknown`/`Void` to avoid false positives). Verified on real
  910/947 gosub calls (reversible_ts 5/5).
- [x] **C8 — Robust numeric-literal parsing** — use oxc's already-parsed value via
  `numeric_literal_to_i32` (hex/binary/octal/`_` for free), accepting `0..=u32::MAX` reinterpreted
  as i32; applied to all 3 literal sites.
- [x] **C9 — `secondary` operand byte** — `decode_transmog_flag` bails on a byte other than 0/1
  instead of silently collapsing to `false`→`0` on re-encode (chose bail over a wide
  `bool`→`u8` ripple across ~63 sites; canonical scripts only use 0/1, both roundtrips pass).
- [x] **C10 — String-array path** — `array_N[..] = <string>` now bails cleanly (no
  `pop_array_string` opcode exists); also covered defensively by C2's `finish()` book check.

## Batch D — Tier 4: quality / dead code / coverage / diagnostics

- [ ] **C11 — Dead `sema.rs`** — `Sema::new` has zero call sites; all diagnostics are
  warning/note with no spans (can never block). Remove the dead module (+ scope.rs machinery only
  it uses) to end the false "semantic checking exists" signal. (Real checks land via C7.)
- [ ] **C12 — Close the roundtrip test gap** (`tests/real_cache.rs`). Assert
  `encode_script(parse_cs2_asm(asm)) == original_bytes` on the ASM path so symmetric encode/decode
  bugs (C5, C9) are caught.
- [ ] **C13 — Structured/`--json` diagnostics for `assemble-script`/`validate-script`** — one
  canonical event (`event/outcome/duration_ms/build/subbuild/script_id/errors`).
- [ ] **C14 — Line numbers in ASM parse errors** (`parse_cs2_asm`). Thread a 1-based line index
  and wrap operand errors with `line N: <text>` context.

---

## Notes / decisions
- Verified false alarm (NOT a bug): `push_constant_string` discriminator — Rust `0/1/2` matches
  the client wire format (client switches on `BaseVarType.index` but decodes the wire byte via
  serialId, which is `0=int,1=long,2=string`).
