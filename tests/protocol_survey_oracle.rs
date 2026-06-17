//! Oracle + determinism gate for `survey-payloads` (the port of the retired
//! `scripts/protocol-payload-survey.py`).
//!
//! Skips silently when the client/server source trees are absent so CI without
//! them still passes. When present:
//! - run the survey in-process and byte-compare `payload-classification.json`
//!   and `payloads.json` against the checked-in `data/protocol/910/` files;
//! - double-run and assert byte-identical output (determinism, no timestamps).

use rs3_cache_rs::protocol_registry::{SurveyPayloadsOpts, survey};
use std::path::{Path, PathBuf};

// `cargo test` runs with the crate directory as CWD, so these relative roots
// resolve exactly as the `survey-payloads` CLI defaults do.
fn client_root() -> PathBuf {
    PathBuf::from("../../client")
}

fn server_root() -> PathBuf {
    PathBuf::from("../../server")
}

fn out_dir() -> PathBuf {
    PathBuf::from("data/protocol/910")
}

fn server_prot_ts(server_root: &Path) -> PathBuf {
    server_root.join("src/jagex/network/protocol/ServerProt.ts")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn survey_matches_disk_and_is_deterministic() {
    let client = client_root();
    let server = server_root();
    let ts = server_prot_ts(&server);
    if !ts.is_file() {
        eprintln!("skip: server ServerProt.ts absent at {}", ts.display());
        return;
    }

    let dir = out_dir();
    let opts = SurveyPayloadsOpts {
        client_root: &client,
        server_root: &server,
        out_dir: &dir,
    };
    let a = survey(&opts).expect("survey");

    // Byte-compare both emitted JSON documents against the checked-in files.
    assert_eq!(
        a.classification,
        read(&dir.join("payload-classification.json")),
        "payload-classification.json differs from disk; re-run `survey-payloads`"
    );
    assert_eq!(
        a.payloads,
        read(&dir.join("payloads.json")),
        "payloads.json differs from disk; re-run `survey-payloads`"
    );

    // Determinism: a second run is byte-identical.
    let b = survey(&opts).expect("survey twice");
    assert_eq!(a.classification, b.classification);
    assert_eq!(a.payloads, b.payloads);
    assert_eq!(a.summary, b.summary);
}
