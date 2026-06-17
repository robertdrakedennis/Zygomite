use super::ScalarValue;
use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct DbTableColumn {
    pub column: u8,
    pub tuple_types: Vec<u16>,
    pub defaults: Vec<Vec<ScalarValue>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DbTableEntry {
    pub id: u32,
    pub columns: Vec<DbTableColumn>,
}

pub fn parse_dbtable(id: u32, data: &[u8]) -> Result<DbTableEntry> {
    let mut packet = Packet::new(data);
    let mut columns = BTreeMap::<u8, DbTableColumn>::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("dbtable {id} did not consume full payload");
                }
                return Ok(DbTableEntry {
                    id,
                    columns: columns.into_values().collect(),
                });
            }
            1 => {
                let _column_count = packet.g1()?;
                let mut value = packet.g1()?;
                while value != u8::MAX {
                    let column = value & 127;
                    let has_default = (value & 128) != 0;
                    let tuple_length = usize::from(packet.g1()?);
                    let tuple_types = read_type_ids(&mut packet, tuple_length)?;
                    set_dbtable_tuple_types(id, column, &tuple_types, &mut columns)?;

                    if has_default {
                        let default_count = usize::from(packet.gsmart1or2()?);
                        for _ in 0..default_count {
                            let tuple = read_tuple(&mut packet, &tuple_types)?;
                            push_dbtable_default(column, tuple, &mut columns)?;
                        }
                    }

                    value = packet.g1()?;
                }
            }
            2 => {
                let size = usize::try_from(packet.g4s()?).context("negative dbtable block size")?;
                let start = packet.pos();
                let column_count = usize::from(packet.g1()?);
                let mut tuple_types: Vec<Option<Vec<u16>>> = vec![None; column_count];

                let mut column = packet.g1()?;
                while column != u8::MAX {
                    let column_idx = usize::from(column);
                    let tuples = tuple_types
                        .get_mut(column_idx)
                        .with_context(|| format!("dbtable {id} column index out of range"))?;

                    let mut op = packet.g1()?;
                    while op != 0 {
                        match op {
                            1 => {
                                let tuple_length = usize::from(packet.g1()?);
                                let types = read_type_ids(&mut packet, tuple_length)?;
                                set_dbtable_tuple_types(id, column, &types, &mut columns)?;
                                *tuples = Some(types);
                            }
                            2 => {
                                let defaults = usize::from(packet.gsmart1or2()?);
                                let types = tuples
                                    .as_ref()
                                    .with_context(|| format!("dbtable {id} missing column schema"))?
                                    .clone();
                                for _ in 0..defaults {
                                    let tuple = read_tuple(&mut packet, &types)?;
                                    push_dbtable_default(column, tuple, &mut columns)?;
                                }
                            }
                            value => bail!("invalid dbtable column op {value} in {id}"),
                        }
                        op = packet.g1()?;
                    }

                    column = packet.g1()?;
                }

                let consumed = packet.pos().saturating_sub(start);
                if consumed != size {
                    bail!("dbtable {id} size mismatch: consumed {consumed} expected {size}");
                }
            }
            opcode => bail!("unknown dbtable opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DbRowColumn {
    pub column: u8,
    pub tuple_types: Vec<u16>,
    pub rows: Vec<Vec<ScalarValue>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DbRowEntry {
    pub id: u32,
    pub table: Option<u32>,
    pub columns: Vec<DbRowColumn>,
}

pub fn parse_dbrow(id: u32, data: &[u8]) -> Result<DbRowEntry> {
    let mut packet = Packet::new(data);
    let mut table = None;
    let mut columns = BTreeMap::<u8, DbRowColumn>::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("dbrow {id} did not consume full payload");
                }
                return Ok(DbRowEntry {
                    id,
                    table,
                    columns: columns.into_values().collect(),
                });
            }
            3 => {
                let _unused = packet.g1()?;
                let mut column = packet.g1()?;
                while column != u8::MAX {
                    let tuple_count = usize::from(packet.g1()?);
                    let tuple_types = read_type_ids(&mut packet, tuple_count)?;
                    let row_count = usize::from(packet.gsmart1or2()?);

                    let column_entry = columns.entry(column).or_insert_with(|| DbRowColumn {
                        column,
                        tuple_types: tuple_types.clone(),
                        rows: Vec::new(),
                    });
                    if column_entry.tuple_types != tuple_types {
                        bail!("dbrow {id} conflicting tuple schema in column {column}");
                    }

                    for _ in 0..row_count {
                        column_entry
                            .rows
                            .push(read_tuple(&mut packet, &tuple_types)?);
                    }

                    column = packet.g1()?;
                }
            }
            4 => {
                table = Some(packet.gvarint2()?);
            }
            opcode => bail!("unknown dbrow opcode {opcode} in {id}"),
        }
    }
}

fn read_type_ids(packet: &mut Packet<'_>, count: usize) -> Result<Vec<u16>> {
    let mut types = Vec::with_capacity(count);
    for _ in 0..count {
        types.push(packet.gsmart1or2()?);
    }
    Ok(types)
}

fn set_dbtable_tuple_types(
    id: u32,
    column: u8,
    tuple_types: &[u16],
    columns: &mut BTreeMap<u8, DbTableColumn>,
) -> Result<()> {
    let entry = columns.entry(column).or_insert_with(|| DbTableColumn {
        column,
        tuple_types: tuple_types.to_vec(),
        defaults: Vec::new(),
    });
    if entry.tuple_types != tuple_types {
        bail!("dbtable {id} conflicting tuple schema in column {column}");
    }
    Ok(())
}

fn push_dbtable_default(
    column: u8,
    tuple: Vec<ScalarValue>,
    columns: &mut BTreeMap<u8, DbTableColumn>,
) -> Result<()> {
    let Some(entry) = columns.get_mut(&column) else {
        bail!("dbtable missing column {column} before defaults");
    };
    entry.defaults.push(tuple);
    Ok(())
}

fn read_tuple(packet: &mut Packet<'_>, tuple_types: &[u16]) -> Result<Vec<ScalarValue>> {
    let mut values = Vec::with_capacity(tuple_types.len());
    for type_id in tuple_types {
        let value = match dbtype_base(*type_id) {
            DbTypeBase::Int => ScalarValue::Int(packet.g4s()?),
            DbTypeBase::Long => ScalarValue::Long(packet.g8s()?),
            DbTypeBase::String => ScalarValue::Str(packet.gjstr()?),
        };
        values.push(value);
    }
    Ok(values)
}

/// Storage base of a `ScriptVarType` id, as the dbtable/dbrow wire format reads
/// and writes it. Exposed so the config transcoder encodes scalars by the same
/// classification the reader decodes them with (the client's
/// `ScriptVarType.baseType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbTypeBase {
    /// 32-bit signed integer scalar.
    Int,
    /// 64-bit signed long scalar.
    Long,
    /// Null-terminated Windows-1252 string scalar.
    String,
}

/// Classify a `ScriptVarType` id into its storage [`DbTypeBase`]. Shared by the
/// dbtable reader ([`parse_dbtable`]) and the config transcoder's encoder.
#[must_use]
pub fn dbtype_base(type_id: u16) -> DbTypeBase {
    match type_id {
        36 => DbTypeBase::String,
        35 | 49 | 56 | 71 | 110 | 115 | 116 | 118 => DbTypeBase::Long,
        _ => DbTypeBase::Int,
    }
}
