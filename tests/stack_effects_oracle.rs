//! Oracle + determinism gate for `extract-stack-effects` (the port of the
//! retired `scripts/extract-stack-effects.py`).
//!
//! Globs the whole clientscript package (the bug the port fixes: the Python read
//! one hardcoded `ScriptRunner.java`, but the handlers were split across the
//! `*Ops.java` classes), runs the extraction in-process, and byte-compares the
//! result against the checked-in `data/stack-effects.txt`. Skips silently when
//! the client source tree is absent so CI without it still passes.

use rs3_cache_rs::stack_effects::{
    DEFAULT_CLIENTSCRIPT_DIR, DEFAULT_OPCODE_FILES, ExtractStackEffectsOpts, extract,
};
use std::path::{Path, PathBuf};

// `cargo test` runs with the crate directory as CWD, so these relative roots
// resolve exactly as the `extract-stack-effects` CLI defaults do.
fn opcode_files() -> Vec<PathBuf> {
    DEFAULT_OPCODE_FILES.iter().map(PathBuf::from).collect()
}

fn committed() -> PathBuf {
    PathBuf::from("data/stack-effects.txt")
}

#[test]
fn extract_matches_committed_table_and_is_deterministic() {
    let dir = PathBuf::from(DEFAULT_CLIENTSCRIPT_DIR);
    if !dir.is_dir() {
        eprintln!("skip: clientscript package absent at {}", dir.display());
        return;
    }
    let books = opcode_files();
    let out = Path::new("/dev/null"); // `extract` does not write; `run` would.
    let opts = ExtractStackEffectsOpts {
        clientscript_dir: &dir,
        opcode_files: &books,
        out,
    };

    let table = extract(&opts).expect("extract stack effects");
    let on_disk = std::fs::read_to_string(committed()).expect("read committed stack-effects.txt");
    assert_eq!(
        table, on_disk,
        "extract-stack-effects output differs from data/stack-effects.txt; re-run the subcommand"
    );

    // Determinism: a second run is byte-identical (sorted, no timestamps).
    let again = extract(&opts).expect("extract twice");
    assert_eq!(table, again);
}

#[test]
fn extraction_reads_more_than_just_scriptrunner() {
    // Guard the whole point of the port: globbing the package must surface far
    // more handlers than `ScriptRunner.java` alone (which now holds almost none
    // of the stack-touching opcode handlers).
    let dir = PathBuf::from(DEFAULT_CLIENTSCRIPT_DIR);
    if !dir.is_dir() {
        eprintln!("skip: clientscript package absent at {}", dir.display());
        return;
    }
    let books = opcode_files();
    let out = Path::new("/dev/null");

    let whole_pkg = ExtractStackEffectsOpts {
        clientscript_dir: &dir,
        opcode_files: &books,
        out,
    };
    let rows = extract(&whole_pkg)
        .expect("extract")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .count();
    assert!(
        rows > 500,
        "expected the whole-package glob to yield hundreds of handlers, got {rows}"
    );
}
