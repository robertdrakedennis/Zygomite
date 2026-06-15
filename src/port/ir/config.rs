//! The typed config intermediate representation (plan §4.3).
//!
//! A build-neutral, typed model of the config records a 948→910 port touches:
//! [`DbTable`] (DBTABLETYPE schema), [`DbRow`] (DBROWTYPE), [`DbTableIndex`]
//! (the archive-49 `db_find` index), and the field-id *packing* the
//! [`crate::port::book::BuildDescriptor`] owns — the SAME packing model the CS2
//! IR's `dbfield_repack` uses (`DbFieldPacking`), so a field-id repack is
//! "two builds' encodings of one ref" here too (plan §4.3 / §6 `IdPackingDiff`).
//!
//! Most of the decode + encode already exists: the crate's [`crate::config`]
//! decodes a schema to a [`crate::config::DbTableEntry`], and
//! [`crate::config_transcode`] re-encodes it as a 910 opcode-1 schema and builds
//! the `BaseVarType`-serial index. This module LIFTS those into typed IR records
//! the port layer's config driver operates on, so `config transcode` /
//! `config port` share one model instead of re-deriving it.
//!
//! # Byte-exactness contract
//! The committed config `.dat`s (`relic-system-948/config/40-948.dat`, …) carry a
//! NODE-zlib gzip stream that is not reproducible across zlib implementations, so
//! the regression contract is on the DECOMPRESSED group BODY (the file chunks +
//! footer the client actually decodes) — exactly as the existing config-transcode
//! oracle asserts. The config IR's encoders are the byte-stable layer; the gzip
//! container is produced by the live-write path only.

use std::collections::BTreeMap;

use crate::config::{DbTableEntry, ScalarValue};
use crate::error::Result;
use crate::port::book::BuildDescriptor;

/// One DBTABLETYPE schema as typed IR: the table id and its columns. A typed lift
/// of [`crate::config::DbTableEntry`] (build-neutral — the schema layout is the
/// same in the donor opcode-2 and the 910 opcode-1 forms; only the wire opcode
/// differs, which the encoder owns).
#[derive(Clone, Debug)]
pub struct DbTable {
    /// The table id.
    pub id: u32,
    /// The columns, in column-index order.
    pub columns: Vec<DbColumn>,
}

/// One column of a [`DbTable`]: its index, the tuple types, and any default
/// tuples (each a flat list of typed scalars classified by the tuple types).
#[derive(Clone, Debug)]
pub struct DbColumn {
    /// The column index.
    pub column: u8,
    /// The tuple's `ScriptVarType` ids.
    pub tuple_types: Vec<u16>,
    /// Default tuples (one inner Vec per default, each `tuple_types.len()` long).
    pub defaults: Vec<Vec<ScalarValue>>,
}

impl DbTable {
    /// Lift a decoded [`DbTableEntry`] (any input opcode) into typed IR.
    #[must_use]
    pub fn from_entry(entry: &DbTableEntry) -> Self {
        Self {
            id: entry.id,
            columns: entry
                .columns
                .iter()
                .map(|c| DbColumn {
                    column: c.column,
                    tuple_types: c.tuple_types.clone(),
                    defaults: c.defaults.clone(),
                })
                .collect(),
        }
    }

    /// Lower the IR table back to a [`DbTableEntry`] (the form the config encoder
    /// consumes). The inverse of [`Self::from_entry`].
    #[must_use]
    pub fn to_entry(&self) -> DbTableEntry {
        DbTableEntry {
            id: self.id,
            columns: self
                .columns
                .iter()
                .map(|c| crate::config::DbTableColumn {
                    column: c.column,
                    tuple_types: c.tuple_types.clone(),
                    defaults: c.defaults.clone(),
                })
                .collect(),
        }
    }

    /// Encode the table as a TARGET-build DBTABLETYPE schema (today 910 opcode 1),
    /// folding [`crate::config_transcode::encode_dbtable_schema`]. The `target`
    /// descriptor selects the wire form; the IR is opcode-agnostic.
    pub fn encode(&self, _target: &BuildDescriptor) -> Result<Vec<u8>> {
        crate::config_transcode::encode_dbtable_schema(&self.to_entry())
    }
}

/// A DBTABLEINDEX (`db_find` index) as typed IR: a `key → [row ids]` map for one
/// integer-keyed index file (the 910 `BaseVarType`-serial form). Lifts the shape
/// [`crate::config_transcode::encode_int_index`] consumes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbTableIndex {
    /// `key → ascending row ids`.
    pub entries: BTreeMap<i32, Vec<u32>>,
}

impl DbTableIndex {
    /// A master index that maps key 0 → every `row` in ascending order.
    #[must_use]
    pub fn master(rows: impl IntoIterator<Item = u32>) -> Self {
        let mut sorted: Vec<u32> = rows.into_iter().collect();
        sorted.sort_unstable();
        let mut entries = BTreeMap::new();
        entries.insert(0, sorted);
        Self { entries }
    }

    /// Add a `(key, row)` membership (keeps each key's rows ascending + unique).
    pub fn insert(&mut self, key: i32, row: u32) {
        let rows = self.entries.entry(key).or_default();
        if let Err(pos) = rows.binary_search(&row) {
            rows.insert(pos, row);
        }
    }

    /// Encode the index in the target build's serial form (today 910
    /// `BaseVarType.INTEGER`), folding [`crate::config_transcode::encode_int_index`].
    #[must_use]
    pub fn encode(&self, _target: &BuildDescriptor) -> Vec<u8> {
        crate::config_transcode::encode_int_index(&self.entries)
    }
}

/// A db-field reference as a packed int and its `{table, column, tuple}` triple,
/// re-packed through a target descriptor — the config-side use of the SAME
/// `DbFieldPacking` the CS2 IR uses (plan §4.3: "share the packing model").
#[must_use]
pub fn repack_db_field(packed: i32, source: &BuildDescriptor, target: &BuildDescriptor) -> i32 {
    let field = source.decode_db_field(packed);
    target.encode_db_field(&field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn descriptor(build: u32) -> BuildDescriptor {
        BuildDescriptor::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"), build)
            .expect("load descriptor")
    }

    #[test]
    fn db_field_repack_matches_shift_for_ritual_fields() {
        let d948 = descriptor(948);
        let d910 = descriptor(910);
        // The ritual db-field constants repack to v>>4 via the shared packing.
        for v in [962_611, 958_480, 966_816] {
            assert_eq!(repack_db_field(v, &d948, &d910), v >> 4);
        }
    }

    #[test]
    fn index_master_and_insert_keep_rows_sorted() {
        let mut idx = DbTableIndex::master([7659, 7189, 7660]);
        assert_eq!(idx.entries[&0], vec![7189, 7659, 7660]);
        idx.insert(526, 7585);
        idx.insert(526, 7584);
        idx.insert(526, 7585); // duplicate ignored
        assert_eq!(idx.entries[&526], vec![7584, 7585]);
    }

    #[test]
    fn table_round_trips_entry_to_ir_and_back() {
        let entry = DbTableEntry {
            id: 235,
            columns: vec![crate::config::DbTableColumn {
                column: 3,
                tuple_types: vec![0],
                defaults: vec![],
            }],
        };
        let ir = DbTable::from_entry(&entry);
        let back = ir.to_entry();
        assert_eq!(back.id, entry.id);
        assert_eq!(back.columns.len(), 1);
        assert_eq!(back.columns[0].column, 3);
    }
}
