# `protocol_registry.rs` decomposition — architecture refactor plan

**Goal:** split the 2,537-LOC flat `src/protocol_registry.rs` into `src/protocol_registry/mod.rs`
(re-exports + shared) plus by-concern submodules. **Strictly behavior-preserving** — pure code
movement, no parse/extract/generate-logic or output changes.

## Why (evidence, measured 2026-06-15)
`src/protocol_registry.rs` = **2,537 LOC** with **six clearly-separable concerns**:
1. **Schema types (~lines 41–471):** `Prot`(enum +impl), `JavaPacket`, `JavaParse`, `TsPacket`,
   `SchemaPacket`, `Schema`, `Finding`, `ReportSummary`, `Report`, `Divergence`,
   `DivergenceBaseline` — the protocol data model.
2. **Source parsing (~lines 130–495):** `parse_java` (+`parse_java_field`/`parse_id_size`/
   `scan_ctor_body`) and `parse_ts` (+`parse_ts_args`) — parse client `*Prot.java` / server TS into
   the schema.
3. **Extract (~lines 472–956):** `ExtractProtocolOpts`, `ExtractOutput`, `extract`, `diff_prot`,
   `run_extract`, `print_extract_summary` — the `extract-protocol` command + cross-diff.
4. **Size-expression mini-language (~lines 929–1200):** `ExprTok`, `Expr`, `ExprParser`(+impl),
   `tokenize_expr`, `parse_term`, `validate_expr` — parses/validates payload size expressions.
5. **Generate (~lines 838–1765):** `GenerateProtocolOpts`, `GenerateOutput`, `PayloadParam`,
   `PayloadField`, `PayloadPacket`, `Payloads`, `generate`, `render_client_tsv`, `validate_payload`,
   `render_size_expr`, `render_encoder_fn`, `render_encoders_ts`, `run_generate` — the
   `generate-protocol` command + payload/encoder codegen.
6. **Tests (~lines 1700–end, ~837 LOC):** the `#[cfg(test)]` module.

## API + coupling notes
- **Public API used externally is tiny:** `run_extract`, `ExtractProtocolOpts`, `run_generate`,
  `GenerateProtocolOpts` (called by `commands/`). Keep them + every other currently-`pub` item
  reachable from `protocol_registry::` via `mod.rs` re-exports so `crate::protocol_registry::X`
  resolves unchanged. `cargo build` is the proof.
- This is a **free-function + types** module (not a big shared-state impl like migrate), so the split
  is clean `use`-wiring — no multiple-impl-block gymnastics. Types carrying impls (`Prot`,
  `ExprParser`) move **with** their impl.

## Target architecture
- `src/protocol_registry/types.rs` — concern 1: the schema model (`Prot`+impl, `JavaPacket`,
  `JavaParse`, `TsPacket`, `SchemaPacket`, `Schema`, `Finding`, `Report`/`ReportSummary`,
  `Divergence`/`DivergenceBaseline`). serde types — **do not change field names/order/attrs/types**.
- `src/protocol_registry/parse.rs` — concern 2: `parse_java` + its helpers, `parse_ts` + `parse_ts_args`.
- `src/protocol_registry/extract.rs` — concern 3: `ExtractProtocolOpts`, `ExtractOutput`, `extract`,
  `diff_prot`, `run_extract`, `print_extract_summary`.
- `src/protocol_registry/expr.rs` — concern 4: `ExprTok`, `Expr`, `ExprParser`(+impl),
  `tokenize_expr`, `parse_term`, `validate_expr`.
- `src/protocol_registry/generate.rs` — concern 5: `GenerateProtocolOpts`, `GenerateOutput`, the
  `Payload*` types, `generate`, `render_*`, `validate_payload`, `run_generate`.
- `src/protocol_registry/tests.rs` — concern 6: the `#[cfg(test)] mod` **verbatim**.
- `src/protocol_registry/mod.rs` (thin) — `pub mod` declarations + `pub use` re-exports preserving the
  full public API. If any small type is genuinely shared across most concerns and has no clear home,
  keep it here.

(If a Payload/Output type is shared between extract and generate, put it in `types.rs`; if used by
only one, keep it with that concern. When in doubt, `types.rs`.)

## Behavior-preservation proof
- **Byte-identical movement:** reconstruct from `git show HEAD:src/protocol_registry.rs` and diff
  moved blocks → byte-identical (modulo module headers / `use` / visibility). Report it.
- **Tests:** the full suite (**518**) must pass after every step and at the end — the ~837 LOC of
  relocated tests + `tests/protocol_real.rs` are the gate. Do NOT edit `tests/`.

## Execution discipline — incremental, ALWAYS GREEN
`git mv protocol_registry.rs protocol_registry/mod.rs` FIRST. Then one concern at a time, leaves
first, `cargo build --release` after each: (1) `tests.rs`, (2) `types.rs`, (3) `expr.rs`, (4)
`parse.rs`, (5) `extract.rs`, (6) `generate.rs`, (7) trim `mod.rs`.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No logic or serde (field names/order/attrs/types) changes. Public
  names/signatures unchanged. Move test bodies verbatim.
- **clippy:** `cargo clippy --release --all-targets` stays at **0**. (Clippy on *your* scaffolding
  lines only → use the overlay_plan/validate precedents; never alter moved bodies.)
- **Scope:** only `src/protocol_registry.rs` → `src/protocol_registry/**`. Do NOT touch
  `src/commands/` (the cli shim), `server/`, generated files, or `lib.rs` (the rename suffices). Tree
  is clean. Use Edit/Write; `python3` to search (hook blocks grep/rg on repo files — grep works on
  `/tmp`). Large verbatim moves via a `python3` script writing the file is fine — byte-identical.
- Do **not** commit or push.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed
cargo clippy --release --all-targets        # 0
wc -l src/protocol_registry/*.rs ; git diff --stat | tail
# + the byte-identity diff result
```

## Done criteria
- `protocol_registry.rs` → `protocol_registry/` = `mod.rs` (thin) + `types`/`parse`/`extract`/`expr`/
  `generate`/`tests`. Public API identical; build green, 518 tests pass, clippy 0, byte-identity
  confirmed.

## Budget valve
If you near limits before all concerns are extracted, **stop at a green state** (compiles + all tests
pass) and report what remains. Partial-but-green is a success; a broken build is not.
