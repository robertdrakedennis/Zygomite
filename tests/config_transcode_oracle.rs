//! Regression-lock for `config transcode`: the DBTABLETYPE opcode-1 schema
//! encoder and the `DbTableIndex` `BaseVarType`-serial index encoder must
//! byte-reproduce the committed relic-system-948 oracle.
//!
//! Oracle (NEVER edit — committed regression artifacts the tool must reproduce):
//!   * `server/cache-patches/relic-system-948/config/40-948.dat(+metadata)` —
//!     the whole Config group 40 (DBTABLETYPE), with relic tables 90/92/94
//!     re-encoded as 910 opcode-1 schemas.
//!   * `server/cache-patches/relic-system-948/dbtableindex/94.dat(+metadata)` —
//!     the relic `DbTableIndex` (archive 49 group 94), files 0/1/10, in the 910
//!     `BaseVarType`-serial form.
//!
//! These `.dat` files are embedded with `include_bytes!`. The gzip byte stream
//! itself is Node's and NOT reproducible across zlib implementations, so the
//! contract is on the DECOMPRESSED group body (which is what the client decodes)
//! — exactly as the font oracle notes. Two layers are checked:
//!   1. SELF-CONTAINED: decode each re-encoded oracle schema (opcode 1) and
//!      re-encode it via [`encode_dbtable_schema`] (the opcode-2 → opcode-1
//!      transcode); decode each oracle index file and re-encode it via
//!      [`encode_int_index`]. Both must round-trip byte-for-byte.
//!   2. GATED on the donor/base caches: run the full `transcode_db_groups` and
//!      assert its decompressed bodies + metadata match the oracle.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rs3_cache_rs::config::parse_dbtable;
use rs3_cache_rs::config_transcode::{
    TranscodeInputs, encode_dbtable_schema, encode_int_index, transcode_db_groups,
};
use rs3_cache_rs::font::cli::decompress_raw_group;

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

/// Tables re-encoded as 910 opcode-1 schemas (the client-read relic tables).
const REENCODE_TABLES: [u32; 3] = [90, 92, 94];

/// File-id roster parsed out of a `*.metadata.json` `fileIds` array.
fn roster(meta_json: &str) -> Vec<u32> {
    let value: serde_json::Value = serde_json::from_str(meta_json).expect("parse metadata json");
    value["fileIds"]
        .as_array()
        .expect("fileIds array")
        .iter()
        .map(|v| v.as_u64().expect("file id") as u32)
        .collect()
}

/// Split a single-stripe (marker = 1) decompressed group body into its per-file
/// payloads, keyed by `roster` (inverse of the crate's `pack_group_files`).
fn unpack_single_stripe(body: &[u8], roster: &[u32]) -> BTreeMap<u32, Vec<u8>> {
    let n = roster.len();
    let marker = *body.last().expect("marker byte");
    assert_eq!(marker, 1, "oracle uses a single stripe");
    let footer_len = n * 4;
    let sizes_pos = body.len() - 1 - footer_len;
    let mut sizes = Vec::with_capacity(n);
    let mut prev = 0_i64;
    let mut p = sizes_pos;
    for _ in 0..n {
        let delta = i32::from_be_bytes([body[p], body[p + 1], body[p + 2], body[p + 3]]);
        p += 4;
        prev += i64::from(delta);
        sizes.push(prev as usize);
    }
    let mut files = BTreeMap::new();
    let mut off = 0usize;
    for (i, &fid) in roster.iter().enumerate() {
        files.insert(fid, body[off..off + sizes[i]].to_vec());
        off += sizes[i];
    }
    assert_eq!(off, sizes_pos, "file bytes must end at the size footer");
    files
}

/// Decode a 910 `BaseVarType`-serial integer `DbTableIndex` into its
/// `key -> [row ids]` entries (inverse of `encode_int_index`), for round-trip.
fn decode_int_index(bytes: &[u8]) -> BTreeMap<i32, Vec<u32>> {
    let mut p = 0usize;
    let serial = bytes[p];
    p += 1;
    assert_eq!(serial, 0, "BaseVarType.INTEGER serial id");
    let (count, adv) = read_varint(&bytes[p..]);
    p += adv;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        let key = i32::from_be_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]);
        p += 4;
        let (rows, adv) = read_varint(&bytes[p..]);
        p += adv;
        let mut row_ids = Vec::with_capacity(rows as usize);
        for _ in 0..rows {
            let (id, adv) = read_varint(&bytes[p..]);
            p += adv;
            row_ids.push(id);
        }
        out.insert(key, row_ids);
    }
    assert_eq!(p, bytes.len(), "index file fully consumed");
    out
}

/// Read one `pVarInt2` LEB128 value, returning `(value, bytes_consumed)`.
fn read_varint(bytes: &[u8]) -> (u32, usize) {
    let mut value = 0_u32;
    let mut shift = 0_u32;
    let mut i = 0usize;
    loop {
        let byte = bytes[i];
        value |= u32::from(byte & 0x7f) << shift;
        i += 1;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    (value, i)
}

/// The re-encoded oracle schemas (tables 90/92/94) round-trip: decode the
/// opcode-1 bytes and re-encode them; the result must equal the oracle bytes.
/// This locks the opcode-1 schema encoder (`encodeSchema` port) against the
/// committed group, with no external cache.
#[test]
fn oracle_schemas_round_trip_opcode1() {
    let body = decompress_raw_group(GROUP40_DAT).expect("decompress group 40");
    let files = unpack_single_stripe(&body, &roster(GROUP40_META));
    for table_id in REENCODE_TABLES {
        let oracle = files
            .get(&table_id)
            .unwrap_or_else(|| panic!("oracle group 40 missing table {table_id}"));
        assert_eq!(oracle[0], 1, "table {table_id} oracle must be opcode 1");
        let decoded = parse_dbtable(table_id, oracle)
            .unwrap_or_else(|e| panic!("decode oracle table {table_id}: {e}"));
        let re = encode_dbtable_schema(&decoded)
            .unwrap_or_else(|e| panic!("re-encode table {table_id}: {e}"));
        assert_eq!(
            re,
            *oracle,
            "table {table_id}: re-encoded opcode-1 schema != oracle ({} vs {} bytes)",
            re.len(),
            oracle.len()
        );
    }
}

/// The oracle `DbTableIndex` files (0/1/10) round-trip through `encode_int_index`:
/// decode each to its entries, re-encode, assert byte-identity. Locks the
/// `encodeIntIndex` port against the committed index, with no external cache.
#[test]
fn oracle_index_files_round_trip() {
    let body = decompress_raw_group(INDEX94_DAT).expect("decompress index 94");
    let files = unpack_single_stripe(&body, &roster(INDEX94_META));
    assert_eq!(
        files.keys().copied().collect::<Vec<_>>(),
        vec![0, 1, 10],
        "index 94 roster"
    );
    for (&fid, bytes) in &files {
        let entries = decode_int_index(bytes);
        let re = encode_int_index(&entries);
        assert_eq!(
            re,
            *bytes,
            "index file {fid}: re-encoded != oracle ({} vs {} bytes)",
            re.len(),
            bytes.len()
        );
    }
    // The master index (file 0) maps key 0 -> all 32 relic rows.
    let master = decode_int_index(&files[&0]);
    assert_eq!(master.len(), 1, "master index has one key");
    assert_eq!(master[&0].len(), 32, "master index lists all 32 relic rows");
}

/// The oracle group-40 body unpacks to 80 files with tables 88/89 as donor
/// opcode-2 and 90/92/94 as opcode-1 — a structural sanity on the merge model.
#[test]
fn oracle_group40_structure() {
    let body = decompress_raw_group(GROUP40_DAT).expect("decompress group 40");
    let files = unpack_single_stripe(&body, &roster(GROUP40_META));
    assert_eq!(files.len(), 80, "group 40 file count");
    for donor_only in [88u32, 89] {
        assert_eq!(
            files[&donor_only][0], 2,
            "donor table {donor_only} should be opcode 2 (verbatim)"
        );
    }
    for reencoded in REENCODE_TABLES {
        assert_eq!(
            files[&reencoded][0], 1,
            "re-encoded table {reencoded} should be opcode 1"
        );
    }
}

// ── Full transcode (gated on the donor/base caches) ──────────────────────────

/// Crate-relative donor semantic dir (`dbtables.json` / `dbrows.json`).
fn donor_semantic() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/rs3-cache/948-all/config")
}
fn donor_raw() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/rs3-cache/948-all/raw-flat")
}
fn base_raw() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/rs3-cache/910-all/raw-flat")
}

/// When the donor + base caches are present, the FULL transcode reproduces the
/// oracle's decompressed group-40 + index-94 bodies AND metadata byte-for-byte.
/// Skipped (with a notice) when the local caches are absent, mirroring the
/// crate's other cache-dependent suites.
#[test]
fn full_transcode_reproduces_oracle_bodies() {
    let semantic = donor_semantic();
    let donor = donor_raw();
    let base = base_raw();
    let inputs_present = semantic.join("dbtables.json").is_file()
        && semantic.join("dbrows.json").is_file()
        && donor.join("2/40.dat").is_file()
        && base.join("2/40.dat").is_file();
    if !inputs_present {
        eprintln!(
            "skipping full_transcode_reproduces_oracle_bodies: donor/base caches not present \
             under {}",
            semantic.display()
        );
        return;
    }

    let inputs = TranscodeInputs::load(&semantic).expect("load donor semantic json");
    let out = transcode_db_groups(&inputs, &base, &donor).expect("transcode db groups");

    let oracle_g40 = decompress_raw_group(GROUP40_DAT).expect("decompress oracle group 40");
    assert_eq!(
        out.group40_body,
        oracle_g40,
        "transcoded group-40 body != oracle ({} vs {} bytes)",
        out.group40_body.len(),
        oracle_g40.len()
    );
    assert_eq!(
        out.group40_metadata.trim_end(),
        GROUP40_META.trim_end(),
        "transcoded group-40 metadata != oracle"
    );

    let oracle_idx = decompress_raw_group(INDEX94_DAT).expect("decompress oracle index 94");
    assert_eq!(
        out.index94_body,
        oracle_idx,
        "transcoded index-94 body != oracle ({} vs {} bytes)",
        out.index94_body.len(),
        oracle_idx.len()
    );
    assert_eq!(
        out.index94_metadata.trim_end(),
        INDEX94_META.trim_end(),
        "transcoded index-94 metadata != oracle"
    );
}
