//! Stage 2 drift gates against the real client tree.
//!
//! Skips silently when the client tree or the checked-in registry is absent so
//! CI without the client checkout still passes.
//!
//! 1. Generation drift: the generator's `ClientScriptCommand.java`,
//!    `Cs2Dispatch.java`, and `categories-910.json` are byte-identical to the
//!    files on disk.
//! 2. Extraction fixed point: re-running extraction in-process reproduces the
//!    checked-in `registry-910.json` byte-for-byte.

use rs3_cache_rs::cs2_javagen::{Cs2JavaGenOpts, generate};
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn client_root() -> PathBuf {
    crate_dir().join("../../client")
}

fn data_dir() -> PathBuf {
    crate_dir().join("data")
}

fn registry_path() -> PathBuf {
    data_dir().join("cs2").join("registry-910.json")
}

fn skip_if_tree_absent() -> bool {
    let sr = client_root()
        .join("client/src/main/java/rs2/client/clientscript/ScriptRunner.java");
    if !sr.is_file() {
        eprintln!("skip: client tree absent at {}", sr.display());
        return true;
    }
    if !registry_path().is_file() {
        eprintln!("skip: registry absent at {}", registry_path().display());
        return true;
    }
    false
}

#[test]
fn generation_is_byte_stable_against_tree() {
    if skip_if_tree_absent() {
        return;
    }
    let client = client_root();
    let data = data_dir();
    let opts = Cs2JavaGenOpts {
        registry: None,
        client_root: &client,
        out_dir: None,
        data_dir: &data,
        check: true,
    };
    let emissions = generate(&opts).expect("generate emissions");
    assert_eq!(emissions.len(), 3, "three emissions expected");
    for emission in &emissions {
        let on_disk = std::fs::read_to_string(&emission.path)
            .unwrap_or_else(|e| panic!("read {}: {e}", emission.path.display()));
        assert_eq!(
            on_disk,
            emission.content,
            "generated {} differs from disk",
            emission.path.display()
        );
    }
}

#[test]
fn extraction_is_a_fixed_point() {
    if skip_if_tree_absent() {
        return;
    }
    let checked_in = std::fs::read(registry_path()).expect("read checked-in registry");

    // Reproduce the canonical invocation exactly: relative `--data-dir data` and
    // the default `../../client` client-root. Cargo runs integration tests with
    // the crate root as the working directory, so these relative paths resolve
    // to the same source-path strings the checked-in registry recorded — which
    // a byte-for-byte fixed point requires (the registry records paths verbatim).
    let client = std::path::PathBuf::from("../../client");
    let data = std::path::PathBuf::from("data");

    let tmp = std::env::temp_dir().join("cs2-registry-fixedpoint.json");
    let tmp_report = std::env::temp_dir().join("cs2-registry-fixedpoint.report.json");
    rs3_cache_rs::cs2_registry::run(&rs3_cache_rs::cs2_registry::Cs2RegistryOpts {
        client_root: &client,
        data_dir: &data,
        out_file: Some(&tmp),
        report_file: Some(&tmp_report),
    })
    .expect("re-extract registry");
    let fresh = std::fs::read(&tmp).expect("read fresh registry");
    assert_eq!(
        fresh, checked_in,
        "re-extracted registry differs from checked-in data/cs2/registry-910.json"
    );
}
