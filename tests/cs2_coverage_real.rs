//! Integration test for `cs2-coverage` against the real runtime pack.
//!
//! Skips silently when the pack file or registry is absent so CI without the
//! server data tree still passes. Asserts only mechanical "the scan really
//! ran" invariants — findings are the deliverable, so it never asserts zero
//! findings.
//!
//! Note on bounds: the spec's draft lower bounds (`groups_present >= 15000`,
//! `groups_decoded >= 14000`) were written against an expected ~20k-group
//! corpus. The current 910-space runtime pack (`pack-910-base-948-overlay`)
//! actually carries 14,313 clientscript groups, all of which decode cleanly,
//! so the bounds here are set to `>= 14000` to track reality while still
//! proving the scan covered the whole pack.

use rs3_cache_rs::cs2_coverage::{Cs2CoverageOpts, scan};
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn data_dir() -> PathBuf {
    crate_dir().join("data")
}

fn pack_root() -> PathBuf {
    crate_dir().join("../../server/data/pack-910-base-948-overlay")
}

#[test]
fn coverage_scan_runs_against_real_pack() {
    let pack_root = pack_root();
    let pack_file = pack_root.join("client.scripts.js5");
    let registry = data_dir().join("cs2/registry-910.json");
    if !pack_file.is_file() {
        eprintln!("skip: pack file absent at {}", pack_file.display());
        return;
    }
    if !registry.is_file() {
        eprintln!("skip: registry absent at {}", registry.display());
        return;
    }

    let report = scan(&Cs2CoverageOpts {
        pack_root: &pack_root,
        pack_file: None,
        registry: None,
        out_file: None,
        data_dir: &data_dir(),
    })
    .expect("coverage scan should complete");

    // Schema pin.
    assert_eq!(report.schema, "cs2-coverage/v1");

    // The scan really ran: it indexed and decoded the bulk of the pack.
    assert!(
        report.summary.groups_present >= 14000,
        "expected many present groups, got {}",
        report.summary.groups_present
    );
    assert!(
        report.summary.groups_decoded >= 14000,
        "expected many decoded groups, got {}",
        report.summary.groups_decoded
    );

    // groups_present never exceeds groups_indexed; decoded never exceeds present.
    assert!(report.summary.groups_present <= report.summary.groups_indexed);
    assert!(report.summary.groups_decoded <= report.summary.groups_present);

    // Summary counters agree with the findings vector.
    let decode_failures = report
        .findings
        .iter()
        .filter(|f| f.kind == "decode_failure")
        .count();
    let unassigned = report
        .findings
        .iter()
        .filter(|f| f.kind == "unassigned_opcode")
        .count();
    let unknown = report
        .findings
        .iter()
        .filter(|f| f.kind == "unknown_opcode")
        .count();
    assert_eq!(decode_failures, report.summary.decode_failures);
    assert_eq!(unassigned, report.summary.unassigned_opcode_findings);
    assert_eq!(unknown, report.summary.unknown_opcode_findings);

    // Findings are sorted by (kind, group, opcode).
    let keys: Vec<(&str, u32, u16)> = report
        .findings
        .iter()
        .map(|f| (f.kind.as_str(), f.group, f.opcode.unwrap_or(0)))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "findings must be sorted");

    // opcode_usage is sorted by id and non-empty.
    assert!(!report.opcode_usage.is_empty());
    let ids: Vec<u16> = report.opcode_usage.iter().map(|u| u.id).collect();
    let mut sorted_ids = ids.clone();
    sorted_ids.sort_unstable();
    assert_eq!(ids, sorted_ids, "opcode_usage must be sorted by id");
    assert_eq!(report.summary.distinct_opcodes_used, report.opcode_usage.len());

    // The report round-trips as valid JSON with the expected schema.
    let json = serde_json::to_string_pretty(&report).expect("report serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("report round-trips");
    assert_eq!(value["schema"], "cs2-coverage/v1");

    // Deterministic: serialize twice, byte-identical.
    let again = serde_json::to_string_pretty(&report).expect("report serializes");
    assert_eq!(json, again);

    // Sanity: the resolved pack path lives under the requested pack root.
    assert!(Path::new(&report.pack_file).ends_with("client.scripts.js5"));
}
