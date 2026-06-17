//! BYTE-EXACT oracle for the semantic port layer's CONFIG path (plan §9 step 6).
//! `config port` (driven here via [`rs3_cache_rs::port::config`]) re-encodes the
//! relic DB groups through the typed config IR ([`rs3_cache_rs::port::ir::config`])
//! and must reproduce the committed `config transcode` outputs byte-for-byte — the
//! regression oracle that gates routing `config transcode` through the IR path.
//!
//! Oracle (NEVER edit — committed regression artifacts the layer reproduces):
//!   * `relic-system-948/config/40-948.dat(+metadata)` — Config group 40
//!     (DBTABLETYPE) with the relic tables 90/92/94 re-encoded as 910 opcode-1
//!     schemas.
//!   * `relic-system-948/dbtableindex/94.dat(+metadata)` — the relic `DbTableIndex`,
//!     files 0/1/10, in the 910 `BaseVarType`-serial form.
//!
//! As with the existing `config_transcode_oracle`, the gzip byte stream is Node's
//! and NOT reproducible across zlib implementations, so the contract is on the
//! DECOMPRESSED group BODY (what the client decodes) + the metadata JSON. The test
//! also asserts the IR path produces output IDENTICAL to the JSON `transcode_db_
//! groups` path (the equivalence that justifies folding `config transcode` into the
//! IR layer). Both are gated on the local donor/base caches and skip cleanly when
//! absent.

use std::path::{Path, PathBuf};

use rs3_cache_rs::config_transcode::{TranscodeInputs, transcode_db_groups};
use rs3_cache_rs::font::cli::decompress_raw_group;
use rs3_cache_rs::port::book::BuildDescriptor;
use rs3_cache_rs::port::config::port_relic_db_groups;

/// Committed oracle group-40 raw group + metadata.
const GROUP40_DAT: &[u8] =
    include_bytes!("../../../server/cache-patches/relic-system-948/config/40-948.dat");
const GROUP40_META: &str =
    include_str!("../../../server/cache-patches/relic-system-948/config/40-948.metadata.json");
/// Committed oracle DbTableIndex-94 raw group + metadata.
const INDEX94_DAT: &[u8] =
    include_bytes!("../../../server/cache-patches/relic-system-948/dbtableindex/94.dat");
const INDEX94_META: &str =
    include_str!("../../../server/cache-patches/relic-system-948/dbtableindex/94.metadata.json");

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}
fn donor_semantic() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/rs3-cache/948-all/config")
}
fn donor_raw() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/rs3-cache/948-all/raw-flat")
}
fn base_raw() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/rs3-cache/910-all/raw-flat")
}

fn caches_present() -> bool {
    let semantic = donor_semantic();
    semantic.join("dbtables.json").is_file()
        && semantic.join("dbrows.json").is_file()
        && donor_raw().join("2/40.dat").is_file()
        && base_raw().join("2/40.dat").is_file()
}

/// THE GATE: `config port` (the IR path) reproduces the committed relic group-40 +
/// index-94 DECOMPRESSED bodies + metadata byte-for-byte.
#[test]
fn config_port_reproduces_committed_relic_bodies() {
    if !caches_present() {
        eprintln!(
            "skipping config_port_reproduces_committed_relic_bodies: donor/base caches absent"
        );
        return;
    }
    let inputs = TranscodeInputs::load(&donor_semantic()).expect("load donor semantic json");
    let target = BuildDescriptor::load(&data_dir(), 910).expect("load 910 descriptor");
    let out = port_relic_db_groups(&inputs, &base_raw(), &donor_raw(), &target)
        .expect("port relic DB groups through the config IR");

    let oracle_g40 = decompress_raw_group(GROUP40_DAT).expect("decompress oracle group 40");
    assert_eq!(
        out.group40_body,
        oracle_g40,
        "IR-ported group-40 body != oracle ({} vs {} bytes)",
        out.group40_body.len(),
        oracle_g40.len()
    );
    assert_eq!(
        out.group40_metadata.trim_end(),
        GROUP40_META.trim_end(),
        "IR-ported group-40 metadata != oracle"
    );

    let oracle_idx = decompress_raw_group(INDEX94_DAT).expect("decompress oracle index 94");
    assert_eq!(
        out.index94_body,
        oracle_idx,
        "IR-ported index-94 body != oracle ({} vs {} bytes)",
        out.index94_body.len(),
        oracle_idx.len()
    );
    assert_eq!(
        out.index94_metadata.trim_end(),
        INDEX94_META.trim_end(),
        "IR-ported index-94 metadata != oracle"
    );
}

/// The IR path and the legacy JSON `transcode_db_groups` produce IDENTICAL output
/// (bodies + rosters + metadata) — the equivalence that justifies routing
/// `config transcode` through the config IR.
#[test]
fn config_ir_path_equals_json_path() {
    if !caches_present() {
        eprintln!("skipping config_ir_path_equals_json_path: donor/base caches absent");
        return;
    }
    let inputs = TranscodeInputs::load(&donor_semantic()).expect("load donor semantic json");
    let target = BuildDescriptor::load(&data_dir(), 910).expect("load 910 descriptor");

    let json = transcode_db_groups(&inputs, &base_raw(), &donor_raw()).expect("json transcode");
    let ir =
        port_relic_db_groups(&inputs, &base_raw(), &donor_raw(), &target).expect("ir transcode");

    assert_eq!(
        ir.group40_body, json.group40_body,
        "group-40 bodies differ (IR vs JSON)"
    );
    assert_eq!(
        ir.group40_roster, json.group40_roster,
        "group-40 rosters differ"
    );
    assert_eq!(
        ir.group40_metadata, json.group40_metadata,
        "group-40 metadata differ"
    );
    assert_eq!(
        ir.index94_body, json.index94_body,
        "index-94 bodies differ (IR vs JSON)"
    );
    assert_eq!(
        ir.index94_roster, json.index94_roster,
        "index-94 rosters differ"
    );
    assert_eq!(
        ir.index94_metadata, json.index94_metadata,
        "index-94 metadata differ"
    );
}
