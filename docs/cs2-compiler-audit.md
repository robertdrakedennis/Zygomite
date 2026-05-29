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

## Batch B — Tier 2: turn silent corruption/hidden errors into hard errors

- [ ] **C4 — Replace silent zero/placeholder fallbacks with `bail!`** (3 sites):
  - `encode_operand` catch-all `p4s(0)`/`p1(0)` for unexpected operand variant (`script.rs:767-768`).
  - `parse_cs2_asm` unknown-command `Operand::Byte(0)` on unparseable operand (`script.rs:1244`).
  - `parse_counts` silently dropping `=`-less / unknown keys (`script.rs:~1083`) → wrong header.
- [ ] **C5 — `VarDomain::from_id` unknown id** (`vars.rs:36-38`). Stop mapping unknown→`Player(0)`
  (silent byte corruption on re-encode). Preserve raw id or `bail!` on decode.
- [ ] **C6 — `pjstr` non-Cp1252 fallback** (`packet.rs:347-354`). Stop emitting raw UTF-8 when
  `WINDOWS_1252.encode` reports errors; `bail!` to match the strict `gjstr` decoder.

## Batch C — Tier 3: robustness & fidelity

- [ ] **C7 — Validate gosub arg count/type vs callee signature** (`ts_lower.rs:449` `emit_call`).
  Signature is in hand; `bail!` on arity/`ValueKind` mismatch with a precise message.
- [ ] **C8 — Robust numeric-literal parsing** (`ts_parse.rs:329` +dupes). Accept hex/binary/octal/
  `_` separators and full-u32-as-i32 (colours/bitmasks). Parse the value, not the raw span.
- [ ] **C9 — Preserve `secondary` operand byte** (`script.rs:418/707`). Currently `bool`-collapsed;
  a byte ≠0/1 normalizes to 0. Store raw `u8` (or assert ≤1 on decode).
- [ ] **C10 — Resolve/reject string-array path** (`ts_lower.rs:281`). `pop_array_string`/
  `push_array_string` exist in no opcode table; either wire correct opcodes or `bail!` cleanly.

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
