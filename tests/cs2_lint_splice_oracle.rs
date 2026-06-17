//! Regression-lock for `cs2 lint-splice`: the lint must reproduce EXACTLY the
//! port rewrites `build-relic-scripts.py` already applied — no new surprises, no
//! false positives (the plan's validation contract).
//!
//! Oracle (NEVER edit — committed regression artifacts the tool must reproduce):
//!   * `server/cache-patches/relic-system-948/scripts/*.asm.ts` — the 55 spliced
//!     relic listings, already ported to the 910 opcode book by the python
//!     builder. Linting them against book 910 must report CLEAN (zero findings):
//!     no un-ported `sub`/`enum`, no donor `<<12|<<4` db-field constants, no
//!     dangling `db_find` tuple-index push, no `gosub 7924`.
//!   * `build-relic-scripts.py::apply_rewrites` — the SOURCE OF TRUTH for the
//!     rewrite rules. The synthetic round-trip below reconstructs a donor-form
//!     listing and asserts `--fix` produces the exact ported form the builder's
//!     rules dictate (zero-shift, instruction count preserved).
//!
//! The opcode books are loaded from the crate's `data/` dir (registry-910.json +
//! opcodes-948.txt), so the diff is keyed off the same registries the rest of
//! the crate uses.

use std::path::{Path, PathBuf};

use rs3_cache_rs::cs2::lint::{Severity, lint_text, parse_script_id};

/// Crate-relative path to the committed relic listings (the oracle).
fn relic_scripts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../server/cache-patches/relic-system-948/scripts")
}

/// Crate-relative `data/` dir holding the opcode-book registries.
fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// Collect every `*.asm.ts` listing in the oracle dir.
fn oracle_listings() -> Vec<PathBuf> {
    let dir = relic_scripts_dir();
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read relic scripts dir {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".asm.ts"))
        })
        .collect();
    out.sort();
    out
}

/// Every committed (already-ported) relic listing lints CLEAN against book 910.
/// A single finding would mean either the builder left an un-ported opcode
/// behind, or the lint raises a false positive — both are regressions.
#[test]
fn committed_relic_listings_lint_clean() {
    let listings = oracle_listings();
    assert_eq!(
        listings.len(),
        55,
        "expected 55 committed relic listings (the splice set), got {}",
        listings.len()
    );
    let data = data_dir();
    let mut total_findings = 0usize;
    for path in &listings {
        let file = path.file_name().unwrap().to_str().unwrap();
        let script = parse_script_id(file);
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let result = lint_text(&text, script, &data, 910, 948, false)
            .unwrap_or_else(|e| panic!("lint {file}: {e}"));
        if !result.findings.is_empty() {
            let detail: Vec<String> = result
                .findings
                .iter()
                .map(|f| format!("instr {} {} ({})", f.instr, f.rule, f.detail))
                .collect();
            panic!(
                "{file}: expected CLEAN, got {} finding(s):\n  {}",
                result.findings.len(),
                detail.join("\n  ")
            );
        }
        total_findings += result.findings.len();
    }
    assert_eq!(
        total_findings, 0,
        "committed relic listings must lint clean"
    );
}

/// `--fix` is idempotent on the committed (already-ported) listings: re-running
/// the rewrites changes nothing (there is no `sub`/`enum`/donor-db-field left).
#[test]
fn fix_is_idempotent_on_committed_listings() {
    let data = data_dir();
    for path in oracle_listings() {
        let file = path.file_name().unwrap().to_str().unwrap();
        let script = parse_script_id(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let result = lint_text(&text, script, &data, 910, 948, true)
            .unwrap_or_else(|e| panic!("lint --fix {file}: {e}"));
        // With no findings, the core does not even produce fixed text; when it
        // does (defensive), it must equal the input.
        if let Some(fixed) = result.fixed_text {
            assert_eq!(
                fixed, text,
                "{file}: --fix mutated an already-ported listing"
            );
        }
    }
}

/// The rewrite rules reproduce the python builder's transformations on a
/// synthetic donor-form listing. Each rule is exercised; `--fix` must yield the
/// exact ported form (zero-shift — instruction count preserved).
#[test]
fn fix_reproduces_builder_rewrites_on_synthetic_donor() {
    let data = data_dir();
    // Donor-form listing using every table-driven rewrite (no per-script rules).
    let donor = "\
// @cs2 locals int=1 obj=0 long=0
// @cs2 args int=1 obj=0 long=0
// @cs2 push_int_local 0
// @cs2 push_constant_string int:385024
// @cs2 push_constant_string int:0
// @cs2 db_find 0
// @cs2 push_constant_string int:5
// @cs2 sub 0
// @cs2 enum 0
// @cs2 gosub_with_params 7924
// @cs2 return 0
";
    // Ported form the builder's rules dictate:
    //   385024 >> 4 == 24064; tuple-index push -> `branch 3` (fall-through to the
    //   db_find at instr 3); `int:5; sub` -> `int:-5; add`; `enum` -> `_enum`;
    //   `gosub 7924` -> `gosub 24924`. Instruction count unchanged.
    let expected = "\
// @cs2 locals int=1 obj=0 long=0
// @cs2 args int=1 obj=0 long=0
// @cs2 push_int_local 0
// @cs2 push_constant_string int:24064
// @cs2 branch 3
// @cs2 db_find 0
// @cs2 push_constant_string int:-5
// @cs2 add 0
// @cs2 _enum 0
// @cs2 gosub_with_params 24924
// @cs2 return 0
";
    let result = lint_text(donor, None, &data, 910, 948, true).expect("lint synthetic");
    // Every finding is fixable (no manual).
    assert!(
        result
            .findings
            .iter()
            .all(|f| f.severity == Severity::Fixable),
        "synthetic donor listing should yield only fixable findings: {:?}",
        result.findings
    );
    // All five rules fired.
    let rules: Vec<&str> = result.findings.iter().map(|f| f.rule).collect();
    for rule in [
        "db_field_shift",
        "db_find_arity",
        "sub_to_add",
        "enum_to_underscore",
        "relocate_7924",
    ] {
        assert!(rules.contains(&rule), "missing rule {rule}; got {rules:?}");
    }
    assert_eq!(
        result.fixed_text.expect("fixed text"),
        expected,
        "synthetic --fix did not reproduce the builder's ported form"
    );
}

/// The per-script signature-drift rules (14611 / 14620 / 14587) match the
/// python's `if sid == …` blocks: the donor call becomes a stack-shape no-op.
#[test]
fn fix_applies_signature_drift_rules_per_script() {
    let data = data_dir();

    // 14611: gosub 3092 -> push_constant_string int:0.
    let s14611 = "\
// @cs2 locals int=1 obj=0 long=0
// @cs2 args int=0 obj=0 long=0
// @cs2 gosub_with_params 3092
// @cs2 return 0
";
    let fixed = lint_text(s14611, Some(14611), &data, 910, 948, true)
        .expect("lint 14611")
        .fixed_text
        .expect("fixed");
    assert!(
        fixed.contains("// @cs2 push_constant_string int:0"),
        "14611: gosub 3092 should become push 0; got:\n{fixed}"
    );
    assert!(!fixed.contains("3092"), "14611: 3092 call must be gone");

    // 14620: push 6 -> push 0, and gosub 1858 -> bitcount.
    let s14620 = "\
// @cs2 locals int=1 obj=0 long=0
// @cs2 args int=0 obj=0 long=0
// @cs2 push_constant_string int:6
// @cs2 gosub_with_params 1858
// @cs2 return 0
";
    let fixed = lint_text(s14620, Some(14620), &data, 910, 948, true)
        .expect("lint 14620")
        .fixed_text
        .expect("fixed");
    assert!(fixed.contains("// @cs2 push_constant_string int:0"));
    assert!(fixed.contains("// @cs2 bitcount 0"));
    assert!(!fixed.contains("1858"), "14620: 1858 call must be gone");

    // 14587: gosub 13022 -> bitcount.
    let s14587 = "\
// @cs2 locals int=1 obj=0 long=0
// @cs2 args int=0 obj=0 long=0
// @cs2 gosub_with_params 13022
// @cs2 return 0
";
    let fixed = lint_text(s14587, Some(14587), &data, 910, 948, true)
        .expect("lint 14587")
        .fixed_text
        .expect("fixed");
    assert!(fixed.contains("// @cs2 bitcount 0"));
    assert!(!fixed.contains("13022"), "14587: 13022 call must be gone");
}

/// `parse_script_id` recovers the numeric id from `scriptNNNN.asm.ts`.
#[test]
fn parses_script_ids_from_filenames() {
    assert_eq!(parse_script_id("script14611.asm.ts"), Some(14611));
    assert_eq!(parse_script_id("script24924.asm.ts"), Some(24924));
    assert_eq!(parse_script_id("arch_relic_get_data.asm.ts"), None);
    assert_eq!(parse_script_id("notascript.ts"), None);
}
