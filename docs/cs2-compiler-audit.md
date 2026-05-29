# CS2 Compiler Audit — Fix Plan & Tracker

Branch: `fix/cs2-compiler-audit`. Scope: the compile direction (TS/ASM → CS2 bytecode):
`script.rs` (encode/assemble), `transpile/{ts_parse,ts_lower,scope,structured}.rs`,
`validate.rs` wiring, `vars.rs`, `packet.rs`. (`sema.rs` was removed — see C11.)

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

## Batch D — Tier 4: quality / dead code / coverage / diagnostics ✅ DONE (commit pending)

- [x] **C11 — Removed dead `sema.rs`** (+ its `mod.rs` re-exports). Kept `scope.rs` — it is NOT
  dead (the decompile `codegen.rs` uses `SymbolTable`). The audit's "scope serves only sema" was
  wrong.
- [x] **C12 — True byte-identity roundtrip** — both 910 and 947 byte-perfect tests now assert
  `encode_script(decoded) == data` AND `encode_script(parse_cs2_asm(asm)) == data` against the
  original cache bytes (100 scripts each, passing). The "byte-identical" claim is now actually
  proven, not just structural.
- [x] **C13 — `--json`** on `assemble-script` (canonical completion event: event/outcome/build/
  subbuild/mode/instruction_count/bytes/verified/duration_ms) and `validate-script` (report JSON
  to stdout).
- [x] **C14 — ASM parse errors carry `line N: <text>`** — loop enumerated; parse_counts, switch
  case, and operand parses wrapped with line context.

## Verification (final)
- `cargo clippy --all-targets` (pedantic+nursery+cargo): clean. `cargo fmt --check`: clean.
- `cargo test --lib`: 118 passed.
- `reversible_ts`: 5/5 (real 910 + 947 assemble). `real_cache` byte-perfect roundtrip: 910 + 947
  100 scripts each, byte-identical.
- `ts_export`: 8/9. The 1 failure (`transpile_script621_947_uses_group_name_and_signature`) is
  **pre-existing** — it fails identically at base commit 2f0d6a1 (before any audit change). Cause:
  `--filter-script script621 --subbuild 0` doesn't resolve to group 621 (real 947 data is
  subbuild 1); a test/data issue, not a compiler bug. Flagged for separate follow-up.

---

## Notes / decisions
- Verified false alarm (NOT a bug): `push_constant_string` discriminator — Rust `0/1/2` matches
  the client wire format (client switches on `BaseVarType.index` but decodes the wire byte via
  serialId, which is `0=int,1=long,2=string`).
