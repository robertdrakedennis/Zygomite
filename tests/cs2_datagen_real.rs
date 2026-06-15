//! Stage 4 drift gate: the three generated build-910 opcode views are
//! byte-identical to the files on disk.
//!
//! Mirrors the Stage 2 Java drift gate: `generate-cs2-data --check` must find no
//! drift, proving `opcodes-910.txt`, `opcodes-large-910.txt`, and
//! `opcode-aliases-910.txt` are exact views of `cs2/registry-910.json`.
//!
//! Skips silently when the checked-in registry is absent.

use rs3_cache_rs::cs2_datagen::{Cs2DataGenOpts, run};
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn data_dir() -> PathBuf {
    crate_dir().join("data")
}

fn registry_path() -> PathBuf {
    data_dir().join("cs2").join("registry-910.json")
}

#[test]
fn generated_views_are_byte_stable_against_disk() {
    let registry = registry_path();
    if !registry.is_file() {
        eprintln!("skip: registry absent at {}", registry.display());
        return;
    }

    let data = data_dir();
    let drift = run(&Cs2DataGenOpts {
        registry: None,
        out_dir: None,
        data_dir: &data,
        check: true,
    })
    .expect("generate-cs2-data --check");
    assert!(
        !drift,
        "generate-cs2-data --check reported drift: the 910 opcode views are stale; \
         re-run `generate-cs2-data` after `extract-cs2-registry`"
    );
}
