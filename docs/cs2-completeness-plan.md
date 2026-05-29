# CS2 Decompiler / Compiler / Transpiler — Completeness Plan

Goal: move the toolchain from "lossless byte round-trip + best-effort structured
decompile" to "high-coverage, editable, recompilable structured TypeScript across
the full corpus on both builds (910 + 947)".

## Baseline (measured 2026 on 947, full corpus)

`transpile-scripts --all-scripts` on 947 (20,577 scripts, 0 transpile errors):

| Metric | Value |
|---|---|
| Decompile→**editable structured TS** (recompilable) | **1,800 / 20,577 = 8.7%** |
| Falls back to embedded **ASM trailer** (lossless but not editable) | 18,777 = 91.3% |
| Byte-perfect decode→encode→decode roundtrip (sampled 100/build, 910+947) | passing |

Top blockers preventing structured recovery (script-occurrences; avg 1.55/script):

| Blocker | Scripts | % | Owner |
|---|---|---|---|
| `residual_goto` | 12,773 | 62% | control-flow recovery (`cfg.rs`) |
| `commented_branch` | 10,068 | 49% | control-flow recovery (`cfg.rs`) |
| `reverse_unsupported` | 5,270 | 26% | lowering / recovery (`ts_lower.rs`/`expr_recovery.rs`) |
| `residual_pop` | 3,699 | 18% | expression recovery (`expr_recovery.rs`) |

**Read:** byte-fidelity is essentially done; the real completeness gap is the
**decompile/recovery direction**, dominated by control-flow recovery (goto →
structured if/while/switch). Two blockers (`residual_goto`, `commented_branch`)
gate the majority of the corpus.

Definitions of "complete":
- **Lossless**: every script round-trips byte-identically (already true via ASM
  trailer; needs full-corpus proof + CI gate).
- **Editable**: a high fraction decompile to structured TS a human can edit.
- **Recompilable**: editable TS recompiles to byte-identical CS2 (closed loop).

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

---

## P0 — Make coverage a tracked, repeatable metric (do first)
- [ ] **P0.1** Add a `coverage` report (or `--coverage` on `transpile-scripts`) emitting structured
  metrics: editable %, blocker histogram, transpile-error count, per-build. Reuse the existing
  `transpile-diagnostics.json` aggregation; emit one canonical JSON event.
- [ ] **P0.2** Measure the same baseline on **910** (only 947 measured so far). Record both.
- [ ] **P0.3** Commit baseline numbers here and re-measure after each P1/P2 change to show movement.

## P1 — Control-flow recovery (the dominant lever: ~62%+49% of corpus)
Target `cfg.rs` (build_cfg / emit_structured) + the branch/goto handling.
- [ ] **P1.1** Eliminate `residual_goto`: reconstruct structured loops (`while`/`do`/early-exit) and
  nested `if/else` from the branch graph so no goto remains. Biggest single win (62%).
- [ ] **P1.2** Eliminate `commented_branch`: fold the branches currently emitted as comments into
  real structured conditions (49%). Likely the same CFG work as P1.1.
- [ ] **P1.3** Add structured-recovery regression tests over a representative script set; assert the
  editable % rises and these blockers fall toward 0.

## P2 — Expression recovery (`residual_pop` 18%, `reverse_unsupported` 26%)
Target `expr_recovery.rs` + `ts_lower.rs`.
- [ ] **P2.1** Fold `residual_pop`: leftover stack pops not absorbed into expressions/statements →
  recover into assignments/discards so no residual pop remains.
- [ ] **P2.2** Enumerate the distinct `reverse_unsupported` causes (instrument the diagnostic to
  carry the specific construct/opcode), then implement the top offenders. Currently a catch-all —
  break it down before building.
- [ ] **P2.3** Lowerer feature gaps surfaced by the compiler audit that also block recompile:
  subtraction on 910 (synthesize `add(a, neg(b))` or document as engine-unsupported), `goto`,
  string arrays — implement or give precise unsupported diagnostics.

## P3 — Close the recompile loop (prove editable == recompilable)
- [ ] **P3.1** Add a **structured-recompile roundtrip** test: for every `editableStructured` script,
  decode → structured TS → `assemble-script --strict-structured` → assert byte-identical to source.
  (Today only decode→encode is proven; the structured path is only spot-checked by `reversible_ts`.)
- [ ] **P3.2** Run it over the full editable set on both builds; fix any structured-recompile
  mismatch. As P1/P2 raise the editable set, this gate grows with it.

## P4 — Adversarial audit of the decompile direction (unaudited half)
- [ ] **P4.1** Apply the same multi-dimension audit used on the compiler to `expr_recovery.rs`,
  `cfg.rs`, `codegen.rs`, `writer.rs`, `structured_writer.rs`: panics/`unreachable!`/`unwrap` on
  real input, silent fallbacks, fidelity gaps, and the `reverse_unsupported`/comment escape hatches.

## P5 — Full-corpus & cross-build byte fidelity (lossless guarantee)
- [ ] **P5.1** Run the byte-perfect roundtrip with `RS3_ROUNDTRIP_LIMIT=0` on the full 20,577 scripts
  for **both** 910 and 947; fix any non-byte-identical script.
- [ ] **P5.2** Promote a bounded full-corpus (or large-sample) roundtrip to a CI-runnable gate
  (env-gated on cache availability, like the existing tests).

## P6 — Generative / fuzz testing for the parsers
- [ ] **P6.1** `proptest` roundtrip for `js5` group pack/unpack and `script` encode/decode:
  `decode(encode(x)) == x` over arbitrary valid structures.
- [ ] **P6.2** Fuzz the decoder against malformed bytes (the C4–C9 bails should make it fail cleanly,
  never panic) — confirms robustness now that silent fallbacks are gone.

## Sequencing & exit criteria
1. **P0** first (can't manage what you don't measure; cheap).
2. **P1** is the highest-leverage work — it alone should move editable % from ~9% toward a majority.
3. **P2** follows / overlaps P1 (shared recovery code).
4. **P3** continuously, as the editable set grows.
5. **P4/P5/P6** harden and prove.

Exit criteria for "complete enough":
- Editable structured-recovery rate ≥ ~90% on both builds (P1+P2), with remaining cases carrying
  precise, enumerated unsupported diagnostics (not catch-alls).
- 100% of `editableStructured` scripts recompile byte-identically (P3).
- Full-corpus decode→encode byte-identical on both builds, gated in CI (P5).
- Decompile direction audited (P4); parsers fuzzed (P6).

## Notes
- Prior work: [cs2-compiler-audit.md](cs2-compiler-audit.md) hardened the **compile** direction
  (verification, no silent corruption, accurate filtered signatures). This plan targets the
  **decompile/recovery** direction, which is where the coverage gap lives.
- The ASM trailer fallback means the tool is already **lossless** today; this plan is about making
  the output **editable + recompilable**, i.e. fulfilling the "transpiler" promise.
