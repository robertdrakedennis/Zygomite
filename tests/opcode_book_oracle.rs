//! Oracle + determinism gate for `derive-opcode-book` (the port of the retired
//! `scripts/derive-opcode-book.py`).
//!
//! Derives the 948 book from the 947 book by cross-cache lockstep alignment and
//! checks it against the committed `data/opcodes-948.txt`. Gated on both flat
//! caches existing (`cache/unpacked/947` & `948`); skips with a clear message
//! when either is absent, matching the other cache-dependent tests.
//!
//! ## Why this is a *consistency* gate, not byte-equality
//!
//! `data/opcodes-948.txt` is **hand-synced from upstream** (its header: "Synced
//! from zwyz/rs3-cache master … Hand-synced INPUT — not generated"), and carries
//! 2,244 commands. The lockstep derivation can only resolve the ~1,230 commands
//! that actually appear (uncontested) in the ~20k shared scripts — the newer
//! widget families (`cc_*`/`if_*`, `push_long_constant`, `branch_if_true/false`,
//! …) are never exercised by an unchanged 947↔948 script, so they cannot be
//! voted on. The retired Python produced this same strict subset; byte-equality
//! against the full upstream file was never achievable from these caches.
//!
//! So the meaningful invariant — the one that proves the alignment is *correct*
//! and the `script.rs` operand-width reuse agrees with the bytecode — is:
//! every `name,id` the derivation emits matches the committed file verbatim
//! (the walk never derives a *wrong* opcode), the emitted names follow the
//! old-book order as a subsequence, and the run is deterministic. A wrong
//! width would shift opcodes and break the verbatim-subset check immediately.

use rs3_cache_rs::opcode_book::{DeriveOpcodeBookOpts, derive};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn old_cache() -> PathBuf {
    PathBuf::from("../../cache/unpacked/947")
}

fn new_cache() -> PathBuf {
    PathBuf::from("../../cache/unpacked/948")
}

fn old_book() -> PathBuf {
    PathBuf::from("data/opcodes-947.txt")
}

fn committed_948() -> PathBuf {
    PathBuf::from("data/opcodes-948.txt")
}

fn caches_present() -> bool {
    old_cache().join("12").is_dir() && new_cache().join("12").is_dir()
}

/// The committed book as a `name,id` line set (comment lines stripped) plus the
/// in-order command list.
fn committed_lines_and_order() -> (HashSet<String>, Vec<String>) {
    let text = std::fs::read_to_string(committed_948()).expect("read committed opcodes-948.txt");
    let mut lines = HashSet::new();
    let mut order = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("//") || !line.contains(',') {
            continue;
        }
        lines.insert(line.to_string());
        if let Some((name, _)) = line.rsplit_once(',') {
            order.push(name.to_string());
        }
    }
    (lines, order)
}

#[test]
fn derived_book_is_a_verbatim_consistent_subset_of_committed() {
    if !caches_present() {
        eprintln!("skip: 947/948 flat caches absent (cache/unpacked/947, /948)");
        return;
    }
    let old = old_cache();
    let new = new_cache();
    let book = old_book();
    let out = Path::new("/dev/null"); // `derive` does not write; `run` would.
    let opts = DeriveOpcodeBookOpts {
        old_cache: &old,
        new_cache: &new,
        old_book: &book,
        out,
        archive: 12,
    };

    let outcome = derive(&opts).expect("derive opcode book");
    assert!(
        outcome.derived > 1000,
        "expected the lockstep walk to resolve >1000 commands, got {}",
        outcome.derived
    );
    assert_eq!(outcome.conflicts, 0, "unexpected vote conflicts");

    let (committed_lines, committed_order) = committed_lines_and_order();

    // 1. Every derived `name,id` line is present verbatim in the committed book
    //    (the alignment never derives a *wrong* opcode for any command).
    let derived_lines: Vec<&str> = outcome.book.lines().collect();
    for line in &derived_lines {
        assert!(
            committed_lines.contains(*line),
            "derived line `{line}` is not present verbatim in data/opcodes-948.txt"
        );
    }

    // 2. The derived names follow the committed book order as a subsequence
    //    (old-book order is preserved; extras would trail, but here every
    //    derived command is an old-book command).
    let committed_index: std::collections::HashMap<&str, usize> = committed_order
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut last = None;
    for line in &derived_lines {
        let name = line.rsplit_once(',').map(|(n, _)| n).unwrap_or(line);
        let idx = committed_index
            .get(name)
            .unwrap_or_else(|| panic!("derived command `{name}` missing from committed book"));
        if let Some(prev) = last {
            assert!(
                *idx > prev,
                "derived order breaks committed order at `{name}`"
            );
        }
        last = Some(*idx);
    }
}

#[test]
fn derivation_is_deterministic() {
    if !caches_present() {
        eprintln!("skip: 947/948 flat caches absent (cache/unpacked/947, /948)");
        return;
    }
    let old = old_cache();
    let new = new_cache();
    let book = old_book();
    let out = Path::new("/dev/null");
    let opts = DeriveOpcodeBookOpts {
        old_cache: &old,
        new_cache: &new,
        old_book: &book,
        out,
        archive: 12,
    };

    let a = derive(&opts).expect("derive once");
    let b = derive(&opts).expect("derive twice");
    assert_eq!(a.book, b.book, "derived book is not deterministic");
    assert_eq!(a.entries, b.entries);
    assert_eq!(a.used, b.used);
}
