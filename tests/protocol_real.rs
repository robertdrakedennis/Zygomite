//! Stage 6 drift + determinism gate for `extract-protocol` / `generate-protocol`.
//!
//! Skips silently when the client/server source trees are absent so CI without
//! them still passes. When present:
//! - run extraction in-process and byte-compare the schema, report, and baseline
//!   against the checked-in `data/protocol/910/` files;
//! - run generation in-process and byte-compare both emitted artifacts against
//!   the server/client trees;
//! - double-run both subcommands and assert byte-identical output.

use rs3_cache_rs::protocol_registry::{
    ExtractProtocolOpts, GenerateProtocolOpts, Prot, extract, generate,
};
use std::path::{Path, PathBuf};

// `cargo test` runs with the crate directory as CWD, so the relative roots below
// resolve exactly as the `extract-protocol` / `generate-protocol` CLI defaults do.
// Using the same path strings keeps the schema's recorded `source` byte-identical
// to the checked-in files.
fn client_root() -> PathBuf {
    PathBuf::from("../../client")
}

fn server_root() -> PathBuf {
    PathBuf::from("../../server")
}

fn schema_dir() -> PathBuf {
    PathBuf::from("data/protocol/910")
}

fn server_prot_java(client_root: &Path) -> PathBuf {
    client_root.join("client/src/main/java/com/jagex/game/network/protocol/ServerProt.java")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn extract_matches_disk_and_is_deterministic() {
    let client = client_root();
    let server = server_root();
    let java = server_prot_java(&client);
    if !java.is_file() {
        eprintln!("skip: client ServerProt.java absent at {}", java.display());
        return;
    }

    let out_dir = schema_dir();
    let opts = ExtractProtocolOpts {
        client_root: &client,
        server_root: &server,
        out_dir: &out_dir,
    };
    let a = extract(&opts).expect("extract");

    // Counts match the spec's verified ground truth (§1.1).
    assert_eq!(a.counts["server"], 195, "server packet count");
    assert_eq!(a.counts["client"], 123, "client packet count");
    assert_eq!(a.counts["login"], 12, "login packet count");

    // Byte-compare schema/report/baseline against the checked-in files.
    for prot in [Prot::Server, Prot::Client, Prot::Login] {
        let path = out_dir.join(format!("{}_prot.json", prot.tag()));
        assert_eq!(
            a.schemas[prot.tag()],
            read(&path),
            "{} schema differs from disk; re-run extract-protocol",
            prot.tag()
        );
    }
    assert_eq!(
        a.report,
        read(&out_dir.join("protocol-910.report.json")),
        "report differs from disk; re-run extract-protocol"
    );
    assert_eq!(
        a.baseline,
        read(&out_dir.join("known-divergences.json")),
        "baseline differs from disk; re-run extract-protocol"
    );

    // Determinism: a second run is byte-identical.
    let b = extract(&opts).expect("extract twice");
    assert_eq!(a.report, b.report);
    assert_eq!(a.baseline, b.baseline);
    assert_eq!(a.schemas, b.schemas);
}

#[test]
fn generate_matches_disk_and_is_deterministic() {
    let client = client_root();
    let server = server_root();
    let dir = schema_dir();
    if !dir.join("server_prot.json").is_file() {
        eprintln!("skip: schema absent at {}", dir.display());
        return;
    }

    let opts = GenerateProtocolOpts {
        schema_dir: &dir,
        server_root: &server,
        client_root: &client,
        check: false,
    };
    let a = generate(&opts).expect("generate");

    // Byte-compare the two emitted artifacts against the trees.
    assert_eq!(
        a.server_ts.1,
        read(&a.server_ts.0),
        "protocol910.ts differs from disk; re-run generate-protocol"
    );
    assert_eq!(
        a.client_tsv.1,
        read(&a.client_tsv.0),
        "protocol-910.tsv differs from disk; re-run generate-protocol"
    );

    // Stage 7: the payload encoders artifact (validates payloads.json on the
    // way — mirror + size checks fire during `generate`, so reaching here at
    // all proves the checked-in schema is internally consistent).
    let enc = a
        .encoders_ts
        .as_ref()
        .expect("encoders910.ts must be emitted when payloads.json is present");
    assert_eq!(
        enc.1,
        read(&enc.0),
        "encoders910.ts differs from disk; re-run generate-protocol"
    );

    // Determinism: a second run is byte-identical.
    let b = generate(&opts).expect("generate twice");
    assert_eq!(a.server_ts.1, b.server_ts.1);
    assert_eq!(a.client_tsv.1, b.client_tsv.1);
    assert_eq!(
        a.encoders_ts.as_ref().map(|e| &e.1),
        b.encoders_ts.as_ref().map(|e| &e.1),
        "encoders910.ts not deterministic across runs"
    );
}
