//! `config transcode` — re-encode a donor config group from its wire format to
//! the base client's. The first transcoder ports the relic system's CLIENT-side
//! db artifacts (the only ones the 910 client mis-decodes):
//!
//!   * DBTABLETYPE (Config archive 2, group 40): the donor schemas use opcode 2,
//!     which the 910 `DBTableType.decode` does not know (it only reads opcode 1),
//!     leaving `columnTypes` null → `db_getfield` NPE. We re-encode tables
//!     90/92/94 as 910 opcode-1 schemas and merge them over the 910 base group
//!     (donor server-only tables 88/89 ride along verbatim).
//!   * DbTableIndex (archive 49, group 94): the donor index carries an
//!     `0xff`-prefixed layout; the 910 `DbTableIndex.decode` reads a BaseVarType
//!     serial id first → `field9181` null NPE in `db_find`. We re-encode the
//!     master / col-0 / col-9 indices in the 910 `BaseVarType` serial form.
//!
//! SOURCE OF TRUTH: `server/cache-patches/relic-system-948/build-relic-db-910.ts`
//! (`encodeSchema` opcode-1 / `encodeIntIndex`). The encoders here are a faithful
//! port; the oracle test re-encodes from the same donor semantic JSON and
//! reproduces the committed `config/40-948.dat` + `dbtableindex/94.dat`
//! DECOMPRESSED payloads byte-for-byte (the gzip stream itself is Node's and not
//! reproducible across zlib implementations, exactly as the font oracle notes).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cache_bail as bail;
use crate::config::{DbTypeBase, dbtype_base};
use crate::error::{Context, Result};
use crate::js5::{ArchiveIndex, decompress, unpack_group};
use crate::packet::ByteWriter;

/// Default donor semantic config dir (crate-relative). Holds `dbtables.json` /
/// `dbrows.json` for the 948 donor.
pub const DEFAULT_DONOR_SEMANTIC: &str = "../../cache/rs3-cache/948-all/config";
/// Default donor raw-flat root (for the server-only tables 88/89 ride-along).
pub const DEFAULT_DONOR_RAW: &str = "../../cache/rs3-cache/948-all/raw-flat";
/// Default base (910) raw-flat root.
pub const DEFAULT_BASE_RAW: &str = "../../cache/rs3-cache/910-all/raw-flat";

/// Config archive id.
pub const CONFIG_ARCHIVE: u32 = 2;
/// Archive-set (index) id.
pub const ARCHIVE_SET: u32 = 255;
/// DBTABLETYPE group within the config archive.
pub const DBTABLETYPE_GROUP: u32 = 40;
/// DbTableIndex archive id.
pub const DBTABLEINDEX_ARCHIVE: u32 = 49;

/// Relic tables that get re-encoded as 910 opcode-1 schemas (mirrors the TS
/// `[RESEARCH_TABLE, MYSTERY_TABLE, RELIC_TABLE]` order — group merge is
/// roster-sorted afterwards so the order does not affect output).
pub const REENCODE_TABLES: [u32; 3] = [90, 92, 94];
/// Donor server-only tables spliced verbatim (opcode-2, server-read only).
pub const SERVER_ONLY_TABLES: [u32; 2] = [88, 89];
/// The relic table whose DbTableIndex is rebuilt (archive 49, group 94).
pub const RELIC_TABLE: u32 = 94;

// ── Semantic JSON (donor `dbtables.json` / `dbrows.json`) ────────────────────

/// A value cell in the donor semantic JSON (`{kind: "Int"|"Str", value}`). The
/// `kind` tag documents the wire shape; the encoder classifies by the column's
/// `ScriptVarType` instead, so it reads `value` directly.
#[derive(Clone, Debug, Deserialize)]
struct SemanticValue {
    #[allow(dead_code)]
    kind: String,
    value: serde_json::Value,
}

/// A donor table column with its tuple types and optional defaults/rows.
#[derive(Clone, Debug, Deserialize)]
struct SemanticColumn {
    column: u8,
    tuple_types: Vec<u16>,
    #[serde(default)]
    defaults: Option<Vec<Vec<SemanticValue>>>,
    #[serde(default)]
    rows: Option<Vec<Vec<SemanticValue>>>,
}

/// A donor `dbtables.json` table entry.
#[derive(Clone, Debug, Deserialize)]
struct SemanticTable {
    id: u32,
    columns: Vec<SemanticColumn>,
}

/// A donor `dbrows.json` row entry.
#[derive(Clone, Debug, Deserialize)]
struct SemanticRow {
    id: u32,
    table: u32,
    columns: Vec<SemanticColumn>,
}

// ── Schema encoder (910 DBTABLETYPE opcode 1) ────────────────────────────────

/// One column's schema as the opcode-1 encoder needs it: column index, tuple
/// types, and each default tuple as a flat list of scalars. This is the shared
/// shape both the donor semantic JSON ([`SemanticTable`]) and a decoded
/// [`crate::config::DbTableEntry`] are projected onto.
struct SchemaColumn<'a> {
    column: u8,
    tuple_types: &'a [u16],
    defaults: Vec<Vec<Scalar<'a>>>,
}

/// A scalar to encode (already classified by the column's tuple type at write).
enum Scalar<'a> {
    Int(i64),
    Long(i64),
    Str(&'a str),
}

/// Encode a 910 opcode-1 DBTABLETYPE schema from generic columns. Faithful port
/// of `encodeSchema` in `build-relic-db-910.ts`. `table_id` is used only for
/// error messages.
fn encode_schema_columns(table_id: u32, columns: &[SchemaColumn<'_>]) -> Result<Vec<u8>> {
    let mut buf = ByteWriter::new();
    buf.p1(1);
    let max_column = columns
        .iter()
        .map(|c| c.column)
        .max()
        .with_context(|| format!("table {table_id} has no columns"))?;
    buf.p1(max_column.wrapping_add(1));

    let mut sorted: Vec<&SchemaColumn<'_>> = columns.iter().collect();
    sorted.sort_by_key(|c| c.column);
    for column in sorted {
        let has_defaults = !column.defaults.is_empty();
        buf.p1(column.column | if has_defaults { 0x80 } else { 0 });
        let tuple_len = u8::try_from(column.tuple_types.len()).with_context(|| {
            format!(
                "table {table_id} col {} tuple arity {} exceeds u8",
                column.column,
                column.tuple_types.len()
            )
        })?;
        buf.p1(tuple_len);
        for type_id in column.tuple_types {
            buf.psmart1or2(*type_id)?;
        }
        if has_defaults {
            buf.psmart1or2(u16::try_from(column.defaults.len()).with_context(|| {
                format!(
                    "table {table_id} col {} default count overflow",
                    column.column
                )
            })?)?;
            for field in &column.defaults {
                if field.len() != column.tuple_types.len() {
                    bail!(
                        "table {table_id} col {}: default tuple arity mismatch ({} vs {})",
                        column.column,
                        field.len(),
                        column.tuple_types.len()
                    );
                }
                for scalar in field {
                    encode_scalar(&mut buf, scalar)?;
                }
            }
        }
    }
    buf.p1(255);
    buf.p1(0);
    Ok(buf.data)
}

/// Encode one already-classified scalar (mirrors the TS `type.baseType.encode`).
fn encode_scalar(buf: &mut ByteWriter, scalar: &Scalar<'_>) -> Result<()> {
    match scalar {
        Scalar::Int(n) => {
            buf.p4s(i32::try_from(*n).with_context(|| format!("int value {n} out of i32 range"))?);
        }
        Scalar::Long(n) => buf.p8s(*n),
        Scalar::Str(s) => buf.pjstr(s)?,
    }
    Ok(())
}

/// Lift a donor [`SemanticTable`] (parsed JSON) into the build-neutral config IR
/// [`crate::port::ir::config::DbTable`]. Classifies each default value by its
/// column type into a typed [`crate::config::ScalarValue`] (the same
/// classification [`semantic_scalar`] / the wire encoder use), so encoding the IR
/// reproduces [`encode_schema`] byte-for-byte (plan §9 step 6 "lift to IR").
fn semantic_table_to_ir(table: &SemanticTable) -> Result<crate::port::ir::config::DbTable> {
    use crate::config::ScalarValue;
    let mut columns = Vec::with_capacity(table.columns.len());
    for column in &table.columns {
        let defaults_in = column.defaults.as_deref().unwrap_or(&[]);
        let mut defaults = Vec::with_capacity(defaults_in.len());
        for field in defaults_in {
            if field.len() != column.tuple_types.len() {
                bail!(
                    "table {} col {}: default tuple arity mismatch ({} vs {})",
                    table.id,
                    column.column,
                    field.len(),
                    column.tuple_types.len()
                );
            }
            let mut tuple = Vec::with_capacity(field.len());
            for (i, type_id) in column.tuple_types.iter().enumerate() {
                tuple.push(match dbtype_base(*type_id) {
                    DbTypeBase::Int => ScalarValue::Int(
                        i32::try_from(value_as_i64(&field[i])?).with_context(|| {
                            format!(
                                "table {} col {} int value out of i32 range",
                                table.id, column.column
                            )
                        })?,
                    ),
                    DbTypeBase::Long => ScalarValue::Long(value_as_i64(&field[i])?),
                    DbTypeBase::String => ScalarValue::Str(value_as_str(&field[i])?.to_string()),
                });
            }
            defaults.push(tuple);
        }
        columns.push(crate::port::ir::config::DbColumn {
            column: column.column,
            tuple_types: column.tuple_types.clone(),
            defaults,
        });
    }
    Ok(crate::port::ir::config::DbTable {
        id: table.id,
        columns,
    })
}

/// Project a donor [`SemanticTable`] onto [`SchemaColumn`]s and encode it as a
/// 910 opcode-1 schema. Faithful port of `encodeSchema`.
fn encode_schema(table: &SemanticTable) -> Result<Vec<u8>> {
    let mut columns = Vec::with_capacity(table.columns.len());
    for column in &table.columns {
        let defaults_in = column.defaults.as_deref().unwrap_or(&[]);
        let mut defaults = Vec::with_capacity(defaults_in.len());
        for field in defaults_in {
            if field.len() != column.tuple_types.len() {
                bail!(
                    "table {} col {}: default tuple arity mismatch ({} vs {})",
                    table.id,
                    column.column,
                    field.len(),
                    column.tuple_types.len()
                );
            }
            let mut tuple = Vec::with_capacity(field.len());
            for (i, type_id) in column.tuple_types.iter().enumerate() {
                tuple.push(semantic_scalar(*type_id, &field[i])?);
            }
            defaults.push(tuple);
        }
        columns.push(SchemaColumn {
            column: column.column,
            tuple_types: &column.tuple_types,
            defaults,
        });
    }
    encode_schema_columns(table.id, &columns)
}

/// Classify a donor semantic value by its column type into a [`Scalar`].
fn semantic_scalar(type_id: u16, value: &SemanticValue) -> Result<Scalar<'_>> {
    Ok(match dbtype_base(type_id) {
        DbTypeBase::Int => Scalar::Int(value_as_i64(value)?),
        DbTypeBase::Long => Scalar::Long(value_as_i64(value)?),
        DbTypeBase::String => Scalar::Str(value_as_str(value)?),
    })
}

/// Re-encode a DECODED dbtable (any input opcode) as a 910 opcode-1 schema. This
/// is the opcode-2 → opcode-1 transcode at the heart of the DBTABLETYPE port:
/// the crate's [`crate::config::parse_dbtable`] yields the same column/default
/// shape for opcode 1 and opcode 2, so re-encoding here is opcode-agnostic.
pub fn encode_dbtable_schema(entry: &crate::config::DbTableEntry) -> Result<Vec<u8>> {
    use crate::config::ScalarValue;
    let mut columns = Vec::with_capacity(entry.columns.len());
    for column in &entry.columns {
        let mut defaults = Vec::with_capacity(column.defaults.len());
        for tuple in &column.defaults {
            let mut row = Vec::with_capacity(tuple.len());
            for scalar in tuple {
                row.push(match scalar {
                    ScalarValue::Int(n) => Scalar::Int(i64::from(*n)),
                    ScalarValue::Long(n) => Scalar::Long(*n),
                    ScalarValue::Str(s) => Scalar::Str(s.as_str()),
                });
            }
            defaults.push(row);
        }
        columns.push(SchemaColumn {
            column: column.column,
            tuple_types: &column.tuple_types,
            defaults,
        });
    }
    encode_schema_columns(entry.id, &columns)
}

/// Extract an integer from a semantic value (numbers, or numeric JSON).
fn value_as_i64(value: &SemanticValue) -> Result<i64> {
    value
        .value
        .as_i64()
        .with_context(|| format!("expected integer semantic value, got {:?}", value.value))
}

/// Extract a string from a semantic value.
fn value_as_str(value: &SemanticValue) -> Result<&str> {
    value
        .value
        .as_str()
        .with_context(|| format!("expected string semantic value, got {:?}", value.value))
}

// ── Index encoder (910 DbTableIndex, BaseVarType serial) ─────────────────────

/// Encode an integer-keyed DbTableIndex in the 910 `BaseVarType` serial form.
/// Faithful port of `encodeIntIndex` in `build-relic-db-910.ts`.
pub fn encode_int_index(entries: &BTreeMap<i32, Vec<u32>>) -> Vec<u8> {
    let mut buf = ByteWriter::new();
    buf.p1(0); // BaseVarType.INTEGER serial id
    buf.pvarint2(entries.len() as u32);
    for (key, row_ids) in entries {
        buf.p4s(*key);
        buf.pvarint2(row_ids.len() as u32);
        for row_id in row_ids {
            buf.pvarint2(*row_id);
        }
    }
    buf.data
}

// ── Raw-group helpers (split / pack / compress) ──────────────────────────────

/// Pack per-file payloads into a single-stripe (marker = 1) JS5 group body:
/// every file's bytes concatenated, then one int32 size-delta per file, then the
/// stripe-count marker byte. Inverse of [`unpack_group`] for the 1-stripe case
/// (which is what the relic builder's `buildRawGroup(..., 1)` emits).
fn pack_group_files(files: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in files {
        out.extend_from_slice(f);
    }
    let mut prev = 0_i32;
    for f in files {
        let len = f.len() as i32;
        out.extend_from_slice(&(len - prev).to_be_bytes());
        prev = len;
    }
    out.push(1); // single stripe
    out
}

/// Wrap a group body in a gzip JS5 container (compression 2) plus a 2-byte
/// version trailer, matching `buildRawGroup`. NOTE: the gzip byte stream is not
/// reproducible across zlib implementations, so the regression contract is on
/// the DECOMPRESSED body — see the oracle test.
fn build_raw_group(body: &[u8], version: u16) -> Result<Vec<u8>> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write as _;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(body)
        .context("gzip-compress group body")?;
    let gz = encoder.finish().context("finish gzip stream")?;

    let mut out = Vec::with_capacity(9 + gz.len() + 2);
    out.push(2); // gzip compression
    out.extend_from_slice(&(gz.len() as u32).to_be_bytes()); // compressed length (excl. ulen field)
    out.extend_from_slice(&(body.len() as u32).to_be_bytes()); // uncompressed length
    out.extend_from_slice(&gz);
    out.extend_from_slice(&version.to_be_bytes()); // version trailer
    Ok(out)
}

/// Read and decode the archive index for `archive` from a raw-flat root
/// (`<root>/255/<archive>.dat`).
fn read_index(raw_root: &Path, archive: u32) -> Result<ArchiveIndex> {
    let path = raw_root
        .join(ARCHIVE_SET.to_string())
        .join(format!("{archive}.dat"));
    let bytes =
        std::fs::read(&path).with_context(|| format!("read archive index {}", path.display()))?;
    let decompressed = decompress(&bytes)
        .with_context(|| format!("decompress archive index {}", path.display()))?;
    ArchiveIndex::decode(&decompressed)
        .with_context(|| format!("decode archive index {}", path.display()))
}

/// Read group `group` from `<root>/<archive>/<group>.dat` and unpack it into its
/// per-file map using `index`.
fn read_group_files(
    raw_root: &Path,
    index: &ArchiveIndex,
    archive: u32,
    group: u32,
) -> Result<BTreeMap<u32, Vec<u8>>> {
    let path = raw_root
        .join(archive.to_string())
        .join(format!("{group}.dat"));
    let bytes = std::fs::read(&path).with_context(|| format!("read group {}", path.display()))?;
    // Strip the 2-byte version trailer before decompressing (raw groups carry it).
    let container = &bytes[..bytes.len().saturating_sub(2)];
    unpack_group(index, group, container)
        .with_context(|| format!("unpack group {}", path.display()))
}

// ── Group metadata (matches the *.metadata.json oracle) ──────────────────────

/// Per-group metadata the overlay applier consumes (roster + capacity). Field
/// names and the pretty-print shape match `build-relic-db-910.ts`'s emitted
/// `*.metadata.json` so it reproduces those byte-for-byte.
#[derive(Debug)]
struct GroupMetadata {
    group_size: usize,
    group_capacity: u32,
    file_ids: Vec<u32>,
}

impl GroupMetadata {
    fn new(roster: &[u32]) -> Result<Self> {
        let max = roster.iter().copied().max().context("empty group roster")?;
        Ok(Self {
            group_size: roster.len(),
            group_capacity: max + 1,
            file_ids: roster.to_vec(),
        })
    }

    /// Render in the exact `JSON.stringify(meta, null, 2)` shape the TS emits.
    fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        let _ = writeln!(out, "  \"groupSize\": {},", self.group_size);
        let _ = writeln!(out, "  \"groupCapacity\": {},", self.group_capacity);
        out.push_str("  \"fileIds\": [\n");
        for (i, id) in self.file_ids.iter().enumerate() {
            let comma = if i + 1 < self.file_ids.len() { "," } else { "" };
            let _ = writeln!(out, "    {id}{comma}");
        }
        out.push_str("  ]\n");
        out.push('}');
        out
    }
}

// ── Transcode orchestration ──────────────────────────────────────────────────

/// The semantic inputs needed to transcode group 40 / index 94.
pub struct TranscodeInputs {
    /// Donor `dbtables.json` (parsed).
    tables: Vec<SemanticTable>,
    /// Donor `dbrows.json` (parsed) — only needed for the DbTableIndex rebuild.
    rows: Vec<SemanticRow>,
}

impl TranscodeInputs {
    /// Load the donor semantic JSON from a config dir.
    pub fn load(donor_semantic: &Path) -> Result<Self> {
        let tables = Self::read_json(&donor_semantic.join("dbtables.json"))?;
        let rows = Self::read_json(&donor_semantic.join("dbrows.json"))?;
        Ok(Self { tables, rows })
    }

    fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read semantic json {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parse semantic json {}", path.display()))
    }

    fn table(&self, id: u32) -> Result<&SemanticTable> {
        self.tables
            .iter()
            .find(|t| t.id == id)
            .with_context(|| format!("table {id} missing from donor dbtables.json"))
    }
}

/// The transcoded group bodies (DECOMPRESSED) and their metadata — the
/// regression-stable artifacts. The `.dat` is produced from `group40_body` via
/// [`build_raw_group`] for the live write, but the test asserts on the body.
pub struct TranscodedDbGroup {
    /// Config group-40 DECOMPRESSED body (file chunks + footer).
    pub group40_body: Vec<u8>,
    /// Config group-40 roster (sorted file ids).
    pub group40_roster: Vec<u32>,
    /// Config group-40 metadata JSON text.
    pub group40_metadata: String,
    /// DbTableIndex group-94 DECOMPRESSED body.
    pub index94_body: Vec<u8>,
    /// DbTableIndex group-94 roster.
    pub index94_roster: Vec<u32>,
    /// DbTableIndex group-94 metadata JSON text.
    pub index94_metadata: String,
}

/// Build the re-encoded Config group 40 body: 910 base files + donor 88/89
/// verbatim + tables 90/92/94 re-encoded as opcode-1 schemas. Port of section 1
/// of `build-relic-db-910.ts`.
fn build_group40_body(
    inputs: &TranscodeInputs,
    base_raw: &Path,
    donor_raw: &Path,
) -> Result<(Vec<u8>, Vec<u32>)> {
    let base_index = read_index(base_raw, CONFIG_ARCHIVE)?;
    let donor_index = read_index(donor_raw, CONFIG_ARCHIVE)?;
    let base40 = read_group_files(base_raw, &base_index, CONFIG_ARCHIVE, DBTABLETYPE_GROUP)?;
    let donor40 = read_group_files(donor_raw, &donor_index, CONFIG_ARCHIVE, DBTABLETYPE_GROUP)?;

    let mut merged: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    if base40.is_empty() {
        bail!("910 base config group 40 roster is empty");
    }
    for (id, file) in base40 {
        merged.insert(id, file);
    }
    for id in SERVER_ONLY_TABLES {
        if merged.contains_key(&id) {
            bail!("table {id} unexpectedly present in 910 base group 40");
        }
        let donor_file = donor40
            .get(&id)
            .with_context(|| format!("donor table {id} missing from 948 group 40"))?;
        merged.insert(id, donor_file.clone());
    }
    for table_id in REENCODE_TABLES {
        let table = inputs.table(table_id)?;
        merged.insert(table_id, encode_schema(table)?);
    }

    let roster: Vec<u32> = merged.keys().copied().collect();
    let ordered: Vec<Vec<u8>> = roster.iter().map(|id| merged[id].clone()).collect();
    Ok((pack_group_files(&ordered), roster))
}

/// Build the DbTableIndex group-94 body (files 0 / 1 / 10). Port of section 2 of
/// `build-relic-db-910.ts`.
fn build_index94_body(inputs: &TranscodeInputs) -> (Vec<u8>, Vec<u32>) {
    let relic_rows: Vec<&SemanticRow> = inputs
        .rows
        .iter()
        .filter(|r| r.table == RELIC_TABLE)
        .collect();

    // Mirror the TS `col`: read column `column`'s first row's first tuple cell,
    // returning it only when it is a number (string cells yield `None`).
    let col = |row: &SemanticRow, column: u8| -> Option<i32> {
        let entry = row.columns.iter().find(|c| c.column == column)?;
        let value = entry.rows.as_ref()?.first()?.first()?;
        value.value.as_i64().and_then(|v| i32::try_from(v).ok())
    };

    let mut master_rows: Vec<u32> = relic_rows.iter().map(|r| r.id).collect();
    master_rows.sort_unstable();
    let mut master = BTreeMap::new();
    master.insert(0_i32, master_rows);

    let mut by_power_id: BTreeMap<i32, Vec<u32>> = BTreeMap::new();
    let mut by_relic_obj: BTreeMap<i32, Vec<u32>> = BTreeMap::new();
    for row in &relic_rows {
        if let Some(power_id) = col(row, 0) {
            by_power_id.entry(power_id).or_default().push(row.id);
        }
        if let Some(relic_obj) = col(row, 9) {
            by_relic_obj.entry(relic_obj).or_default().push(row.id);
        }
    }

    let files = vec![
        encode_int_index(&master),
        encode_int_index(&by_power_id),
        encode_int_index(&by_relic_obj),
    ];
    let roster = vec![0_u32, 1, 10];
    (pack_group_files(&files), roster)
}

/// Transcode the relic DB groups from donor semantic JSON. The byte-stable
/// outputs are the DECOMPRESSED bodies + the metadata JSON.
pub fn transcode_db_groups(
    inputs: &TranscodeInputs,
    base_raw: &Path,
    donor_raw: &Path,
) -> Result<TranscodedDbGroup> {
    let (group40_body, group40_roster) = build_group40_body(inputs, base_raw, donor_raw)?;
    let group40_metadata = GroupMetadata::new(&group40_roster)?.to_json();
    let (index94_body, index94_roster) = build_index94_body(inputs);
    let index94_metadata = GroupMetadata::new(&index94_roster)?.to_json();
    Ok(TranscodedDbGroup {
        group40_body,
        group40_roster,
        group40_metadata,
        index94_body,
        index94_roster,
        index94_metadata,
    })
}

// ── IR-routed transcode (plan §9 step 6) ─────────────────────────────────────

/// Build the re-encoded Config group 40 body through the config IR (plan §9 step
/// 6): the relic client-read tables 90/92/94 are lifted donor `SemanticTable` →
/// [`crate::port::ir::config::DbTable`] → `.encode(target)`; everything else
/// (base files, server-only 88/89) is identical to [`build_group40_body`]. Proven
/// byte-identical to the JSON path by the config-port oracle.
fn build_group40_body_ir(
    inputs: &TranscodeInputs,
    base_raw: &Path,
    donor_raw: &Path,
    target: &crate::port::book::BuildDescriptor,
) -> Result<(Vec<u8>, Vec<u32>)> {
    let base_index = read_index(base_raw, CONFIG_ARCHIVE)?;
    let donor_index = read_index(donor_raw, CONFIG_ARCHIVE)?;
    let base40 = read_group_files(base_raw, &base_index, CONFIG_ARCHIVE, DBTABLETYPE_GROUP)?;
    let donor40 = read_group_files(donor_raw, &donor_index, CONFIG_ARCHIVE, DBTABLETYPE_GROUP)?;

    let mut merged: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    if base40.is_empty() {
        bail!("910 base config group 40 roster is empty");
    }
    for (id, file) in base40 {
        merged.insert(id, file);
    }
    for id in SERVER_ONLY_TABLES {
        if merged.contains_key(&id) {
            bail!("table {id} unexpectedly present in 910 base group 40");
        }
        let donor_file = donor40
            .get(&id)
            .with_context(|| format!("donor table {id} missing from 948 group 40"))?;
        merged.insert(id, donor_file.clone());
    }
    for table_id in REENCODE_TABLES {
        let table = inputs.table(table_id)?;
        // The IR re-encode: lift the donor SemanticTable → DbTable → encode(target).
        let ir = semantic_table_to_ir(table)?;
        merged.insert(table_id, ir.encode(target)?);
    }

    let roster: Vec<u32> = merged.keys().copied().collect();
    let ordered: Vec<Vec<u8>> = roster.iter().map(|id| merged[id].clone()).collect();
    Ok((pack_group_files(&ordered), roster))
}

/// Build the DbTableIndex group-94 body through the config IR (plan §9 step 6):
/// the master / col-0 / col-9 indices as [`crate::port::ir::config::DbTableIndex`]
/// records encoded via the target serial form. Byte-identical to
/// [`build_index94_body`].
fn build_index94_body_ir(
    inputs: &TranscodeInputs,
    target: &crate::port::book::BuildDescriptor,
) -> (Vec<u8>, Vec<u32>) {
    use crate::port::ir::config::DbTableIndex;
    let relic_rows: Vec<&SemanticRow> = inputs
        .rows
        .iter()
        .filter(|r| r.table == RELIC_TABLE)
        .collect();

    let col = |row: &SemanticRow, column: u8| -> Option<i32> {
        let entry = row.columns.iter().find(|c| c.column == column)?;
        let value = entry.rows.as_ref()?.first()?.first()?;
        value.value.as_i64().and_then(|v| i32::try_from(v).ok())
    };

    // file 0 = master (key 0 → every relic row, ascending).
    let master = DbTableIndex::master(relic_rows.iter().map(|r| r.id));
    // file 1 = col-0 index; file 10 = col-9 index.
    let mut by_power_id = DbTableIndex::default();
    let mut by_relic_obj = DbTableIndex::default();
    for row in &relic_rows {
        if let Some(power_id) = col(row, 0) {
            by_power_id.insert(power_id, row.id);
        }
        if let Some(relic_obj) = col(row, 9) {
            by_relic_obj.insert(relic_obj, row.id);
        }
    }

    let files = vec![
        master.encode(target),
        by_power_id.encode(target),
        by_relic_obj.encode(target),
    ];
    let roster = vec![0_u32, 1, 10];
    (pack_group_files(&files), roster)
}

/// Transcode the relic DB groups through the config IR (the IR-routed equivalent
/// of [`transcode_db_groups`]). Produces the same byte-stable decompressed bodies
/// and metadata — proven by the config-port oracle. This is what `config port`
/// drives, and what `config transcode`'s `run` is routed through.
pub fn transcode_db_groups_ir(
    inputs: &TranscodeInputs,
    base_raw: &Path,
    donor_raw: &Path,
    target: &crate::port::book::BuildDescriptor,
) -> Result<TranscodedDbGroup> {
    let (group40_body, group40_roster) =
        build_group40_body_ir(inputs, base_raw, donor_raw, target)?;
    let group40_metadata = GroupMetadata::new(&group40_roster)?.to_json();
    let (index94_body, index94_roster) = build_index94_body_ir(inputs, target);
    let index94_metadata = GroupMetadata::new(&index94_roster)?.to_json();
    Ok(TranscodedDbGroup {
        group40_body,
        group40_roster,
        group40_metadata,
        index94_body,
        index94_roster,
        index94_metadata,
    })
}

/// Write a [`TranscodedDbGroup`] to an output dir: `config/40-948.dat(+metadata)`
/// and `dbtableindex/94.dat(+metadata)`. Shared by `config transcode` and
/// `config port` so both produce the same files. The gzip container is produced by
/// the live-write path only (not byte-reproducible across zlib implementations);
/// the byte-stable contract is on the decompressed body in [`TranscodedDbGroup`].
pub fn write_transcoded_db_group(out: &TranscodedDbGroup, dir: &Path) -> Result<()> {
    let config_dir = dir.join("config");
    let index_dir = dir.join("dbtableindex");
    std::fs::create_dir_all(&config_dir)
        .with_context(|| format!("create {}", config_dir.display()))?;
    std::fs::create_dir_all(&index_dir)
        .with_context(|| format!("create {}", index_dir.display()))?;

    let group40_dat = build_raw_group(&out.group40_body, 1)?;
    std::fs::write(config_dir.join("40-948.dat"), &group40_dat).context("write 40-948.dat")?;
    std::fs::write(
        config_dir.join("40-948.metadata.json"),
        &out.group40_metadata,
    )
    .context("write 40-948.metadata.json")?;

    let index94_dat = build_raw_group(&out.index94_body, 1)?;
    std::fs::write(index_dir.join("94.dat"), &index94_dat).context("write 94.dat")?;
    std::fs::write(index_dir.join("94.metadata.json"), &out.index94_metadata)
        .context("write 94.metadata.json")?;
    Ok(())
}

// ── CLI ──────────────────────────────────────────────────────────────────────

/// Options for [`run`].
pub struct TranscodeOptions<'a> {
    /// Config archive id (must be 2 for the DBTABLETYPE transcoder).
    pub archive: u32,
    /// Group id (must be 40 for the DBTABLETYPE transcoder).
    pub group: u32,
    /// Donor build (must be 948).
    pub from: u32,
    /// Target build (must be 910).
    pub to: u32,
    /// Donor semantic config dir (`dbtables.json` / `dbrows.json`).
    pub donor_semantic: &'a Path,
    /// Donor raw-flat root.
    pub donor_raw: &'a Path,
    /// Base (910) raw-flat root.
    pub base_raw: &'a Path,
    /// Optional output dir; when set, writes `<group>-948.dat(+metadata)` and the
    /// DbTableIndex `94.dat(+metadata)`. READ-ONLY caches; never writes the
    /// oracle dir.
    pub out_dir: Option<&'a Path>,
    /// Emit a JSON summary instead of the human report.
    pub json: bool,
}

/// Run `config transcode` — now a THIN ALIAS over the typed config port layer
/// ([`crate::port::config`]). Today only `--archive 2 --group 40 --from 948 --to
/// 910` is supported (the relic DBTABLETYPE + DbTableIndex chain); other
/// combinations error with a clear message.
///
/// The byte production is routed through the config IR ([`transcode_db_groups_ir`],
/// proven byte-identical to the legacy JSON [`transcode_db_groups`] by the
/// config-port oracle); the legacy JSON path remains for the equivalence oracle and
/// any in-process caller. The `-948.dat` output names + the report format are
/// preserved so existing tooling/docs that call `config transcode` are unchanged.
pub fn run(opts: &TranscodeOptions<'_>) -> Result<()> {
    if opts.from != 948 || opts.to != 910 {
        bail!(
            "config transcode currently supports only --from 948 --to 910 (got {} -> {})",
            opts.from,
            opts.to
        );
    }
    if opts.archive != CONFIG_ARCHIVE || opts.group != DBTABLETYPE_GROUP {
        bail!(
            "config transcode currently supports only --archive {CONFIG_ARCHIVE} --group {DBTABLETYPE_GROUP} (DBTABLETYPE); got archive {} group {}",
            opts.archive,
            opts.group
        );
    }

    let target =
        crate::port::book::BuildDescriptor::load(&crate::cs2::lint::default_data_dir(), opts.to)?;
    let inputs = TranscodeInputs::load(opts.donor_semantic)?;
    let out = transcode_db_groups_ir(&inputs, opts.base_raw, opts.donor_raw, &target)?;

    if let Some(dir) = opts.out_dir {
        write_transcoded_db_group(&out, dir)?;
    }

    if opts.json {
        let summary = serde_json::json!({
            "archive": opts.archive,
            "group": opts.group,
            "from": opts.from,
            "to": opts.to,
            "group40": {
                "roster": out.group40_roster,
                "body_len": out.group40_body.len(),
            },
            "index94": {
                "roster": out.index94_roster,
                "body_len": out.index94_body.len(),
            },
            "wrote": opts.out_dir.is_some(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).context("encode transcode summary")?
        );
    } else {
        print!("{}", render_human(opts, &out));
    }
    Ok(())
}

/// Human summary of a transcode run.
fn render_human(opts: &TranscodeOptions<'_>, out: &TranscodedDbGroup) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "config transcode — archive {} group {} ({} -> {})",
        opts.archive, opts.group, opts.from, opts.to
    );
    let _ = writeln!(
        s,
        "  group 40: {} files (donor {} verbatim, {} re-encoded), body {} bytes",
        out.group40_roster.len(),
        SERVER_ONLY_TABLES
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/"),
        REENCODE_TABLES
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/"),
        out.group40_body.len()
    );
    let _ = writeln!(
        s,
        "  index 94: files {} , body {} bytes",
        out.index94_roster
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/"),
        out.index94_body.len()
    );
    if opts.out_dir.is_some() {
        s.push_str("  wrote config/40-948.dat(+metadata) and dbtableindex/94.dat(+metadata)\n");
    }
    s
}

/// Convenience default paths for the CLI dispatch.
#[must_use]
pub fn default_donor_semantic() -> PathBuf {
    PathBuf::from(DEFAULT_DONOR_SEMANTIC)
}
/// Default donor raw-flat root.
#[must_use]
pub fn default_donor_raw() -> PathBuf {
    PathBuf::from(DEFAULT_DONOR_RAW)
}
/// Default base raw-flat root.
#[must_use]
pub fn default_base_raw() -> PathBuf {
    PathBuf::from(DEFAULT_BASE_RAW)
}
