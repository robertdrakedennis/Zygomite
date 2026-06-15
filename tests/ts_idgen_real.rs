//! Stage 5 drift gate: the generated TS id modules + manifest under
//! `server/src/generated/cache` are byte-identical to what `generate-ts-ids`
//! produces from the runtime 948 pack joined with `data/names/910`.
//!
//! Mirrors the Stage 4 opcode-view drift gate: `generate-ts-ids --check` against
//! the on-disk output must find no drift, proving the checked-in generated
//! constants are an exact view of the pack + curated names.
//!
//! Skips silently when the runtime pack or the names dir is absent.

use rs3_cache_rs::ts_idgen::{GenerateTsIdsOpts, run};
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pack_root() -> PathBuf {
    crate_dir()
        .join("..")
        .join("..")
        .join("server")
        .join("data")
        .join("pack-910-base-948-overlay")
}

fn names_dir() -> PathBuf {
    crate_dir().join("data").join("names").join("910")
}

fn out_dir() -> PathBuf {
    crate_dir()
        .join("..")
        .join("..")
        .join("server")
        .join("src")
        .join("generated")
        .join("cache")
}

#[test]
fn generated_ts_ids_are_byte_stable_against_disk() {
    let pack = pack_root();
    let names = names_dir();
    if !pack.join("client.interfaces.js5").is_file() || !names.is_dir() {
        eprintln!(
            "skip: pack ({}) or names dir ({}) absent",
            pack.display(),
            names.display()
        );
        return;
    }

    let out = out_dir();
    let drift = run(&GenerateTsIdsOpts {
        pack_root: &pack,
        names_dir: &names,
        out_dir: &out,
        check: true,
    })
    .expect("generate-ts-ids --check");
    assert!(
        !drift,
        "generate-ts-ids --check reported drift: the generated cache id modules are stale; \
         re-run `cargo run --release -- generate-ts-ids`"
    );
}
