use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum ScalarValue {
    Int(i32),
    Long(i64),
    Str(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct ParamEntry {
    pub id: u32,
    pub type_char: Option<u8>,
    pub type_id: Option<u16>,
    pub default: Option<ScalarValue>,
    pub autodisable: bool,
}

pub fn parse_param(id: u32, data: &[u8]) -> Result<ParamEntry> {
    let mut packet = Packet::new(data);
    let mut entry = ParamEntry {
        id,
        type_char: None,
        type_id: None,
        default: None,
        autodisable: true,
    };

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("param {id} did not consume full payload");
                }
                return Ok(entry);
            }
            1 => entry.type_char = Some(packet.g1()?),
            2 => entry.default = Some(ScalarValue::Int(packet.g4s()?)),
            4 => entry.autodisable = false,
            5 => entry.default = Some(ScalarValue::Str(packet.gjstr()?)),
            101 => entry.type_id = Some(packet.gsmart1or2()?),
            opcode => bail!("unknown param opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EnumPair {
    pub key: i32,
    pub value: ScalarValue,
    pub dense: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnumEntry {
    pub id: u32,
    pub input_type_char: Option<u8>,
    pub output_type_char: Option<u8>,
    pub input_type_id: Option<u16>,
    pub output_type_id: Option<u16>,
    pub default: Option<ScalarValue>,
    pub values: Vec<EnumPair>,
}

pub fn parse_enum(id: u32, data: &[u8]) -> Result<EnumEntry> {
    let mut packet = Packet::new(data);
    let mut entry = EnumEntry {
        id,
        input_type_char: None,
        output_type_char: None,
        input_type_id: None,
        output_type_id: None,
        default: None,
        values: Vec::new(),
    };

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("enum {id} did not consume full payload");
                }
                return Ok(entry);
            }
            1 => entry.input_type_char = Some(packet.g1()?),
            2 => entry.output_type_char = Some(packet.g1()?),
            3 => entry.default = Some(ScalarValue::Str(packet.gjstr()?)),
            4 => entry.default = Some(ScalarValue::Int(packet.g4s()?)),
            5 => {
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: packet.g4s()?,
                        value: ScalarValue::Str(packet.gjstr()?),
                        dense: false,
                    });
                }
            }
            6 => {
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: packet.g4s()?,
                        value: ScalarValue::Int(packet.g4s()?),
                        dense: false,
                    });
                }
            }
            7 => {
                let _capacity = packet.g2()?;
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: i32::from(packet.g2()?),
                        value: ScalarValue::Str(packet.gjstr()?),
                        dense: true,
                    });
                }
            }
            8 => {
                let _capacity = packet.g2()?;
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: i32::from(packet.g2()?),
                        value: ScalarValue::Int(packet.g4s()?),
                        dense: true,
                    });
                }
            }
            101 => entry.input_type_id = Some(packet.gsmart1or2()?),
            102 => entry.output_type_id = Some(packet.gsmart1or2()?),
            opcode => bail!("unknown enum opcode {opcode} in {id}"),
        }
    }
}

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

#[derive(Clone, Debug, Serialize)]
pub struct StructParamEntry {
    pub param_id: u32,
    pub value: ScalarValue,
}

#[derive(Clone, Debug, Serialize)]
pub struct StructEntry {
    pub id: u32,
    pub params: Vec<StructParamEntry>,
}

pub fn parse_struct(id: u32, data: &[u8]) -> Result<StructEntry> {
    let mut packet = Packet::new(data);
    let mut params = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("struct {id} did not consume full payload");
                }
                return Ok(StructEntry { id, params });
            }
            249 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let is_string = packet.g1()? == 1;
                    let param_id = packet.g3()?;
                    let value = if is_string {
                        ScalarValue::Str(packet.gjstr()?)
                    } else {
                        ScalarValue::Int(packet.g4s()?)
                    };
                    params.push(StructParamEntry { param_id, value });
                }
            }
            opcode => bail!("unknown struct opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InvStockEntry {
    pub obj_id: u16,
    pub count: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct InvEntry {
    pub id: u32,
    pub size: Option<u16>,
    pub stocks: Vec<InvStockEntry>,
}

pub fn parse_inv(id: u32, data: &[u8]) -> Result<InvEntry> {
    let mut packet = Packet::new(data);
    let mut size = None;
    let mut stocks = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("inv {id} did not consume full payload");
                }
                return Ok(InvEntry { id, size, stocks });
            }
            2 => size = Some(packet.g2()?),
            4 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    stocks.push(InvStockEntry {
                        obj_id: packet.g2()?,
                        count: packet.g2()?,
                    });
                }
            }
            opcode => bail!("unknown inv opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CursorHotspot {
    pub x: u8,
    pub y: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct CursorEntry {
    pub id: u32,
    pub graphic: Option<i32>,
    pub hotspot: Option<CursorHotspot>,
}

pub fn parse_cursor(id: u32, data: &[u8]) -> Result<CursorEntry> {
    let mut packet = Packet::new(data);
    let mut graphic = None;
    let mut hotspot = None;

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("cursor {id} did not consume full payload");
                }
                return Ok(CursorEntry {
                    id,
                    graphic,
                    hotspot,
                });
            }
            1 => graphic = Some(packet.gsmart2or4null()?),
            2 => {
                hotspot = Some(CursorHotspot {
                    x: packet.g1()?,
                    y: packet.g1()?,
                });
            }
            opcode => bail!("unknown cursor opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SeqGroupOverride {
    pub label: u16,
    pub value: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeqGroupEntry {
    pub id: u32,
    pub walkmerge: Vec<u16>,
    pub unknown3_default: Option<u8>,
    pub unknown3: Vec<SeqGroupOverride>,
    pub unknown4_default: Option<u8>,
    pub unknown4: Vec<SeqGroupOverride>,
}

pub fn parse_seqgroup(id: u32, data: &[u8]) -> Result<SeqGroupEntry> {
    let mut packet = Packet::new(data);
    let mut walkmerge = Vec::new();
    let mut unknown3_default = None;
    let mut unknown3 = Vec::new();
    let mut unknown4_default = None;
    let mut unknown4 = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("seqgroup {id} did not consume full payload");
                }
                return Ok(SeqGroupEntry {
                    id,
                    walkmerge,
                    unknown3_default,
                    unknown3,
                    unknown4_default,
                    unknown4,
                });
            }
            2 => {
                let count = usize::from(packet.gsmart1or2()?);
                for _ in 0..count {
                    walkmerge.push(packet.gsmart1or2()?);
                }
            }
            3 => {
                unknown3_default = Some(packet.g1()?);
                let count = usize::from(packet.gsmart1or2()?);
                for _ in 0..count {
                    unknown3.push(SeqGroupOverride {
                        label: packet.gsmart1or2()?,
                        value: packet.g1()?,
                    });
                }
            }
            4 => {
                unknown4_default = Some(packet.g1()?);
                let count = usize::from(packet.gsmart1or2()?);
                for _ in 0..count {
                    unknown4.push(SeqGroupOverride {
                        label: packet.gsmart1or2()?,
                        value: packet.g1()?,
                    });
                }
            }
            opcode => bail!("unknown seqgroup opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ControllerEntry {
    pub id: u32,
}

pub fn parse_controller(id: u32, data: &[u8]) -> Result<ControllerEntry> {
    let mut packet = Packet::new(data);
    let opcode = packet.g1()?;
    if opcode != 0 {
        bail!("unknown controller opcode {opcode} in {id}");
    }
    if !packet.is_done() {
        bail!("controller {id} did not consume full payload");
    }
    Ok(ControllerEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct CategoryEntry {
    pub id: u32,
}

pub fn parse_category(id: u32, data: &[u8]) -> Result<CategoryEntry> {
    let mut packet = Packet::new(data);
    let opcode = packet.g1()?;
    if opcode != 0 {
        bail!("unknown category opcode {opcode} in {id}");
    }
    if !packet.is_done() {
        bail!("category {id} did not consume full payload");
    }
    Ok(CategoryEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct AreaEntry {
    pub id: u32,
}

pub fn parse_area(id: u32, data: &[u8]) -> Result<AreaEntry> {
    parse_empty_config("area", id, data)?;
    Ok(AreaEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct HuntEntry {
    pub id: u32,
}

pub fn parse_hunt(id: u32, data: &[u8]) -> Result<HuntEntry> {
    parse_empty_config("hunt", id, data)?;
    Ok(HuntEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct MesAnimEntry {
    pub id: u32,
}

pub fn parse_mesanim(id: u32, data: &[u8]) -> Result<MesAnimEntry> {
    parse_empty_config("mesanim", id, data)?;
    Ok(MesAnimEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct ItemCodeEntry {
    pub id: u32,
}

pub fn parse_itemcode(id: u32, data: &[u8]) -> Result<ItemCodeEntry> {
    parse_empty_config("itemcode", id, data)?;
    Ok(ItemCodeEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct GameLogEventEntry {
    pub id: u32,
}

pub fn parse_gamelogevent(id: u32, data: &[u8]) -> Result<GameLogEventEntry> {
    parse_empty_config("gamelogevent", id, data)?;
    Ok(GameLogEventEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct BugTemplateEntry {
    pub id: u32,
}

pub fn parse_bugtemplate(id: u32, data: &[u8]) -> Result<BugTemplateEntry> {
    parse_empty_config("bugtemplate", id, data)?;
    Ok(BugTemplateEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct VarClientStringEntry {
    pub id: u32,
    pub scope_perm: bool,
}

pub fn parse_var_client_string(id: u32, data: &[u8]) -> Result<VarClientStringEntry> {
    let mut packet = Packet::new(data);
    let mut scope_perm = false;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("varcstr {id} did not consume full payload");
                }
                return Ok(VarClientStringEntry { id, scope_perm });
            }
            2 => scope_perm = true,
            opcode => bail!("unknown varcstr opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VarNpcBitEntry {
    pub id: u32,
}

pub fn parse_var_npc_bit(id: u32, data: &[u8]) -> Result<VarNpcBitEntry> {
    parse_empty_config("varnbit", id, data)?;
    Ok(VarNpcBitEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct VarSharedEntry {
    pub id: u32,
}

pub fn parse_var_shared(id: u32, data: &[u8]) -> Result<VarSharedEntry> {
    parse_empty_config("vars", id, data)?;
    Ok(VarSharedEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct VarSharedStringEntry {
    pub id: u32,
}

pub fn parse_var_shared_string(id: u32, data: &[u8]) -> Result<VarSharedStringEntry> {
    parse_empty_config("varsstr", id, data)?;
    Ok(VarSharedStringEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct FloorUnderlayEntry {
    pub id: u32,
    pub colour: Option<u32>,
    pub material: Option<i32>,
    pub texture_scale: Option<u16>,
    pub hardshadow: bool,
    pub occlude: bool,
}

pub fn parse_underlay(id: u32, data: &[u8]) -> Result<FloorUnderlayEntry> {
    let mut packet = Packet::new(data);
    let mut colour = None;
    let mut material = None;
    let mut texture_scale = None;
    let mut hardshadow = true;
    let mut occlude = true;

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("underlay {id} did not consume full payload");
                }
                return Ok(FloorUnderlayEntry {
                    id,
                    colour,
                    material,
                    texture_scale,
                    hardshadow,
                    occlude,
                });
            }
            1 => colour = Some(packet.g3()?),
            2 => material = Some(packet.g2null()?),
            3 => texture_scale = Some(packet.g2()?),
            4 => hardshadow = false,
            5 => occlude = false,
            opcode => bail!("unknown underlay opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FloorOverlayToggles {
    pub unknown8: bool,
    pub smoothedges: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FloorOverlayEntry {
    pub id: u32,
    pub colour: Option<u32>,
    pub material: Option<i32>,
    pub occlude: bool,
    pub debugname: Option<String>,
    pub mapcolour: Option<u32>,
    pub toggles: FloorOverlayToggles,
    pub texture_scale: Option<u16>,
    pub hardshadow: bool,
    pub priority: Option<u8>,
    pub waterfog_colour: Option<u32>,
    pub waterfog_scale: Option<u8>,
    pub unknown15: Option<u16>,
    pub waterfog_offset: Option<u8>,
    pub waterfog_unknown_a: Option<u16>,
    pub waterfog_unknown_b: Option<u8>,
    pub waterfog_unknown_c: Option<u16>,
}

pub fn parse_overlay(id: u32, data: &[u8]) -> Result<FloorOverlayEntry> {
    let mut packet = Packet::new(data);
    let mut colour = None;
    let mut material = None;
    let mut occlude = true;
    let mut debugname = None;
    let mut mapcolour = None;
    let mut unknown8 = false;
    let mut texture_scale = None;
    let mut hardshadow = true;
    let mut priority = None;
    let mut smoothedges = false;
    let mut waterfog_colour = None;
    let mut waterfog_scale = None;
    let mut unknown15 = None;
    let mut waterfog_offset = None;
    let mut waterfog_unknown_a = None;
    let mut waterfog_unknown_b = None;
    let mut waterfog_unknown_c = None;

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("overlay {id} did not consume full payload");
                }
                return Ok(FloorOverlayEntry {
                    id,
                    colour,
                    material,
                    occlude,
                    debugname,
                    mapcolour,
                    toggles: FloorOverlayToggles {
                        unknown8,
                        smoothedges,
                    },
                    texture_scale,
                    hardshadow,
                    priority,
                    waterfog_colour,
                    waterfog_scale,
                    unknown15,
                    waterfog_offset,
                    waterfog_unknown_a,
                    waterfog_unknown_b,
                    waterfog_unknown_c,
                });
            }
            1 => colour = Some(packet.g3()?),
            2 => material = Some(i32::from(packet.g1()?)),
            3 => material = Some(packet.g2null()?),
            5 => occlude = false,
            6 => debugname = Some(packet.gjstr()?),
            7 => mapcolour = Some(packet.g3()?),
            8 => unknown8 = true,
            9 => texture_scale = Some(packet.g2()?),
            10 => hardshadow = false,
            11 => priority = Some(packet.g1()?),
            12 => smoothedges = true,
            13 => waterfog_colour = Some(packet.g3()?),
            14 => waterfog_scale = Some(packet.g1()?),
            15 => unknown15 = Some(packet.g2()?),
            16 => waterfog_offset = Some(packet.g1()?),
            20 => waterfog_unknown_a = Some(packet.g2()?),
            21 => waterfog_unknown_b = Some(packet.g1()?),
            22 => waterfog_unknown_c = Some(packet.g2()?),
            opcode => bail!("unknown overlay opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MsiEntry {
    pub id: u32,
    pub graphic: Option<i32>,
    pub unknown2: Option<u32>,
    pub unknown3: bool,
    pub unknown4: bool,
    pub unknown5: bool,
}

pub fn parse_msi(id: u32, data: &[u8]) -> Result<MsiEntry> {
    let mut packet = Packet::new(data);
    let mut graphic = None;
    let mut unknown2 = None;
    let mut unknown3 = false;
    let mut unknown4 = false;
    let mut unknown5 = false;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("msi {id} did not consume full payload");
                }
                return Ok(MsiEntry {
                    id,
                    graphic,
                    unknown2,
                    unknown3,
                    unknown4,
                    unknown5,
                });
            }
            1 => graphic = Some(packet.gsmart2or4null()?),
            2 => unknown2 = Some(packet.g3()?),
            3 => unknown3 = true,
            4 => unknown4 = true,
            5 => unknown5 = true,
            opcode => bail!("unknown msi opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SkyBoxEntry {
    pub id: u32,
    pub material: Option<u16>,
    pub unknown2: Vec<u16>,
    pub unknown3: Option<u8>,
    pub fillmode: Option<u8>,
    pub unknown5: Option<i32>,
    pub unknown6: Option<i32>,
}

pub fn parse_skybox(id: u32, data: &[u8]) -> Result<SkyBoxEntry> {
    let mut packet = Packet::new(data);
    let mut material = None;
    let mut unknown2 = Vec::new();
    let mut unknown3 = None;
    let mut fillmode = None;
    let mut unknown5 = None;
    let mut unknown6 = None;

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("skybox {id} did not consume full payload");
                }
                return Ok(SkyBoxEntry {
                    id,
                    material,
                    unknown2,
                    unknown3,
                    fillmode,
                    unknown5,
                    unknown6,
                });
            }
            1 => material = Some(packet.g2()?),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    unknown2.push(packet.g2()?);
                }
            }
            3 => unknown3 = Some(packet.g1()?),
            4 => fillmode = Some(packet.g1()?),
            5 => unknown5 = Some(packet.gsmart2or4null()?),
            6 => unknown6 = Some(packet.gsmart2or4null()?),
            opcode => bail!("unknown skybox opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldAreaSquareRange {
    pub start: i32,
    pub end: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldAreaTemplateRange {
    pub anchor: i32,
    pub template: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldAreaEntry {
    pub id: u32,
    pub colour: Option<u32>,
    pub impostor_squares: Vec<WorldAreaSquareRange>,
    pub impostor_zones: Vec<WorldAreaTemplateRange>,
}

pub fn parse_worldarea(id: u32, data: &[u8]) -> Result<WorldAreaEntry> {
    let mut packet = Packet::new(data);
    let mut colour = None;
    let mut impostor_squares = Vec::new();
    let mut impostor_zones = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("worldarea {id} did not consume full payload");
                }
                return Ok(WorldAreaEntry {
                    id,
                    colour,
                    impostor_squares,
                    impostor_zones,
                });
            }
            2 => colour = Some(packet.g3()?),
            3 => {
                impostor_squares.push(WorldAreaSquareRange {
                    start: packet.g4s()?,
                    end: packet.g4s()?,
                });
            }
            4 => {
                let anchor = packet.g4s()?;
                let template = format_template_zone(packet.g4s()?)?;
                impostor_zones.push(WorldAreaTemplateRange { anchor, template });
            }
            opcode => bail!("unknown worldarea opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QuickChatCategoryLink {
    pub id: u16,
    pub shortcut: i8,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuickChatCategoryEntry {
    pub id: u32,
    pub desc: Option<String>,
    pub subcats: Vec<QuickChatCategoryLink>,
    pub phrases: Vec<QuickChatCategoryLink>,
    pub unknown4: bool,
}

pub fn parse_quickchatcat(id: u32, data: &[u8]) -> Result<QuickChatCategoryEntry> {
    let mut packet = Packet::new(data);
    let mut desc = None;
    let mut subcats = Vec::new();
    let mut phrases = Vec::new();
    let mut unknown4 = false;

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("quickchatcat {id} did not consume full payload");
                }
                return Ok(QuickChatCategoryEntry {
                    id,
                    desc,
                    subcats,
                    phrases,
                    unknown4,
                });
            }
            1 => desc = Some(packet.gjstr()?),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    subcats.push(QuickChatCategoryLink {
                        id: packet.g2()?,
                        shortcut: packet.g1s()?,
                    });
                }
            }
            3 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    phrases.push(QuickChatCategoryLink {
                        id: packet.g2()?,
                        shortcut: packet.g1s()?,
                    });
                }
            }
            4 => unknown4 = true,
            opcode => bail!("unknown quickchatcat opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HeadbarEntry {
    pub id: u32,
    pub unknown1: Option<u16>,
    pub showpriority: Option<u8>,
    pub hidepriority: Option<u8>,
    pub fadeout_disabled: bool,
    pub sticktime: Option<u16>,
    pub unknown6: Option<u16>,
    pub full: Option<i32>,
    pub empty: Option<i32>,
    pub fullplayergroup: Option<i32>,
    pub emptyplayergroup: Option<i32>,
    pub fadeout: Option<u16>,
    pub fullplayergroupteam: Option<i32>,
    pub emptyplayergroupteam: Option<i32>,
    pub unknown14: Option<i32>,
    pub unknown15: Option<i32>,
    pub unknown16: bool,
    pub unknown17: Option<u8>,
}

pub fn parse_headbar(id: u32, data: &[u8]) -> Result<HeadbarEntry> {
    let mut packet = Packet::new(data);
    let mut unknown1 = None;
    let mut showpriority = None;
    let mut hidepriority = None;
    let mut fadeout_disabled = false;
    let mut sticktime = None;
    let mut unknown6 = None;
    let mut full = None;
    let mut empty = None;
    let mut fullplayergroup = None;
    let mut emptyplayergroup = None;
    let mut fadeout = None;
    let mut fullplayergroupteam = None;
    let mut emptyplayergroupteam = None;
    let mut unknown14 = None;
    let mut unknown15 = None;
    let mut unknown16_flag = false;
    let mut unknown17 = None;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("headbar {id} did not consume full payload");
                }
                return Ok(HeadbarEntry {
                    id,
                    unknown1,
                    showpriority,
                    hidepriority,
                    fadeout_disabled,
                    sticktime,
                    unknown6,
                    full,
                    empty,
                    fullplayergroup,
                    emptyplayergroup,
                    fadeout,
                    fullplayergroupteam,
                    emptyplayergroupteam,
                    unknown14,
                    unknown15,
                    unknown16: unknown16_flag,
                    unknown17,
                });
            }
            1 => unknown1 = Some(packet.g2()?),
            2 => showpriority = Some(packet.g1()?),
            3 => hidepriority = Some(packet.g1()?),
            4 => fadeout_disabled = true,
            5 => sticktime = Some(packet.g2()?),
            6 => unknown6 = Some(packet.g2()?),
            7 => full = Some(packet.gsmart2or4null()?),
            8 => empty = Some(packet.gsmart2or4null()?),
            9 => fullplayergroup = Some(packet.gsmart2or4null()?),
            10 => emptyplayergroup = Some(packet.gsmart2or4null()?),
            11 => fadeout = Some(packet.g2()?),
            12 => fullplayergroupteam = Some(packet.gsmart2or4null()?),
            13 => emptyplayergroupteam = Some(packet.gsmart2or4null()?),
            14 => unknown14 = Some(packet.gsmart2or4null()?),
            15 => unknown15 = Some(packet.gsmart2or4null()?),
            16 => unknown16_flag = true,
            17 => unknown17 = Some(packet.g1()?),
            opcode => bail!("unknown headbar opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HitmarkMulti {
    Indexed { index: u8, hitmark_id: i32 },
    Default { hitmark_id: i32 },
}

#[derive(Clone, Debug, Serialize)]
pub struct HitmarkEntry {
    pub id: u32,
    pub damagefont: Option<i32>,
    pub damagecolour: Option<u32>,
    pub classgraphic: Option<i32>,
    pub leftgraphic: Option<i32>,
    pub middlegraphic: Option<i32>,
    pub rightgraphic: Option<i32>,
    pub scrolltooffsetx: Option<i16>,
    pub damageformat: Option<String>,
    pub sticktime: Option<u16>,
    pub scrolltooffsety: Option<i16>,
    pub fadeout_disabled: bool,
    pub replacemode: Option<u8>,
    pub damageyof: Option<i16>,
    pub fadeout: Option<u16>,
    pub graphicof: Option<(u16, u16)>,
    pub multivarbit: Option<i32>,
    pub multivarp: Option<i32>,
    pub multimarks: Vec<HitmarkMulti>,
    pub damagescaleto: Option<u16>,
    pub damagescalefrom: Option<u16>,
}

pub fn parse_hitmark(id: u32, data: &[u8]) -> Result<HitmarkEntry> {
    let mut packet = Packet::new(data);
    let mut damagefont = None;
    let mut damagecolour = None;
    let mut classgraphic = None;
    let mut leftgraphic = None;
    let mut middlegraphic = None;
    let mut rightgraphic = None;
    let mut scroll_offset_x = None;
    let mut damageformat = None;
    let mut sticktime = None;
    let mut scroll_offset_y = None;
    let mut fadeout_disabled = false;
    let mut replacemode = None;
    let mut damageyof = None;
    let mut fadeout = None;
    let mut graphicof = None;
    let mut multivarbit = None;
    let mut multivarp = None;
    let mut multimarks = Vec::new();
    let mut damagescaleto = None;
    let mut damagescalefrom = None;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("hitmark {id} did not consume full payload");
                }
                return Ok(HitmarkEntry {
                    id,
                    damagefont,
                    damagecolour,
                    classgraphic,
                    leftgraphic,
                    middlegraphic,
                    rightgraphic,
                    scrolltooffsetx: scroll_offset_x,
                    damageformat,
                    sticktime,
                    scrolltooffsety: scroll_offset_y,
                    fadeout_disabled,
                    replacemode,
                    damageyof,
                    fadeout,
                    graphicof,
                    multivarbit,
                    multivarp,
                    multimarks,
                    damagescaleto,
                    damagescalefrom,
                });
            }
            1 => damagefont = Some(packet.gsmart2or4null()?),
            2 => damagecolour = Some(packet.g3()?),
            3 => classgraphic = Some(packet.gsmart2or4null()?),
            4 => leftgraphic = Some(packet.gsmart2or4null()?),
            5 => middlegraphic = Some(packet.gsmart2or4null()?),
            6 => rightgraphic = Some(packet.gsmart2or4null()?),
            7 => scroll_offset_x = Some(packet.g2s()?),
            8 => damageformat = Some(packet.gjstr2()?),
            9 => sticktime = Some(packet.g2()?),
            10 => scroll_offset_y = Some(packet.g2s()?),
            11 => fadeout_disabled = true,
            12 => replacemode = Some(packet.g1()?),
            13 => damageyof = Some(packet.g2s()?),
            14 => fadeout = Some(packet.g2()?),
            16 => graphicof = Some((packet.g2()?, packet.g2()?)),
            op @ (17 | 18) => {
                let local_multivarbit = packet.g2null()?;
                if local_multivarbit != -1 {
                    multivarbit = Some(local_multivarbit);
                }
                let local_multivarp = packet.g2null()?;
                if local_multivarp != -1 {
                    multivarp = Some(local_multivarp);
                }
                if op == 18 {
                    let value = packet.g2null()?;
                    if value != -1 {
                        multimarks.push(HitmarkMulti::Default { hitmark_id: value });
                    }
                }
                let count = usize::from(packet.g1()?);
                for index in 0..=count {
                    let value = packet.g2null()?;
                    if value != -1 {
                        multimarks.push(HitmarkMulti::Indexed {
                            index: u8::try_from(index).context("hitmark multi index overflow")?,
                            hitmark_id: value,
                        });
                    }
                }
            }
            19 => damagescaleto = Some(packet.g2()?),
            20 => damagescalefrom = Some(packet.g2()?),
            opcode => bail!("unknown hitmark opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LightEntry {
    pub id: u32,
    pub function: Option<u8>,
    pub frequency: Option<u16>,
    pub amplitude: Option<u16>,
    pub offset: Option<i16>,
    pub unknown5: Option<i32>,
    pub unknown6: Option<i32>,
    pub unknown7: Option<i32>,
    pub swayamount: Option<f32>,
    pub swayamountrandom: bool,
    pub swayduration: Option<i32>,
    pub swaydurationrandom: Option<i32>,
    pub swayeasing: Option<f32>,
    pub swayfade: Option<f32>,
}

pub fn parse_light(id: u32, data: &[u8]) -> Result<LightEntry> {
    let mut packet = Packet::new(data);
    let mut function = None;
    let mut frequency = None;
    let mut amplitude = None;
    let mut offset = None;
    let mut unknown5 = None;
    let mut unknown6 = None;
    let mut unknown7 = None;
    let mut swayamount = None;
    let mut swayamountrandom = false;
    let mut swayduration = None;
    let mut swaydurationrandom = None;
    let mut swayeasing = None;
    let mut swayfade = None;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("light {id} did not consume full payload");
                }
                return Ok(LightEntry {
                    id,
                    function,
                    frequency,
                    amplitude,
                    offset,
                    unknown5,
                    unknown6,
                    unknown7,
                    swayamount,
                    swayamountrandom,
                    swayduration,
                    swaydurationrandom,
                    swayeasing,
                    swayfade,
                });
            }
            1 => function = Some(packet.g1()?),
            2 => frequency = Some(packet.g2()?),
            3 => amplitude = Some(packet.g2()?),
            4 => offset = Some(packet.g2s()?),
            5 => unknown5 = Some(packet.g4s()?),
            6 => unknown6 = Some(packet.g4s()?),
            7 => unknown7 = Some(packet.g4s()?),
            8 => swayamount = Some(gfloat_be(&mut packet)?),
            9 => swayamountrandom = true,
            10 => swayduration = Some(packet.g4s()?),
            11 => swaydurationrandom = Some(packet.g4s()?),
            12 => swayeasing = Some(gfloat_be(&mut packet)?),
            13 => swayfade = Some(gfloat_be(&mut packet)?),
            opcode => bail!("unknown light opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuickChatDynamicCommand {
    ListDialog { enum_id: u16 },
    ObjDialog,
    CountDialog,
    StatBase { stat_id: u16 },
    EnumString { enum_id: u16, var_player_id: u16 },
    EnumStringClan { enum_id: u16 },
    VarPlayerInt { var_player_id: u16 },
    VarPlayerBit { varbit_id: u16 },
    ObjTradeDialog,
    EnumStringStatBase { enum_id: u16, stat_id: u16 },
    Unknown12,
    Unknown13,
    VarWorldInt { var_world_id: u16 },
    CombatLevel,
    EnumStringVarPlayerBit { enum_id: u16, varbit_id: u16 },
}

#[derive(Clone, Debug, Serialize)]
pub struct QuickChatPhraseEntry {
    pub id: u32,
    pub template: Option<String>,
    pub autoresponses: Vec<u16>,
    pub dynamic_commands: Vec<QuickChatDynamicCommand>,
    pub unknown4_no: bool,
}

pub fn parse_quickchatphrase(id: u32, data: &[u8]) -> Result<QuickChatPhraseEntry> {
    let mut packet = Packet::new(data);
    let mut template = None;
    let mut autoresponses = Vec::new();
    let mut dynamic_commands = Vec::new();
    let mut unknown4_no = false;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("quickchatphrase {id} did not consume full payload");
                }
                return Ok(QuickChatPhraseEntry {
                    id,
                    template,
                    autoresponses,
                    dynamic_commands,
                    unknown4_no,
                });
            }
            1 => template = Some(packet.gjstr()?),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    autoresponses.push(packet.g2()?);
                }
            }
            3 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let command = packet.g2()?;
                    dynamic_commands.push(match command {
                        0 => QuickChatDynamicCommand::ListDialog {
                            enum_id: packet.g2()?,
                        },
                        1 => QuickChatDynamicCommand::ObjDialog,
                        2 => QuickChatDynamicCommand::CountDialog,
                        4 => QuickChatDynamicCommand::StatBase {
                            stat_id: packet.g2()?,
                        },
                        6 => QuickChatDynamicCommand::EnumString {
                            enum_id: packet.g2()?,
                            var_player_id: packet.g2()?,
                        },
                        7 => QuickChatDynamicCommand::EnumStringClan {
                            enum_id: packet.g2()?,
                        },
                        8 => QuickChatDynamicCommand::VarPlayerInt {
                            var_player_id: packet.g2()?,
                        },
                        9 => QuickChatDynamicCommand::VarPlayerBit {
                            varbit_id: packet.g2()?,
                        },
                        10 => QuickChatDynamicCommand::ObjTradeDialog,
                        11 => QuickChatDynamicCommand::EnumStringStatBase {
                            enum_id: packet.g2()?,
                            stat_id: packet.g2()?,
                        },
                        12 => QuickChatDynamicCommand::Unknown12,
                        13 => QuickChatDynamicCommand::Unknown13,
                        14 => QuickChatDynamicCommand::VarWorldInt {
                            var_world_id: packet.g2()?,
                        },
                        15 => QuickChatDynamicCommand::CombatLevel,
                        16 => QuickChatDynamicCommand::EnumStringVarPlayerBit {
                            enum_id: packet.g2()?,
                            varbit_id: packet.g2()?,
                        },
                        value => bail!("invalid quickchat dynamiccommand {value} in {id}"),
                    });
                }
            }
            4 => unknown4_no = true,
            opcode => bail!("unknown quickchatphrase opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BillboardEntry {
    pub id: u32,
    pub material: Option<i32>,
    pub unknown2: Option<(u16, u16)>,
    pub unknown3: Option<i8>,
    pub unknown4: Option<u8>,
    pub unknown5: Option<u8>,
    pub unknown6: bool,
    pub unknown7: bool,
}

pub fn parse_billboard(id: u32, data: &[u8]) -> Result<BillboardEntry> {
    let mut packet = Packet::new(data);
    let mut material = None;
    let mut unknown2 = None;
    let mut unknown3 = None;
    let mut unknown4 = None;
    let mut unknown5 = None;
    let mut unknown6 = false;
    let mut unknown7 = false;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("billboard {id} did not consume full payload");
                }
                return Ok(BillboardEntry {
                    id,
                    material,
                    unknown2,
                    unknown3,
                    unknown4,
                    unknown5,
                    unknown6,
                    unknown7,
                });
            }
            1 => material = Some(packet.g2null()?),
            2 => unknown2 = Some((packet.g2()?, packet.g2()?)),
            3 => unknown3 = Some(packet.g1s()?),
            4 => unknown4 = Some(packet.g1()?),
            5 => unknown5 = Some(packet.g1()?),
            6 => unknown6 = true,
            7 => unknown7 = true,
            opcode => bail!("unknown billboard opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParticleEffectorOp {
    Unknown1(u16),
    Unknown2(u8),
    Unknown3 { x: i32, y: i32, z: i32 },
    Unknown4 { a: u8, b: i32 },
    Unknown6(u8),
    Unknown8,
    Unknown9,
    Unknown10,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParticleEffectorEntry {
    pub id: u32,
    pub ops: Vec<ParticleEffectorOp>,
}

pub fn parse_particle_effector(id: u32, data: &[u8]) -> Result<ParticleEffectorEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("particleeffector {id} did not consume full payload");
                }
                return Ok(ParticleEffectorEntry { id, ops });
            }
            1 => ops.push(ParticleEffectorOp::Unknown1(packet.g2()?)),
            2 => ops.push(ParticleEffectorOp::Unknown2(packet.g1()?)),
            3 => ops.push(ParticleEffectorOp::Unknown3 {
                x: packet.g4s()?,
                y: packet.g4s()?,
                z: packet.g4s()?,
            }),
            4 => ops.push(ParticleEffectorOp::Unknown4 {
                a: packet.g1()?,
                b: packet.g4s()?,
            }),
            6 => ops.push(ParticleEffectorOp::Unknown6(packet.g1()?)),
            8 => ops.push(ParticleEffectorOp::Unknown8),
            9 => ops.push(ParticleEffectorOp::Unknown9),
            10 => ops.push(ParticleEffectorOp::Unknown10),
            opcode => bail!("unknown particleeffector opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParticleEmitterOp {
    Unknown1 { a: u16, b: u16, c: u16, d: u16 },
    Unknown2(u8),
    Unknown3 { a: i32, b: i32 },
    Unknown4 { a: u8, b: i8 },
    Unknown5(u16),
    Unknown6 { a: i32, b: i32 },
    Unknown7 { a: u16, b: u16 },
    Unknown8 { a: u16, b: u16 },
    Unknown9(Vec<u16>),
    Unknown10(Vec<u16>),
    Unknown12(i8),
    Unknown13(i8),
    Unknown14(u16),
    Unknown15(u16),
    Unknown16 { a: u8, b: u16, c: u16, d: u8 },
    Unknown17(u16),
    Unknown18(i32),
    Unknown19(u8),
    Unknown20(u8),
    Unknown21(u8),
    Unknown22(i32),
    Unknown23(u8),
    Unknown24No,
    Unknown25(Vec<u16>),
    Unknown26No,
    Unknown27(u16),
    Unknown28(u8),
    AngularVelocity(i16),
    AngularVelocityRange { min: i16, max: i16 },
    Unknown30Yes,
    Unknown31 { a: u16, b: u16 },
    LightingNo,
    Unknown33Yes,
    Unknown34No,
    Unknown35(i16),
    Unknown35Range { min: i16, max: i16, mode: u8 },
    Unknown36Yes,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParticleEmitterEntry {
    pub id: u32,
    pub ops: Vec<ParticleEmitterOp>,
}

pub fn parse_particle_emitter(id: u32, data: &[u8]) -> Result<ParticleEmitterEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("particleemitter {id} did not consume full payload");
                }
                return Ok(ParticleEmitterEntry { id, ops });
            }
            1 => ops.push(ParticleEmitterOp::Unknown1 {
                a: packet.g2()?,
                b: packet.g2()?,
                c: packet.g2()?,
                d: packet.g2()?,
            }),
            2 => ops.push(ParticleEmitterOp::Unknown2(packet.g1()?)),
            3 => ops.push(ParticleEmitterOp::Unknown3 {
                a: packet.g4s()?,
                b: packet.g4s()?,
            }),
            4 => ops.push(ParticleEmitterOp::Unknown4 {
                a: packet.g1()?,
                b: packet.g1s()?,
            }),
            5 => ops.push(ParticleEmitterOp::Unknown5(packet.g2()?)),
            6 => ops.push(ParticleEmitterOp::Unknown6 {
                a: packet.g4s()?,
                b: packet.g4s()?,
            }),
            7 => ops.push(ParticleEmitterOp::Unknown7 {
                a: packet.g2()?,
                b: packet.g2()?,
            }),
            8 => ops.push(ParticleEmitterOp::Unknown8 {
                a: packet.g2()?,
                b: packet.g2()?,
            }),
            9 => ops.push(ParticleEmitterOp::Unknown9(read_u16_list_g1_count(
                &mut packet,
            )?)),
            10 => ops.push(ParticleEmitterOp::Unknown10(read_u16_list_g1_count(
                &mut packet,
            )?)),
            12 => ops.push(ParticleEmitterOp::Unknown12(packet.g1s()?)),
            13 => ops.push(ParticleEmitterOp::Unknown13(packet.g1s()?)),
            14 => ops.push(ParticleEmitterOp::Unknown14(packet.g2()?)),
            15 => ops.push(ParticleEmitterOp::Unknown15(packet.g2()?)),
            16 => ops.push(ParticleEmitterOp::Unknown16 {
                a: packet.g1()?,
                b: packet.g2()?,
                c: packet.g2()?,
                d: packet.g1()?,
            }),
            17 => ops.push(ParticleEmitterOp::Unknown17(packet.g2()?)),
            18 => ops.push(ParticleEmitterOp::Unknown18(packet.g4s()?)),
            19 => ops.push(ParticleEmitterOp::Unknown19(packet.g1()?)),
            20 => ops.push(ParticleEmitterOp::Unknown20(packet.g1()?)),
            21 => ops.push(ParticleEmitterOp::Unknown21(packet.g1()?)),
            22 => ops.push(ParticleEmitterOp::Unknown22(packet.g4s()?)),
            23 => ops.push(ParticleEmitterOp::Unknown23(packet.g1()?)),
            24 => ops.push(ParticleEmitterOp::Unknown24No),
            25 => ops.push(ParticleEmitterOp::Unknown25(read_u16_list_g1_count(
                &mut packet,
            )?)),
            26 => ops.push(ParticleEmitterOp::Unknown26No),
            27 => ops.push(ParticleEmitterOp::Unknown27(packet.g2()?)),
            28 => ops.push(ParticleEmitterOp::Unknown28(packet.g1()?)),
            29 => {
                let mode = packet.g1()?;
                if mode == 0 {
                    ops.push(ParticleEmitterOp::AngularVelocity(packet.g2s()?));
                } else {
                    ops.push(ParticleEmitterOp::AngularVelocityRange {
                        min: packet.g2s()?,
                        max: packet.g2s()?,
                    });
                }
            }
            30 => ops.push(ParticleEmitterOp::Unknown30Yes),
            31 => ops.push(ParticleEmitterOp::Unknown31 {
                a: packet.g2()?,
                b: packet.g2()?,
            }),
            32 => ops.push(ParticleEmitterOp::LightingNo),
            33 => ops.push(ParticleEmitterOp::Unknown33Yes),
            34 => ops.push(ParticleEmitterOp::Unknown34No),
            35 => {
                let mode = packet.g1()?;
                if mode == 0 {
                    ops.push(ParticleEmitterOp::Unknown35(packet.g2s()?));
                } else {
                    ops.push(ParticleEmitterOp::Unknown35Range {
                        min: packet.g2s()?,
                        max: packet.g2s()?,
                        mode: packet.g1()?,
                    });
                }
            }
            36 => ops.push(ParticleEmitterOp::Unknown36Yes),
            opcode => bail!("unknown particleemitter opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TextureEntry {
    pub id: u32,
    pub averagecolour: u16,
    pub opaque: bool,
    pub sprite: i32,
    pub unknown1: i32,
    pub animation: (u8, u8),
}

pub fn parse_texture(id: u32, data: &[u8]) -> Result<TextureEntry> {
    let mut packet = Packet::new(data);
    let averagecolour = packet.g2()?;
    let opaque = packet.g1()? == 1;
    if packet.g1()? != 1 {
        bail!("texture {id} unsupported format");
    }
    let sprite = packet.g2null()?;
    let unknown1 = packet.g4s()?;
    let animation = (packet.g1()?, packet.g1()?);
    if !packet.is_done() {
        bail!("texture {id} did not consume full payload");
    }
    Ok(TextureEntry {
        id,
        averagecolour,
        opaque,
        sprite,
        unknown1,
        animation,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct StylesheetPropertyEntry {
    pub unknown: u8,
    pub key_hash: i32,
    pub value: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct StylesheetEntry {
    pub id: u32,
    pub parent: i32,
    pub entries: Vec<StylesheetPropertyEntry>,
}

pub fn parse_stylesheet(id: u32, data: &[u8]) -> Result<StylesheetEntry> {
    let mut packet = Packet::new(data);
    let parent = packet.g2null()?;
    let count = usize::from(packet.g2()?);
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(StylesheetPropertyEntry {
            unknown: packet.g1()?,
            key_hash: packet.g4s()?,
            value: packet.g4s()?,
        });
    }
    if !packet.is_done() {
        bail!("stylesheet {id} did not consume full payload");
    }
    Ok(StylesheetEntry {
        id,
        parent,
        entries,
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IdkOp {
    BodyPart(u8),
    Model(i32),
    Disable,
    RecolPair { src: u16, dst: u16 },
    RetexPair { src: u16, dst: u16 },
    Recol3(u16),
    Recol4(u16),
    RecolIndices(u16),
    RetexIndices(u16),
    Recol5Src(u16),
    Recol6Src(u16),
    Recol7Src(u16),
    Recol8Src(u16),
    Recol9Src(u16),
    Recol10Src(u16),
    Recol1Dst(u16),
    Recol2Dst(u16),
    Recol3Dst(u16),
    Recol4Dst(u16),
    Recol5Dst(u16),
    Recol6Dst(u16),
    Recol7Dst(u16),
    Recol8Dst(u16),
    Recol9Dst(u16),
    Recol10Dst(u16),
    HeadModel { slot: u8, model: i32 },
}

#[derive(Clone, Debug, Serialize)]
pub struct IdkEntry {
    pub id: u32,
    pub ops: Vec<IdkOp>,
}

pub fn parse_idk(id: u32, data: &[u8]) -> Result<IdkEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("idk {id} did not consume full payload");
                }
                return Ok(IdkEntry { id, ops });
            }
            1 => ops.push(IdkOp::BodyPart(packet.g1()?)),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(IdkOp::Model(packet.gsmart2or4null()?));
                }
            }
            3 => ops.push(IdkOp::Disable),
            40 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(IdkOp::RecolPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(IdkOp::RetexPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            42 => ops.push(IdkOp::Recol3(packet.g2()?)),
            43 => ops.push(IdkOp::Recol4(packet.g2()?)),
            44 => ops.push(IdkOp::RecolIndices(packet.g2()?)),
            45 => ops.push(IdkOp::RetexIndices(packet.g2()?)),
            46 => ops.push(IdkOp::Recol7Src(packet.g2()?)),
            47 => ops.push(IdkOp::Recol8Src(packet.g2()?)),
            48 => ops.push(IdkOp::Recol9Src(packet.g2()?)),
            49 => ops.push(IdkOp::Recol10Src(packet.g2()?)),
            50 => ops.push(IdkOp::Recol1Dst(packet.g2()?)),
            51 => ops.push(IdkOp::Recol2Dst(packet.g2()?)),
            52 => ops.push(IdkOp::Recol3Dst(packet.g2()?)),
            53 => ops.push(IdkOp::Recol4Dst(packet.g2()?)),
            54 => ops.push(IdkOp::Recol5Dst(packet.g2()?)),
            55 => ops.push(IdkOp::Recol6Dst(packet.g2()?)),
            56 => ops.push(IdkOp::Recol7Dst(packet.g2()?)),
            57 => ops.push(IdkOp::Recol8Dst(packet.g2()?)),
            58 => ops.push(IdkOp::Recol9Dst(packet.g2()?)),
            59 => ops.push(IdkOp::Recol10Dst(packet.g2()?)),
            code @ 60..=69 => {
                let slot = code - 59;
                ops.push(IdkOp::HeadModel {
                    slot,
                    model: packet.gsmart2or4null()?,
                });
            }
            opcode => bail!("unknown idk opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SpotOp {
    Model(i32),
    Anim(i32),
    HasAlpha,
    ResizeH(u16),
    ResizeV(u16),
    Rotation(u16),
    Ambient(u8),
    Contrast(u8),
    AllowLoop,
    HillChangeRotate,
    HillChangeRotateG2(u16),
    HillChangeRotateG4(i32),
    RecolPair { src: u16, dst: u16 },
    RetexPair { src: u16, dst: u16 },
    Recol3(u16),
    Recol4(u16),
    RecolIndices(u16),
    RetexIndices(u16),
    Unknown46,
    Recol8Src(u16),
    Recol9Src(u16),
    Recol10Src(u16),
    Recol1Dst(u16),
    Recol2Dst(u16),
    Recol3Dst(u16),
    Recol4Dst(u16),
    Recol5Dst(u16),
    Recol6Dst(u16),
    Recol7Dst(u16),
    Recol8Dst(u16),
    Recol9Dst(u16),
    Recol10Dst(u16),
}

#[derive(Clone, Debug, Serialize)]
pub struct SpotEntry {
    pub id: u32,
    pub ops: Vec<SpotOp>,
}

pub fn parse_spot(id: u32, data: &[u8]) -> Result<SpotEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("spot {id} did not consume full payload");
                }
                return Ok(SpotEntry { id, ops });
            }
            1 => ops.push(SpotOp::Model(packet.gsmart2or4null()?)),
            2 => ops.push(SpotOp::Anim(packet.gsmart2or4null()?)),
            3 => ops.push(SpotOp::HasAlpha),
            4 => ops.push(SpotOp::ResizeH(packet.g2()?)),
            5 => ops.push(SpotOp::ResizeV(packet.g2()?)),
            6 => ops.push(SpotOp::Rotation(packet.g2()?)),
            7 => ops.push(SpotOp::Ambient(packet.g1()?)),
            8 => ops.push(SpotOp::Contrast(packet.g1()?)),
            10 => ops.push(SpotOp::AllowLoop),
            9 => ops.push(SpotOp::HillChangeRotate),
            15 => ops.push(SpotOp::HillChangeRotateG2(packet.g2()?)),
            16 => ops.push(SpotOp::HillChangeRotateG4(packet.g4s()?)),
            40 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(SpotOp::RecolPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(SpotOp::RetexPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            42 => ops.push(SpotOp::Recol3(packet.g2()?)),
            43 => ops.push(SpotOp::Recol4(packet.g2()?)),
            44 => ops.push(SpotOp::RecolIndices(packet.g2()?)),
            45 => ops.push(SpotOp::RetexIndices(packet.g2()?)),
            46 => ops.push(SpotOp::Unknown46),
            47 => ops.push(SpotOp::Recol8Src(packet.g2()?)),
            48 => ops.push(SpotOp::Recol9Src(packet.g2()?)),
            49 => ops.push(SpotOp::Recol10Src(packet.g2()?)),
            50 => ops.push(SpotOp::Recol1Dst(packet.g2()?)),
            51 => ops.push(SpotOp::Recol2Dst(packet.g2()?)),
            52 => ops.push(SpotOp::Recol3Dst(packet.g2()?)),
            53 => ops.push(SpotOp::Recol4Dst(packet.g2()?)),
            54 => ops.push(SpotOp::Recol5Dst(packet.g2()?)),
            55 => ops.push(SpotOp::Recol6Dst(packet.g2()?)),
            56 => ops.push(SpotOp::Recol7Dst(packet.g2()?)),
            57 => ops.push(SpotOp::Recol8Dst(packet.g2()?)),
            58 => ops.push(SpotOp::Recol9Dst(packet.g2()?)),
            59 => ops.push(SpotOp::Recol10Dst(packet.g2()?)),
            opcode => bail!("unknown spot opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum QuestOp {
    Name(String),
    SortName(String),
    ProgressVarp {
        varp_id: u16,
        a: i32,
        b: i32,
    },
    ProgressVarbit {
        varbit_id: u16,
        a: i32,
        b: i32,
    },
    Unknown5(u16),
    Type(u8),
    Difficulty(u8),
    Members,
    Points(u8),
    Unknown10(i32),
    Unknown12(i32),
    QuestReq(u16),
    StatReq {
        stat_id: u8,
        level: u8,
    },
    PointsReq(u16),
    Icon(i32),
    VarpReq {
        varp_id: i32,
        min: i32,
        max: i32,
        text: String,
    },
    VarbitReq {
        varbit_id: i32,
        min: i32,
        max: i32,
        text: String,
    },
    Param {
        param_id: u32,
        value: ScalarValue,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct QuestEntry {
    pub id: u32,
    pub ops: Vec<QuestOp>,
}

pub fn parse_quest(id: u32, data: &[u8]) -> Result<QuestEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("quest {id} did not consume full payload");
                }
                return Ok(QuestEntry { id, ops });
            }
            1 => ops.push(QuestOp::Name(packet.gjstr2()?)),
            2 => ops.push(QuestOp::SortName(packet.gjstr2()?)),
            3 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::ProgressVarp {
                        varp_id: packet.g2()?,
                        a: packet.g4s()?,
                        b: packet.g4s()?,
                    });
                }
            }
            4 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::ProgressVarbit {
                        varbit_id: packet.g2()?,
                        a: packet.g4s()?,
                        b: packet.g4s()?,
                    });
                }
            }
            5 => ops.push(QuestOp::Unknown5(packet.g2()?)),
            6 => ops.push(QuestOp::Type(packet.g1()?)),
            7 => ops.push(QuestOp::Difficulty(packet.g1()?)),
            8 => ops.push(QuestOp::Members),
            9 => ops.push(QuestOp::Points(packet.g1()?)),
            10 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::Unknown10(packet.g4s()?));
                }
            }
            12 => ops.push(QuestOp::Unknown12(packet.g4s()?)),
            13 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::QuestReq(packet.g2()?));
                }
            }
            14 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::StatReq {
                        stat_id: packet.g1()?,
                        level: packet.g1()?,
                    });
                }
            }
            15 => ops.push(QuestOp::PointsReq(packet.g2()?)),
            17 => ops.push(QuestOp::Icon(packet.gsmart2or4null()?)),
            18 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::VarpReq {
                        varp_id: packet.g4s()?,
                        min: packet.g4s()?,
                        max: packet.g4s()?,
                        text: packet.gjstr()?,
                    });
                }
            }
            19 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::VarbitReq {
                        varbit_id: packet.g4s()?,
                        min: packet.g4s()?,
                        max: packet.g4s()?,
                        text: packet.gjstr()?,
                    });
                }
            }
            249 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let is_string = packet.g1()? == 1;
                    let param_id = packet.g3()?;
                    let value = if is_string {
                        ScalarValue::Str(packet.gjstr()?)
                    } else {
                        ScalarValue::Int(packet.g4s()?)
                    };
                    ops.push(QuestOp::Param { param_id, value });
                }
            }
            opcode => bail!("unknown quest opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SeqFrameEntry {
    pub anim_id: u16,
    pub frame_id: u16,
    pub delay: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeqIFrameEntry {
    pub anim_id: u16,
    pub frame_id: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeqSoundEntry {
    pub slot: u16,
    pub type_id: u32,
    pub loops: u8,
    pub range: u8,
    pub extra: Vec<u16>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SeqUnknown19 {
    G1G1 { a: u8, b: u8 },
    G2G1 { a: u16, b: u8 },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SeqUnknown20 {
    G1G2G2 { a: u8, b: u16, c: u16 },
    G2G2G2 { a: u16, b: u16, c: u16 },
}

#[derive(Clone, Debug, Serialize)]
// Animation sequence flags are inherently many independent booleans.
#[allow(clippy::struct_excessive_bools)]
pub struct SeqEntry {
    pub id: u32,
    pub frames: Vec<SeqFrameEntry>,
    pub loopframes: Option<u16>,
    pub walkmerge: Vec<u16>,
    pub stretches: bool,
    pub priority: Option<u8>,
    pub lefthand_raw: Option<u16>,
    pub righthand_raw: Option<u16>,
    pub loopcount: Option<u8>,
    pub preanim_move: Option<u8>,
    pub postanim_move: Option<u8>,
    pub replacemode: Option<u8>,
    pub iframes: Vec<SeqIFrameEntry>,
    pub sounds: Vec<SeqSoundEntry>,
    pub unknown14: bool,
    pub unknown15: bool,
    pub unknown16: bool,
    pub unknown17: Option<u8>,
    pub unknown18: bool,
    pub unknown19: Vec<SeqUnknown19>,
    pub unknown20: Vec<SeqUnknown20>,
    pub unknown22: Option<u8>,
    pub unknown23: Option<u16>,
    pub group: Option<u16>,
    pub keyframeset: Option<u16>,
    pub keyframerange: Option<(u16, u16)>,
    pub unknown27: Option<i8>,
    pub params: Vec<StructParamEntry>,
}

pub fn parse_seq(id: u32, data: &[u8]) -> Result<SeqEntry> {
    let mut packet = Packet::new(data);
    let mut frames = Vec::new();
    let mut loopframes = None;
    let mut walkmerge = Vec::new();
    let mut stretches = false;
    let mut priority = None;
    let mut lefthand_raw = None;
    let mut righthand_raw = None;
    let mut loopcount = None;
    let mut preanim_move = None;
    let mut postanim_move = None;
    let mut replacemode = None;
    let mut iframes = Vec::new();
    let mut sounds = Vec::new();
    let mut unknown14 = false;
    let mut unknown15 = false;
    let mut unknown16 = false;
    let mut unknown17 = None;
    let mut unknown18 = false;
    let mut unknown19 = Vec::new();
    let mut unknown20 = Vec::new();
    let mut unknown22 = None;
    let mut unknown23 = None;
    let mut group = None;
    let mut keyframeset = None;
    let mut keyframerange = None;
    let mut unknown27 = None;
    let mut params = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("seq {id} did not consume full payload");
                }
                return Ok(SeqEntry {
                    id,
                    frames,
                    loopframes,
                    walkmerge,
                    stretches,
                    priority,
                    lefthand_raw,
                    righthand_raw,
                    loopcount,
                    preanim_move,
                    postanim_move,
                    replacemode,
                    iframes,
                    sounds,
                    unknown14,
                    unknown15,
                    unknown16,
                    unknown17,
                    unknown18,
                    unknown19,
                    unknown20,
                    unknown22,
                    unknown23,
                    group,
                    keyframeset,
                    keyframerange,
                    unknown27,
                    params,
                });
            }
            1 => {
                let count = usize::from(packet.g2()?);
                let mut delays = Vec::with_capacity(count);
                let mut frame_ids = Vec::with_capacity(count);
                let mut anim_ids = Vec::with_capacity(count);
                for _ in 0..count {
                    delays.push(packet.g2()?);
                }
                for _ in 0..count {
                    frame_ids.push(packet.g2()?);
                }
                for _ in 0..count {
                    anim_ids.push(packet.g2()?);
                }
                for i in 0..count {
                    frames.push(SeqFrameEntry {
                        anim_id: anim_ids[i],
                        frame_id: frame_ids[i],
                        delay: delays[i],
                    });
                }
            }
            2 => loopframes = Some(packet.g2()?),
            3 => {
                let count = usize::from(packet.gsmart1or2()?);
                for _ in 0..count {
                    walkmerge.push(packet.gsmart1or2()?);
                }
            }
            4 => stretches = true,
            5 => priority = Some(packet.g1()?),
            6 => lefthand_raw = Some(packet.g2()?),
            7 => righthand_raw = Some(packet.g2()?),
            8 => loopcount = Some(packet.g1()?),
            9 => preanim_move = Some(packet.g1()?),
            10 => postanim_move = Some(packet.g1()?),
            11 => replacemode = Some(packet.g1()?),
            12 => {
                let count = usize::from(packet.g1()?);
                let mut frame_ids = Vec::with_capacity(count);
                let mut anim_ids = Vec::with_capacity(count);
                for _ in 0..count {
                    frame_ids.push(packet.g2()?);
                }
                for _ in 0..count {
                    anim_ids.push(packet.g2()?);
                }
                for i in 0..count {
                    iframes.push(SeqIFrameEntry {
                        anim_id: anim_ids[i],
                        frame_id: frame_ids[i],
                    });
                }
            }
            112 => {
                let count = usize::from(packet.g2()?);
                let mut frame_ids = Vec::with_capacity(count);
                let mut anim_ids = Vec::with_capacity(count);
                for _ in 0..count {
                    frame_ids.push(packet.g2()?);
                }
                for _ in 0..count {
                    anim_ids.push(packet.g2()?);
                }
                for i in 0..count {
                    iframes.push(SeqIFrameEntry {
                        anim_id: anim_ids[i],
                        frame_id: frame_ids[i],
                    });
                }
            }
            13 => {
                let count = usize::from(packet.g2()?);
                for slot in 0..count {
                    let inner = usize::from(packet.g1()?);
                    if inner == 0 {
                        continue;
                    }
                    let value = packet.g3()?;
                    let mut extra = Vec::with_capacity(inner.saturating_sub(1));
                    for _ in 1..inner {
                        extra.push(packet.g2()?);
                    }
                    sounds.push(SeqSoundEntry {
                        slot: u16::try_from(slot).context("seq sound slot overflow")?,
                        type_id: value >> 8,
                        loops: u8::try_from((value >> 4) & 7)
                            .context("seq sound loops overflow")?,
                        range: u8::try_from(value & 0xF).context("seq sound range overflow")?,
                        extra,
                    });
                }
            }
            14 => unknown14 = true,
            15 => unknown15 = true,
            16 => unknown16 = true,
            17 => unknown17 = Some(packet.g1()?),
            18 => unknown18 = true,
            19 => unknown19.push(SeqUnknown19::G1G1 {
                a: packet.g1()?,
                b: packet.g1()?,
            }),
            119 => unknown19.push(SeqUnknown19::G2G1 {
                a: packet.g2()?,
                b: packet.g1()?,
            }),
            20 => unknown20.push(SeqUnknown20::G1G2G2 {
                a: packet.g1()?,
                b: packet.g2()?,
                c: packet.g2()?,
            }),
            120 => unknown20.push(SeqUnknown20::G2G2G2 {
                a: packet.g2()?,
                b: packet.g2()?,
                c: packet.g2()?,
            }),
            22 => unknown22 = Some(packet.g1()?),
            23 => unknown23 = Some(packet.g2()?),
            24 => group = Some(packet.g2()?),
            25 => keyframeset = Some(packet.g2()?),
            26 => keyframerange = Some((packet.g2()?, packet.g2()?)),
            27 => unknown27 = Some(packet.g1s()?),
            249 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let is_string = packet.g1()? == 1;
                    let param_id = packet.g3()?;
                    let value = if is_string {
                        ScalarValue::Str(packet.gjstr()?)
                    } else {
                        ScalarValue::Int(packet.g4s()?)
                    };
                    params.push(StructParamEntry { param_id, value });
                }
            }
            opcode => bail!("unknown seq opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct OpListEntry {
    pub id: u32,
    pub ops: Vec<String>,
}

pub fn parse_achievement(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("achievement {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!("name={}", packet.gjstr2()?)),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("desc={},{}", packet.g1()?, packet.gjstr2()?));
                }
            }
            3 => ops.push(format!("category={}", packet.g2()?)),
            4 => ops.push(format!("sprite={}", packet.gsmart2or4null()?)),
            5 => ops.push(format!("runescore={}", packet.g1()?)),
            6 => ops.push(format!("graceday={}", packet.g2()?)),
            7 => ops.push(format!("reward={}", packet.gjstr2()?)),
            8 => parse_achievement_stat_var_list(&mut packet, &mut ops, "statprereq", true)?,
            9 => parse_achievement_var_list(&mut packet, &mut ops, "varpprereq")?,
            10 => parse_achievement_var_list(&mut packet, &mut ops, "varbitprereq")?,
            11 => parse_achievement_ref_list(&mut packet, &mut ops, "achievementprereq")?,
            12 => parse_achievement_stat_var_list(&mut packet, &mut ops, "statreq", true)?,
            13 => parse_achievement_var_list(&mut packet, &mut ops, "varpreq")?,
            14 => parse_achievement_var_list(&mut packet, &mut ops, "varbitreq")?,
            15 => parse_achievement_ref_list(&mut packet, &mut ops, "achievementreq")?,
            16 => ops.push(format!("subcat={}", packet.g2()?)),
            17 => ops.push(String::from("locked=yes")),
            18 => ops.push(format!("hide={}", packet.g1()?)),
            19 => ops.push(String::from("members=no")),
            20 => parse_achievement_ref_list(&mut packet, &mut ops, "questprereq")?,
            21 => parse_achievement_ref_list(&mut packet, &mut ops, "questreq")?,
            22 => parse_achievement_testbit_list(&mut packet, &mut ops, "varptestbitprereq")?,
            23 => parse_achievement_testbit_list(&mut packet, &mut ops, "varptestbitreq")?,
            24 => parse_achievement_testbit_list(&mut packet, &mut ops, "varbittestbitprereq")?,
            25 => parse_achievement_testbit_list(&mut packet, &mut ops, "varbittestbitreq")?,
            26 => ops.push(format!("unknown26={}", packet.g2()?)),
            27 => ops.push(String::from("checklist=yes")),
            28 => {
                let count = usize::from(packet.gsmart1or2()?);
                for _ in 0..count {
                    ops.push(format!("unknown28={}", packet.gsmart1or2()?));
                }
            }
            29 => ops.push(format!("unknown29={}", packet.g1()?)),
            30 => {
                let count = usize::from(packet.gsmart1or2()?);
                for _ in 0..count {
                    ops.push(format!("unknown30={}", packet.gsmart1or2()?));
                }
            }
            31 => ops.push(format!("unknown31={}", packet.g1()?)),
            32 => ops.push(format!(
                "unknown32={},{},{}",
                packet.g1()?,
                packet.g1()?,
                packet.g1()?
            )),
            opcode => bail!("unknown achievement opcode {opcode} in {id}"),
        }
    }
}

pub fn parse_bas(id: u32, data: &[u8], build: u32) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("bas {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!(
                "readyanim={},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            2 => ops.push(format!("crawlanim={}", packet.gsmart2or4null()?)),
            3 => ops.push(format!("crawlanim_b={}", packet.gsmart2or4null()?)),
            4 => ops.push(format!("crawlanim_l={}", packet.gsmart2or4null()?)),
            5 => ops.push(format!("crawlanim_r={}", packet.gsmart2or4null()?)),
            6 => ops.push(format!("runanim={}", packet.gsmart2or4null()?)),
            7 => ops.push(format!("runanim_b={}", packet.gsmart2or4null()?)),
            8 => ops.push(format!("runanim_l={}", packet.gsmart2or4null()?)),
            9 => ops.push(format!("runanim_r={}", packet.gsmart2or4null()?)),
            26 => ops.push(format!("hillrotate={},{}", packet.g1()?, packet.g1()?)),
            27 => ops.push(format!(
                "unknown27={},{},{},{},{},{},{}",
                packet.g1()?,
                packet.g2s()?,
                packet.g2s()?,
                packet.g2s()?,
                packet.g2s()?,
                packet.g2s()?,
                packet.g2s()?
            )),
            28 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("unknown28={}", packet.g1()?));
                }
            }
            29 => ops.push(format!("turnspeed={}", packet.g1()?)),
            30 => ops.push(format!("turnacceleration={}", packet.g2()?)),
            31 => ops.push(format!("unknown31={}", packet.g1()?)),
            32 => ops.push(format!("unknown32={}", packet.g2()?)),
            33 => ops.push(format!("unknown33={}", packet.g2s()?)),
            34 => ops.push(format!("unknown34={}", packet.g1()?)),
            35 => ops.push(format!("unknown35={}", packet.g2()?)),
            36 => ops.push(format!("unknown36={}", packet.g2s()?)),
            37 => ops.push(format!("walkspeed={}", packet.g1()?)),
            38 => ops.push(format!("readyanim_l={}", packet.gsmart2or4null()?)),
            39 => ops.push(format!("readyanim_r={}", packet.gsmart2or4null()?)),
            40 => ops.push(format!("walkanim_b={}", packet.gsmart2or4null()?)),
            41 => ops.push(format!("walkanim_l={}", packet.gsmart2or4null()?)),
            42 => ops.push(format!("walkanim_r={}", packet.gsmart2or4null()?)),
            43 => ops.push(format!("unknown43={}", packet.g2()?)),
            44 => ops.push(format!("unknown44={}", packet.g2()?)),
            45 => ops.push(format!("overlayheight={}", packet.g2()?)),
            46 => ops.push(format!("crawlturn_l={}", packet.gsmart2or4null()?)),
            47 => ops.push(format!("crawlturn_r={}", packet.gsmart2or4null()?)),
            48 => ops.push(format!("runturn_l={}", packet.gsmart2or4null()?)),
            49 => ops.push(format!("runturn_r={}", packet.gsmart2or4null()?)),
            50 => ops.push(format!("walkturn_l={}", packet.gsmart2or4null()?)),
            51 => ops.push(format!("walkturn_r={}", packet.gsmart2or4null()?)),
            52 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let mut line = format!(
                        "randomreadyanim={},{}",
                        packet.gsmart2or4null()?,
                        packet.g1()?
                    );
                    if build >= 916 {
                        let unknown_count = usize::from(packet.g1()?);
                        for _ in 0..unknown_count {
                            let _ = write!(line, ",{}", packet.g1()?);
                        }
                    }
                    ops.push(line);
                }
            }
            53 => ops.push(String::from("unknown53=no")),
            54 => ops.push(format!("hillrotatelimit={},{}", packet.g1()?, packet.g1()?)),
            55 => ops.push(format!("unknown55={},{}", packet.g1()?, packet.g2()?)),
            56 => ops.push(format!(
                "unknown56={},{},{},{}",
                packet.g1()?,
                packet.g2s()?,
                packet.g2s()?,
                packet.g2s()?
            )),
            opcode => bail!("unknown bas opcode {opcode} in {id}"),
        }
    }
}

pub fn parse_mel(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("mel {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!("graphic={}", packet.gsmart2or4null()?)),
            2 => ops.push(format!("unknown2={}", packet.gsmart2or4null()?)),
            3 => ops.push(format!("text={}", packet.gjstr()?)),
            4 => ops.push(format!("colour={}", packet.g3()?)),
            5 => ops.push(format!("unknown5={}", packet.g3()?)),
            6 => ops.push(format!("size={}", packet.g1()?)),
            7 => ops.push(format!("vis={}", packet.g1()?)),
            8 => ops.push(format!("unknown8={}", packet.g1()?)),
            9 => {
                let varbit = packet.g2null()?;
                let varp = packet.g2null()?;
                let min = packet.g4s()?;
                let max = packet.g4s()?;
                if varbit != -1 && varp != -1 {
                    bail!("mel {id} has both varbit and varp in opcode 9");
                }
                if varp != -1 {
                    ops.push(format!("condition=varp:{varp},{min},{max}"));
                } else if varbit != -1 {
                    ops.push(format!("condition=varbit:{varbit},{min},{max}"));
                }
            }
            10 => ops.push(format!("op1={}", packet.gjstr()?)),
            11 => ops.push(format!("op2={}", packet.gjstr()?)),
            12 => ops.push(format!("op3={}", packet.gjstr()?)),
            13 => ops.push(format!("op4={}", packet.gjstr()?)),
            14 => ops.push(format!("op5={}", packet.gjstr()?)),
            15 => {
                let count1 = usize::from(packet.g1()?);
                for _ in 0..(count1 * 2) {
                    ops.push(format!("unknown15a={}", packet.g2s()?));
                }
                ops.push(format!("unknown15b={}", packet.g4s()?));
                let count2 = usize::from(packet.g1()?);
                for _ in 0..count2 {
                    ops.push(format!("unknown15c={}", packet.g4s()?));
                }
                for _ in 0..count1 {
                    ops.push(format!("unknown15d={}", packet.g1s()?));
                }
            }
            16 => ops.push(String::from("listable=no")),
            17 => ops.push(format!("opbase={}", packet.gjstr()?)),
            18 => ops.push(format!("unknown18={}", packet.gsmart2or4null()?)),
            19 => ops.push(format!("category={}", packet.g2()?)),
            20 => ops.push(format!(
                "unknown20={},{},{},{}",
                packet.g2null()?,
                packet.g2null()?,
                packet.g4s()?,
                packet.g4s()?
            )),
            21 => ops.push(format!("unknown21={}", packet.g4s()?)),
            22 => ops.push(format!("unknown22={}", packet.g4s()?)),
            23 => ops.push(format!(
                "unknown23={},{},{}",
                packet.g1()?,
                packet.g1()?,
                packet.g1()?
            )),
            24 => ops.push(format!("unknown24={},{}", packet.g2s()?, packet.g2s()?)),
            25 => ops.push(format!("unknown25={}", packet.gsmart2or4null()?)),
            26 => parse_multimel(&mut packet, &mut ops, false)?,
            27 => parse_multimel(&mut packet, &mut ops, true)?,
            28 => ops.push(format!("unknown28={}", packet.g1()?)),
            29 => ops.push(format!("alignx={}", packet.g1()?)),
            30 => ops.push(format!("aligny={}", packet.g1()?)),
            249 => parse_param_ops(&mut packet, &mut ops)?,
            opcode => bail!("unknown mel opcode {opcode} in {id}"),
        }
    }
}

pub fn parse_water(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("water {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!("unknown1={}", packet.g2()?)),
            2 => ops.push(format!("normal_map_material1_scale={}", packet.g2()?)),
            3 => ops.push(format!("unknown3={}", packet.g2()?)),
            4 => ops.push(format!("normal_map_material2_scale={}", packet.g2()?)),
            5 => ops.push(format!("reflection_strength={}", packet.g2()?)),
            6 => ops.push(format!("unknown6=0x{:06x}", packet.g3()?)),
            7 => ops.push(format!("unknown7={},{}", packet.g2()?, packet.g2()?)),
            8 => ops.push(format!("unknown8={}", packet.g2()?)),
            9 => ops.push(format!("water_foam_scale={}", packet.g2()?)),
            10 => ops.push(format!("foam_material_scale={}", packet.g2()?)),
            11 => ops.push(format!("unknown11={}", packet.g2()?)),
            12 => ops.push(format!("basergba=0x{:08x}", packet.g4s()? as u32)),
            13 => ops.push(format!("unknown13={}", packet.g2()?)),
            14 => ops.push(format!("water_depth_foam={}", packet.g2()?)),
            15 => ops.push(format!("unknown15={}", packet.g4s()?)),
            16 => ops.push(format!("unknown16={}", packet.g2()?)),
            17 => ops.push(format!("unknown17={}", packet.g2()?)),
            18 => ops.push(format!("unknown18={}", packet.g1()?)),
            19 => ops.push(format!("unknown19={}", packet.g1()?)),
            20 => ops.push(format!("unknown20={}", packet.g2()?)),
            21 => ops.push(format!("unknown21={}", packet.g2()?)),
            22 => ops.push(format!("unknown22={}", packet.g2()?)),
            23 => ops.push(format!("unknown23={}", packet.g2()?)),
            24 => ops.push(format!("unknown24={}", packet.g1()?)),
            25 => ops.push(format!("specular_shininess={}", packet.g2()?)),
            26 => ops.push(format!(
                "unknown26={},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g2()?
            )),
            27 => ops.push(format!("specular_factor={}", packet.g2()?)),
            28 => ops.push(format!("unknown28={}", packet.g2()?)),
            29 => ops.push(format!("normal_map_material1={}", packet.g2()?)),
            30 => ops.push(format!("normal_map_material2={}", packet.g2()?)),
            31 => ops.push(format!("normal_map_material3={}", packet.g2()?)),
            32 => ops.push(format!("normal_map_material3_scale={}", packet.g2()?)),
            opcode @ 33..=80 => decode_normal_map_params(&mut packet, &mut ops, opcode)?,
            81 => ops.push(format!(
                "still_water_normal_strength={}",
                gfloat_be(&mut packet)?
            )),
            82 => ops.push(format!("flow_noise={}", gfloat_be(&mut packet)?)),
            83 => ops.push(format!("fresnel_bias={}", gfloat_be(&mut packet)?)),
            84 => ops.push(format!("unknown84={}", gfloat_be(&mut packet)?)),
            85 => ops.push(format!("override_default_water_type={}", packet.g1()?)),
            86 => ops.push(format!("emisive_map_material={}", packet.g2()?)),
            87 => ops.push(format!("emissive_map_material_scale={}", packet.g2()?)),
            88 => ops.push(format!(
                "emissive_uv_scale={},{}",
                gfloat_be(&mut packet)?,
                gfloat_be(&mut packet)?
            )),
            89 => ops.push(format!("emissive_rgb={}", packet.g4s()?)),
            90 => ops.push(format!("emissive_scale={}", gfloat_be(&mut packet)?)),
            91 => ops.push(format!(
                "emissive_map_refraction_depth={}",
                gfloat_be(&mut packet)?
            )),
            92 => ops.push(format!("emissive_map_mode={}", packet.g1()?)),
            93 => ops.push(format!("emissive_source={}", gfloat_be(&mut packet)?)),
            94 => ops.push(format!("emissive_flow_speed={}", gfloat_be(&mut packet)?)),
            95 => ops.push(format!(
                "emissive_flow_rotation_degrees={}",
                gfloat_be(&mut packet)?
            )),
            96 => ops.push(format!("emissive_uv_mode={}", packet.g1()?)),
            97 => ops.push(format!(
                "extinction_rgb_depth_metres={},{},{}",
                gfloat_be(&mut packet)?,
                gfloat_be(&mut packet)?,
                gfloat_be(&mut packet)?
            )),
            98 => ops.push(format!("extinction_opaque_water_colour={}", packet.g4s()?)),
            99 => ops.push(format!(
                "extinction_visibility_metres={}",
                gfloat_be(&mut packet)?
            )),
            100 => ops.push(format!("caustics_scale={}", gfloat_be(&mut packet)?)),
            101 => ops.push(format!(
                "caustics_refraction_scale={}",
                gfloat_be(&mut packet)?
            )),
            102 => ops.push(format!(
                "caustics_depth_fade_cutoff={}",
                gfloat_be(&mut packet)?
            )),
            103 => ops.push(format!(
                "caustics_depth_fade_scale={}",
                gfloat_be(&mut packet)?
            )),
            104 => ops.push(format!(
                "caustics_edge_fade_start={}",
                gfloat_be(&mut packet)?
            )),
            105 => ops.push(format!(
                "caustics_edge_fade_end={}",
                gfloat_be(&mut packet)?
            )),
            106 => ops.push(format!(
                "caustics_over_water_fade_start={}",
                gfloat_be(&mut packet)?
            )),
            107 => ops.push(format!(
                "caustics_over_water_fade_end={}",
                gfloat_be(&mut packet)?
            )),
            108 => ops.push(format!("emissive_blend={}", gfloat_be(&mut packet)?)),
            opcode => bail!("unknown water opcode {opcode} in {id}"),
        }
    }
}

pub fn parse_material(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let version = packet.g1()?;
    let mut ops = vec![format!("version={version}")];

    match version {
        0 => parse_material_v0(&mut packet, &mut ops)?,
        1 | 2 => parse_material_v1(&mut packet, &mut ops)?,
        _ => bail!("unsupported material version {version} in {id}"),
    }

    if !packet.is_done() {
        bail!("material {id} did not consume full payload (pos {} of {})", packet.pos(), data.len());
    }
    Ok(OpListEntry { id, ops })
}

pub fn parse_loc(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("loc {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => {
                let shape_count = usize::from(packet.g1()?);
                for _ in 0..shape_count {
                    let shape = packet.g1s()?;
                    let model_count = usize::from(packet.g1()?);
                    for _ in 0..model_count {
                        ops.push(format!("model={shape},{}", packet.gsmart2or4null()?));
                    }
                }
            }
            2 => ops.push(format!("name={}", packet.gjstr()?)),
            3 => ops.push(format!("desc={}", packet.gjstr()?)),
            5 => {
                let shape_count1 = usize::from(packet.g1()?);
                for _ in 0..shape_count1 {
                    let shape = packet.g1s()?;
                    let model_count = usize::from(packet.g1()?);
                    for _ in 0..model_count {
                        ops.push(format!("modela={shape},{}", packet.gsmart2or4null()?));
                    }
                }
                let shape_count2 = usize::from(packet.g1()?);
                for _ in 0..shape_count2 {
                    let shape = packet.g1s()?;
                    let model_count = usize::from(packet.g1()?);
                    for _ in 0..model_count {
                        ops.push(format!("modelb={shape},{}", packet.gsmart2or4null()?));
                    }
                }
            }
            14 => ops.push(format!("width={}", packet.g1()?)),
            15 => ops.push(format!("length={}", packet.g1()?)),
            17 => ops.push(String::from("blockwalk=no")),
            18 => ops.push(String::from("blockrange=no")),
            19 => ops.push(format!("active={}", packet.g1()?)),
            21 => ops.push(String::from("hillskew=yes")),
            22 => ops.push(String::from("sharelight=yes")),
            23 => ops.push(String::from("occlude=yes")),
            24 => ops.push(format!("anim={}", packet.gsmart2or4null()?)),
            25 => ops.push(String::from("hasalpha=yes")),
            27 => ops.push(String::from("blockwalk=yes")),
            28 => ops.push(format!("wallwidth={}", packet.g1()?)),
            29 => ops.push(format!("ambient={}", packet.g1s()?)),
            30 => ops.push(format!("op1={}", packet.gjstr()?)),
            31 => ops.push(format!("op2={}", packet.gjstr()?)),
            32 => ops.push(format!("op3={}", packet.gjstr()?)),
            33 => ops.push(format!("op4={}", packet.gjstr()?)),
            34 => ops.push(format!("op5={}", packet.gjstr()?)),
            39 => ops.push(format!("contrast={}", packet.g1s()?)),
            40 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("recol{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("recol{}d={}", i + 1, packet.g2()?));
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("retex{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("retex{}d={}", i + 1, packet.g2()?));
                }
            }
            42 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("unknown42={}", packet.g1s()?));
                }
            }
            44 => ops.push(format!("recolindices={}", packet.g2()?)),
            45 => ops.push(format!("retexindices={}", packet.g2()?)),
            60 => ops.push(format!("mapfunction={}", packet.g2()?)),
            61 => ops.push(format!("category={}", packet.g2()?)),
            62 => ops.push(String::from("mirror=yes")),
            64 => ops.push(String::from("shadow=no")),
            65 => ops.push(format!("resizex={}", packet.g2()?)),
            66 => ops.push(format!("resizey={}", packet.g2()?)),
            67 => ops.push(format!("resizez={}", packet.g2()?)),
            68 => ops.push(format!("mapscene={}", packet.g2()?)),
            69 => {
                let blocked = packet.g1()?;
                let mut dirs = Vec::new();
                if (blocked & 1) == 0 {
                    dirs.push("north");
                }
                if (blocked & 2) == 0 {
                    dirs.push("east");
                }
                if (blocked & 4) == 0 {
                    dirs.push("south");
                }
                if (blocked & 8) == 0 {
                    dirs.push("west");
                }
                if (blocked >> 4) != 0 {
                    bail!("invalid loc blocked value {blocked} in {id}");
                }
                ops.push(format!("forceapproach={}", dirs.join(",")));
            }
            70 => ops.push(format!("offsetx={}", packet.g2s()?)),
            71 => ops.push(format!("offsety={}", packet.g2s()?)),
            72 => ops.push(format!("offsetz={}", packet.g2s()?)),
            73 => ops.push(String::from("forcedecor=yes")),
            74 => ops.push(String::from("breakroutefinding=yes")),
            75 => ops.push(format!("raiseobject={}", packet.g1()?)),
            77 => parse_loc_multi(&mut packet, &mut ops, false)?,
            78 => ops.push(format!("bgsound={},{}", packet.g2()?, packet.g1()?)),
            79 => {
                let mut line = format!(
                    "randomsound={},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    packet.g1()?
                );
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let _ = write!(line, ",{}", packet.g2()?);
                }
                ops.push(line);
            }
            81 => ops.push(format!("hillchange=tree_skew,{}", packet.g1()?)),
            82 => ops.push(String::from("istexture=yes")),
            88 => ops.push(String::from("hardshadow=no")),
            89 => ops.push(String::from("randomanimframe=no")),
            90 => ops.push(String::from("unknown90=yes")),
            91 => ops.push(String::from("members=yes")),
            92 => parse_loc_multi(&mut packet, &mut ops, true)?,
            93 => ops.push(format!("hillchange=rotate,{}", packet.g2()?)),
            94 => ops.push(String::from("hillchange=ceiling_skew")),
            95 => ops.push(format!("hillchange=skew_to_fit,{}", packet.g2()?)),
            96 => ops.push(String::from("unknown96=yes")),
            97 => ops.push(String::from("msirotate=yes")),
            98 => ops.push(String::from("unknown98=yes")),
            99 => ops.push(format!("cursor1={},{}", packet.g1()?, packet.g2()?)),
            100 => ops.push(format!("cursor2={},{}", packet.g1()?, packet.g2()?)),
            101 => ops.push(format!("msiangle={}", packet.g1()?)),
            102 => ops.push(format!("msi={}", packet.g2()?)),
            103 => ops.push(String::from("occlude=no")),
            104 => ops.push(format!("bgsoundvolume={}", packet.g1()?)),
            105 => ops.push(String::from("msimirror=yes")),
            106 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!(
                        "anim={},{}",
                        packet.gsmart2or4null()?,
                        packet.g1()?
                    ));
                }
            }
            107 => ops.push(format!("mapelement={}", packet.g2()?)),
            150 => ops.push(format!("membersop1={}", packet.gjstr()?)),
            151 => ops.push(format!("membersop2={}", packet.gjstr()?)),
            152 => ops.push(format!("membersop3={}", packet.gjstr()?)),
            153 => ops.push(format!("membersop4={}", packet.gjstr()?)),
            154 => ops.push(format!("membersop5={}", packet.gjstr()?)),
            160 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("quest={}", packet.g2()?));
                }
            }
            162 => ops.push(format!("hillchange=rotate,{}", packet.g4s()?)),
            163 => ops.push(format!(
                "tint={},{},{},{}",
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?
            )),
            164 => ops.push(format!("postoffsetx={}", packet.g2s()?)),
            165 => ops.push(format!("postoffsety={}", packet.g2s()?)),
            166 => ops.push(format!("postoffsetz={}", packet.g2s()?)),
            167 => ops.push(format!("unknown167={}", packet.g2()?)),
            168 => ops.push(String::from("unknown168=yes")),
            169 => ops.push(String::from("unknown169=yes")),
            170 => ops.push(format!("unknown170={}", packet.gsmart1or2()?)),
            171 => ops.push(format!("unknown171={}", packet.gsmart1or2()?)),
            173 => ops.push(format!("bgsoundrate={},{}", packet.g2()?, packet.g2()?)),
            177 => ops.push(String::from("unknown177=yes")),
            178 => ops.push(format!("bgsounddropoffrange={}", packet.g1()?)),
            179 => ops.push(String::from("unknown179=yes")),
            186 => ops.push(format!("unknown186={}", packet.g1()?)),
            188 => ops.push(String::from("unknown188=yes")),
            189 => ops.push(String::from("antimacro=yes")),
            190 => ops.push(format!("cursor1={}", packet.g2()?)),
            191 => ops.push(format!("cursor2={}", packet.g2()?)),
            192 => ops.push(format!("cursor3={}", packet.g2()?)),
            193 => ops.push(format!("cursor4={}", packet.g2()?)),
            194 => ops.push(format!("cursor5={}", packet.g2()?)),
            195 => ops.push(format!("cursor6={}", packet.g2()?)),
            196 => {
                let value = match packet.g1()? {
                    0 => "max",
                    1 => "high",
                    2 => "medium",
                    3 => "low",
                    4 => "min",
                    level => bail!("invalid minimumlodleveloverride value {level} in loc {id}"),
                };
                ops.push(format!("minimumlodleveloverride={value}"));
            }
            197 => ops.push(format!("indoorsoverride={}", packet.g1()?)),
            198 => ops.push(String::from("runetek5only=yes")),
            199 => ops.push(String::from("unknown199=no")),
            200 => ops.push(String::from("highdetailonly=yes")),
            201 => ops.push(format!(
                "custombounding={},{},{},{},{},{}",
                gsmart1or2s(&mut packet)?,
                gsmart1or2s(&mut packet)?,
                gsmart1or2s(&mut packet)?,
                gsmart1or2s(&mut packet)?,
                gsmart1or2s(&mut packet)?,
                gsmart1or2s(&mut packet)?
            )),
            202 => ops.push(format!("highlightoverride={}", packet.gsmart1or2()?)),
            203 => ops.push(String::from("unknown203=yes")),
            204 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!(
                        "vfx={},{},{},{},{},{},{},{}",
                        packet.g2()?,
                        packet.g1()?,
                        gfloat_be(&mut packet)?,
                        gfloat_be(&mut packet)?,
                        gfloat_be(&mut packet)?,
                        gfloat_be(&mut packet)?,
                        gfloat_be(&mut packet)?,
                        gfloat_be(&mut packet)?
                    ));
                }
            }
            205 => parse_multi_variants_block(&mut packet, &mut ops)?,
            249 => parse_param_ops(&mut packet, &mut ops)?,
            250 => ops.push(format!("bgsoundshape={}", packet.g1()?)),
            251 => ops.push(format!("bgsounddistancefiltered={}", packet.g1()?)),
            252 => ops.push(format!(
                "bgsounddistancefilterparams={},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g2()?
            )),
            253 => ops.push(format!("randomsoundshape={}", packet.g1()?)),
            254 => ops.push(format!("randomsounddistancefiltered={}", packet.g1()?)),
            255 => ops.push(format!(
                "randomsounddistancefilterparams={},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g2()?
            )),
            opcode => bail!("unknown loc opcode {opcode} in {id}"),
        }
    }
}

pub fn parse_npc(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("npc {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("model{}={}", i + 1, packet.gsmart2or4null()?));
                }
            }
            2 => ops.push(format!("name={}", packet.gjstr()?)),
            3 => ops.push(format!("desc={}", packet.gjstr()?)),
            12 => ops.push(format!("size={}", packet.g1()?)),
            13 => ops.push(format!("readyanim={}", packet.g2()?)),
            14 => ops.push(format!("walkanim={}", packet.g2()?)),
            15 => ops.push(format!("turnleftanim={}", packet.g2()?)),
            16 => ops.push(format!("turnrightanim={}", packet.g2()?)),
            17 => ops.push(format!(
                "walkanim={},{},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g2()?,
                packet.g2()?
            )),
            18 => ops.push(format!("category={}", packet.g2()?)),
            30 => ops.push(format!("op1={}", packet.gjstr()?)),
            31 => ops.push(format!("op2={}", packet.gjstr()?)),
            32 => ops.push(format!("op3={}", packet.gjstr()?)),
            33 => ops.push(format!("op4={}", packet.gjstr()?)),
            34 => ops.push(format!("op5={}", packet.gjstr()?)),
            39 => ops.push(format!("unknown39={}", packet.g1()?)),
            40 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("recol{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("recol{}d={}", i + 1, packet.g2()?));
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("retex{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("retex{}d={}", i + 1, packet.g2()?));
                }
            }
            42 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("unknown42={}", packet.g1s()?));
                }
            }
            44 => ops.push(format!("recolindices={}", packet.g2()?)),
            45 => ops.push(format!("retexindices={}", packet.g2()?)),
            60 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("head{}={}", i + 1, packet.gsmart2or4null()?));
                }
            }
            93 => ops.push(String::from("minimap=no")),
            95 => ops.push(format!("vislevel={}", packet.g2()?)),
            97 => ops.push(format!("resizeh={}", packet.g2()?)),
            98 => ops.push(format!("resizev={}", packet.g2()?)),
            99 => ops.push(String::from("alwaysontop=yes")),
            100 => ops.push(format!("ambient={}", packet.g1s()?)),
            101 => ops.push(format!("contrast={}", packet.g1s()?)),
            102 => {
                let filter = packet.g1()?;
                for i in 0..8_u32 {
                    if (filter & (1_u8 << i)) != 0 {
                        ops.push(format!(
                            "headicon{}={},{}",
                            i + 1,
                            packet.gsmart2or4null()?,
                            gsmart1or2null(&mut packet)?
                        ));
                    }
                }
            }
            103 => ops.push(format!("turnspeed={}", packet.g2()?)),
            106 => parse_npc_multi(&mut packet, &mut ops, false)?,
            107 => ops.push(String::from("active=no")),
            109 => ops.push(String::from("walksmoothing=no")),
            111 => ops.push(String::from("spotshadow=no")),
            113 => ops.push(format!(
                "spotshadowcolour={},{}",
                packet.g2()?,
                packet.g2()?
            )),
            114 => ops.push(format!(
                "spotshadowtrans={},{}",
                packet.g1s()?,
                packet.g1s()?
            )),
            115 => ops.push(format!("unknown115={},{}", packet.g1()?, packet.g1()?)),
            118 => parse_npc_multi(&mut packet, &mut ops, true)?,
            119 => ops.push(format!("unknown119={}", packet.g1s()?)),
            120 => ops.push(format!(
                "unknown120={},{},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g2()?,
                packet.g1()?
            )),
            121 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!(
                        "modeloffset{}={},{},{}",
                        packet.g1()?,
                        packet.g1s()?,
                        packet.g1s()?,
                        packet.g1s()?
                    ));
                }
            }
            122 => ops.push(format!("unknown122={}", packet.g2()?)),
            123 => ops.push(format!("overlayheight={}", packet.g2()?)),
            125 => ops.push(format!("respawndir={}", packet.g1s()?)),
            127 => ops.push(format!("bas={}", packet.g2()?)),
            128 => ops.push(format!("defaultmovemode={}", packet.g1()?)),
            134 => ops.push(format!(
                "bgsound={},{},{},{},{}",
                packet.g2null()?,
                packet.g2null()?,
                packet.g2null()?,
                packet.g2null()?,
                packet.g1()?
            )),
            135 => ops.push(format!("cursor1={},{}", packet.g1()?, packet.g2()?)),
            136 => ops.push(format!("cursor2={},{}", packet.g1()?, packet.g2()?)),
            137 => ops.push(format!("cursorattack={}", packet.g2()?)),
            138 => ops.push(format!("covermarker={}", packet.gsmart2or4null()?)),
            139 => ops.push(format!("unknown139={}", packet.gsmart2or4null()?)),
            140 => ops.push(format!("bgsoundvolume={}", packet.g1()?)),
            141 => ops.push(String::from("follower=yes")),
            142 => ops.push(format!("mapelement={}", packet.g2()?)),
            143 => ops.push(String::from("drawbelow=yes")),
            150 => ops.push(format!("membersop1={}", packet.gjstr()?)),
            151 => ops.push(format!("membersop2={}", packet.gjstr()?)),
            152 => ops.push(format!("membersop3={}", packet.gjstr()?)),
            153 => ops.push(format!("membersop4={}", packet.gjstr()?)),
            154 => ops.push(format!("membersop5={}", packet.gjstr()?)),
            155 => ops.push(format!(
                "tint={},{},{},{}",
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?
            )),
            158 => ops.push(String::from("reprioritiseattackop=yes")),
            159 => ops.push(String::from("reprioritiseattackop=no")),
            160 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("quest={}", packet.g2()?));
                }
            }
            162 => ops.push(String::from("unknown162=yes")),
            163 => ops.push(format!("picksize={}", packet.g1()?)),
            164 => ops.push(format!("bgsoundrate={},{}", packet.g2()?, packet.g2()?)),
            165 => ops.push(format!("picksizeshift={}", packet.g1()?)),
            168 => ops.push(format!("bgsoundsize={}", packet.g1()?)),
            169 => ops.push(String::from("antimacro=no")),
            170 => ops.push(format!("cursor1={}", packet.g2null()?)),
            171 => ops.push(format!("cursor2={}", packet.g2null()?)),
            172 => ops.push(format!("cursor3={}", packet.g2null()?)),
            173 => ops.push(format!("cursor4={}", packet.g2null()?)),
            174 => ops.push(format!("cursor5={}", packet.g2null()?)),
            175 => ops.push(format!("cursor6={}", packet.g2null()?)),
            178 => ops.push(String::from("unknown178=no")),
            179 => ops.push(format!(
                "clickbox={},{},{},{},{},{}",
                packet.gsmart1or2()?,
                packet.gsmart1or2()?,
                packet.gsmart1or2()?,
                packet.gsmart1or2()?,
                packet.gsmart1or2()?,
                packet.gsmart1or2()?
            )),
            180 => ops.push(format!("unknown180={}", packet.g1()?)),
            181 => ops.push(format!(
                "spotshadowtexture={},{}",
                packet.g2()?,
                packet.g1()?
            )),
            182 => ops.push(String::from("transmogfakenpc=yes")),
            184 => ops.push(format!("unknown184={}", packet.g1()?)),
            185 => ops.push(String::from("unknown185=no")),
            186 => parse_multi_variants_block(&mut packet, &mut ops)?,
            249 => parse_param_ops(&mut packet, &mut ops)?,
            252 => ops.push(format!("unknown252={}", packet.g2()?)),
            253 => ops.push(format!("unknown253={}", packet.g1()?)),
            opcode => bail!("unknown npc opcode {opcode} in {id}"),
        }
    }
}

pub fn parse_obj(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("obj {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!("model={}", packet.gsmart2or4null()?)),
            2 => ops.push(format!("name={}", packet.gjstr()?)),
            3 => ops.push(format!("desc={}", packet.gjstr()?)),
            4 => ops.push(format!("2dzoom={}", packet.g2()?)),
            5 => ops.push(format!("2dxan={}", packet.g2()?)),
            6 => ops.push(format!("2dyan={}", packet.g2()?)),
            7 => ops.push(format!("2dxof={}", packet.g2s()?)),
            8 => ops.push(format!("2dyof={}", packet.g2s()?)),
            9 => ops.push(format!("unknown9={}", packet.gjstr()?)),
            10 => ops.push(format!("anim={}", packet.g2()?)),
            11 => ops.push(String::from("stackable=yes")),
            12 => ops.push(format!("cost={}", packet.g4s()?)),
            13 => ops.push(format!("wearpos={}", packet.g1()?)),
            14 => ops.push(format!("wearpos2={}", packet.g1()?)),
            15 => ops.push(String::from("tradeable=no")),
            16 => ops.push(String::from("members=yes")),
            23 => ops.push(format!("manwear={}", packet.gsmart2or4null()?)),
            24 => ops.push(format!("manwear2={}", packet.gsmart2or4null()?)),
            25 => ops.push(format!("womanwear={}", packet.gsmart2or4null()?)),
            26 => ops.push(format!("womanwear2={}", packet.gsmart2or4null()?)),
            27 => ops.push(format!("wearpos3={}", packet.g1()?)),
            30 => ops.push(format!("op1={}", packet.gjstr()?)),
            31 => ops.push(format!("op2={}", packet.gjstr()?)),
            32 => ops.push(format!("op3={}", packet.gjstr()?)),
            33 => ops.push(format!("op4={}", packet.gjstr()?)),
            34 => ops.push(format!("op5={}", packet.gjstr()?)),
            35 => ops.push(format!("iop1={}", packet.gjstr()?)),
            36 => ops.push(format!("iop2={}", packet.gjstr()?)),
            37 => ops.push(format!("iop3={}", packet.gjstr()?)),
            38 => ops.push(format!("iop4={}", packet.gjstr()?)),
            39 => ops.push(format!("iop5={}", packet.gjstr()?)),
            40 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("recol{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("recol{}d={}", i + 1, packet.g2()?));
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("retex{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("retex{}d={}", i + 1, packet.g2()?));
                }
            }
            42 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("unknown42={}", packet.g1s()?));
                }
            }
            43 => ops.push(format!("minimenucolour={}", packet.g4s()?)),
            44 => ops.push(format!("recolindices={}", packet.g2()?)),
            45 => ops.push(format!("retexindices={}", packet.g2()?)),
            65 => ops.push(String::from("stockmarket=yes")),
            69 => ops.push(format!("stockmarketlimit={}", packet.g4s()?)),
            78 => ops.push(format!("manwear3={}", packet.gsmart2or4null()?)),
            79 => ops.push(format!("womanwear3={}", packet.gsmart2or4null()?)),
            90 => ops.push(format!("manhead={}", packet.gsmart2or4null()?)),
            91 => ops.push(format!("womanhead={}", packet.gsmart2or4null()?)),
            92 => ops.push(format!("manhead2={}", packet.gsmart2or4null()?)),
            93 => ops.push(format!("womanhead2={}", packet.gsmart2or4null()?)),
            94 => ops.push(format!("category={}", packet.g2()?)),
            95 => ops.push(format!("2dzan={}", packet.g2()?)),
            96 => ops.push(format!("dummyitem={}", packet.g1()?)),
            97 => ops.push(format!("certlink={}", packet.g2()?)),
            98 => ops.push(format!("certtemplate={}", packet.g2()?)),
            100 => ops.push(format!("count1={},{}", packet.g2()?, packet.g2()?)),
            101 => ops.push(format!("count2={},{}", packet.g2()?, packet.g2()?)),
            102 => ops.push(format!("count3={},{}", packet.g2()?, packet.g2()?)),
            103 => ops.push(format!("count4={},{}", packet.g2()?, packet.g2()?)),
            104 => ops.push(format!("count5={},{}", packet.g2()?, packet.g2()?)),
            105 => ops.push(format!("count6={},{}", packet.g2()?, packet.g2()?)),
            106 => ops.push(format!("count7={},{}", packet.g2()?, packet.g2()?)),
            107 => ops.push(format!("count8={},{}", packet.g2()?, packet.g2()?)),
            108 => ops.push(format!("count9={},{}", packet.g2()?, packet.g2()?)),
            109 => ops.push(format!("count10={},{}", packet.g2()?, packet.g2()?)),
            110 => ops.push(format!("resizex={}", packet.g2()?)),
            111 => ops.push(format!("resizey={}", packet.g2()?)),
            112 => ops.push(format!("resizez={}", packet.g2()?)),
            113 => ops.push(format!("ambient={}", packet.g1s()?)),
            114 => ops.push(format!("contrast={}", packet.g1s()?)),
            115 => ops.push(format!("team={}", packet.g1()?)),
            121 => ops.push(format!("lentlink={}", packet.g2()?)),
            122 => ops.push(format!("lenttemplate={}", packet.g2()?)),
            125 => ops.push(format!(
                "manwearoff={},{},{}",
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?
            )),
            126 => ops.push(format!(
                "womanwearoff={},{},{}",
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?
            )),
            127 => ops.push(format!("cursor1={},{}", packet.g1()?, packet.g2()?)),
            128 => ops.push(format!("cursor2={},{}", packet.g1()?, packet.g2()?)),
            129 => ops.push(format!("icursor1={},{}", packet.g1()?, packet.g2()?)),
            130 => ops.push(format!("icursor2={},{}", packet.g1()?, packet.g2()?)),
            131 => ops.push(format!("unknown131={}", packet.gjstr()?)),
            132 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("quest={}", packet.g2()?));
                }
            }
            134 => ops.push(format!("picksizeshift={}", packet.g1()?)),
            139 => ops.push(format!("boughtlink={}", packet.g2()?)),
            140 => ops.push(format!("boughttemplate={}", packet.g2()?)),
            142 => ops.push(format!("cursor1={}", packet.g2()?)),
            143 => ops.push(format!("cursor2={}", packet.g2()?)),
            144 => ops.push(format!("cursor3={}", packet.g2()?)),
            145 => ops.push(format!("cursor4={}", packet.g2()?)),
            146 => ops.push(format!("cursor5={}", packet.g2()?)),
            148 => ops.push(format!("placeholderlink={}", packet.g2()?)),
            149 => ops.push(format!("placeholdertemplate={}", packet.g2()?)),
            150 => ops.push(format!("icursor1={}", packet.g2()?)),
            151 => ops.push(format!("icursor2={}", packet.g2()?)),
            152 => ops.push(format!("icursor3={}", packet.g2()?)),
            153 => ops.push(format!("icursor4={}", packet.g2()?)),
            154 => ops.push(format!("icursor5={}", packet.g2()?)),
            156 => ops.push(String::from("shadow=no")),
            157 => ops.push(String::from("unknown157=yes")),
            161 => ops.push(format!("shardlink={}", packet.g2()?)),
            162 => ops.push(format!("shardtemplate={}", packet.g2()?)),
            163 => ops.push(format!("shardcount={}", packet.g2()?)),
            164 => ops.push(format!("shardname={}", packet.gjstr()?)),
            165 => ops.push(String::from("stackable=never")),
            167 => ops.push(String::from("unknown167=yes")),
            168 => ops.push(String::from("placeholder=no")),
            178 => ops.push(String::from("stackable=sometimes")),
            181 => ops.push(format!("cost={}", packet.g8s()?)),
            249 => parse_param_ops(&mut packet, &mut ops)?,
            opcode => bail!("unknown obj opcode {opcode} in {id}"),
        }
    }
}

fn parse_loc_multi(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    with_default: bool,
) -> Result<()> {
    let multivarbit = packet.g2null()?;
    if multivarbit != -1 {
        ops.push(format!("multivar=varbit:{multivarbit}"));
    }
    let multivarp = packet.g2null()?;
    if multivarp != -1 {
        ops.push(format!("multivar=varp:{multivarp}"));
    }
    if with_default {
        let default_id = packet.gsmart2or4null()?;
        if default_id != -1 {
            ops.push(format!("multiloc=default,{default_id}"));
        }
    }
    let count = usize::from(packet.gsmart1or2()?);
    for i in 0..=count {
        let multi = packet.gsmart2or4null()?;
        if multi != -1 {
            ops.push(format!("multiloc={i},{multi}"));
        }
    }
    Ok(())
}

fn parse_npc_multi(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    with_default: bool,
) -> Result<()> {
    let multivarbit = packet.g2null()?;
    if multivarbit != -1 {
        ops.push(format!("multivar=varbit:{multivarbit}"));
    }
    let multivarp = packet.g2null()?;
    if multivarp != -1 {
        ops.push(format!("multivar=varp:{multivarp}"));
    }
    if with_default {
        let default_id = packet.g2null()?;
        if default_id != -1 {
            ops.push(format!("multinpc=default,{default_id}"));
        }
    }
    let count = usize::from(packet.gsmart1or2()?);
    for i in 0..=count {
        let multi = packet.g2null()?;
        if multi != -1 {
            ops.push(format!("multinpc={i},{multi}"));
        }
    }
    Ok(())
}

fn parse_multi_variants_block(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    let _unused = packet.g2()?;
    let varbit = packet.g2null()?;
    let varplayer = packet.g2null()?;

    if varbit != -1 {
        ops.push(format!("multivar=varbit:{varbit}"));
    }
    if varplayer != -1 {
        ops.push(format!("multivar=varp:{varplayer}"));
    }

    let flags = packet.g1()?;

    if (flags & 1) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                let mut line = format!(
                    "multimodel={value},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    gsmart2or4s(packet)?
                );
                let n = packet.g1()?;
                if n >= 1 {
                    let _ = write!(line, ",{}", packet.g1()?);
                }
                if n >= 2 {
                    let _ = write!(line, ",{}", packet.g1()?);
                }
                if n >= 3 {
                    let _ = write!(line, ",{}", packet.g1()?);
                }
                ops.push(line);
            }
        }
    }

    if (flags & 2) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                ops.push(format!(
                    "multiheadmodel={value},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    gsmart2or4s(packet)?
                ));
            }
        }
    }

    if (flags & 4) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                ops.push(format!(
                    "multiretex={value},{},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?
                ));
            }
        }
    }

    if (flags & 8) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                ops.push(format!(
                    "multirecol={value},{},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?
                ));
            }
        }
    }

    if (flags & 16) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            ops.push(format!(
                "multitint={value},{},{},{},{},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g1()?,
                packet.g1()?,
                packet.g1()?,
                packet.g1()?
            ));
        }
    }

    ops.push(format!("multidefault={}", packet.g2()?));
    Ok(())
}

// Material format uses short names for texture/animation IDs.
#[allow(clippy::similar_names)]
fn parse_material_v0(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    ops.push(format!("unknown1={}", packet.g1()?));
    ops.push(format!("size={}", packet.g1()?));
    let flags_a = packet.g4s()?;

    let flaga0 = (flags_a & 1) != 0;
    let flaga1 = (flags_a & 2) != 0;
    let flaga2 = (flags_a & 4) != 0;
    let flaga3 = (flags_a & 8) != 0;
    let flaga4 = (flags_a & 16) != 0;

    if flaga0 {
        ops.push(String::from("flaga0=yes"));
    }
    if flaga1 {
        ops.push(String::from("flaga1=yes"));
    }
    if flaga2 {
        ops.push(String::from("flaga2=yes"));
    }
    if flaga3 {
        ops.push(String::from("flaga3=yes"));
    }
    if flaga4 {
        ops.push(String::from("flaga4=yes"));
    }

    if flaga0 || flaga4 {
        ops.push(format!("texture={}", packet.g4s()?));
    }
    if flaga3 || flaga1 {
        ops.push(format!("bloomtexture={}", packet.g4s()?));
    }

    let repeat = packet.g1()?;
    ops.push(format!("repeat={},{}", repeat & 7, (repeat >> 3) & 7));

    let flags_b = packet.g4s()?;
    let flagb4 = (flags_b & 0x10) != 0;
    let flagb5 = (flags_b & 0x20) != 0;
    let flagb6 = (flags_b & 0x40) != 0;
    let flagb11 = (flags_b & 0x800) != 0;
    // Build 948+: new flag bit 23 gates one BE float read after the flags_c
    // speed block (byte-mapped against material 3224 in 947.1 vs 948.1).
    let flagb23 = (flags_b & 0x0080_0000) != 0;
    let flagb18 = (flags_b & 0x40000) != 0;
    let flagb19 = (flags_b & 0x80000) != 0;
    let flagb20 = (flags_b & 0x0010_0000) != 0;
    let flagb21 = (flags_b & 0x0020_0000) != 0;

    ops.push(format!("flagb0={}", yes_no((flags_b & 1) != 0)));
    ops.push(format!("flagb1={}", yes_no((flags_b & 2) != 0)));
    ops.push(format!("flagb2={}", yes_no((flags_b & 4) != 0)));
    ops.push(format!("flagb4={}", yes_no(flagb4)));
    ops.push(format!("flagb21={}", yes_no(flagb21)));
    ops.push(format!("flagb20={}", yes_no(flagb20)));

    if flagb5 {
        ops.push(format!("unknown19={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb6 {
        ops.push(format!("unknown20={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb18 {
        ops.push(format!("unknown4={}", packet.g4s()?));
    }
    if flagb19 {
        ops.push(format!(
            "unknown5={},{},{},{},{}",
            packet.g4s()?,
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            packet.g4s()?,
            packet.g4s()?
        ));
    }
    if flagb4 {
        ops.push(format!(
            "unknown6={},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?
        ));
    }
    if flaga1 {
        ops.push(format!("unknown7={}", gfloat_be(packet)?));
    }

    ops.push(format!("bloom={}", yes_no(packet.g1()? == 1)));
    ops.push(format!("facetmode={}", packet.g1()?));

    match packet.g1()? {
        0 => ops.push(String::from("alphamode=none")),
        1 => ops.push(format!("alphamode=test,{}", packet.g1()?)),
        2 => ops.push(String::from("alphamode=multiply")),
        value => bail!("unknown material alphamode {value}"),
    }

    if flagb11 {
        ops.push(format!(
            "unknown9={},{},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            gfloat_be(packet)?
        ));
    }

    let flags_c = packet.g1()?;
    if (flags_c & 1) != 0 {
        ops.push(format!("speedu={}", packet.g2s()?));
    }
    if (flags_c & 2) != 0 {
        ops.push(format!("speedv={}", packet.g2s()?));
    }

    if flagb23 {
        ops.push(format!("unknown27={}", gfloat_be(packet)?));
    }

    if packet.g1()? == 1 {
        ops.push(format!("effect={}", packet.g1()?));
        ops.push(format!("effectarg1={}", packet.g1()?));
        ops.push(format!("effectarg2={}", packet.g4s()?));
        ops.push(format!("effectcombiner={}", packet.g1()?));
        ops.push(format!("unknown15={}", yes_no(packet.g1()? == 1)));
        ops.push(format!("mipmapping={}", packet.g1()?));
        ops.push(format!("lowdetail={}", yes_no(packet.g1()? == 1)));
        ops.push(format!("highdetail={}", yes_no(packet.g1()? == 1)));
        ops.push(format!("lightness={}", packet.g1()?));
        ops.push(format!("saturation={}", packet.g1()?));
        ops.push(format!("averagecolour={}", packet.g2()?));
    }
    Ok(())
}

// Same pattern as v0; texture/animation variable naming follows game conventions.
#[allow(clippy::similar_names)]
fn parse_material_v1(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    let flags_b = packet.g4s()?;
    if (flags_b >> 22) != 0 {
        bail!("invalid material flags {flags_b}");
    }

    let flagb5 = (flags_b & 0x20) != 0;
    let flagb6 = (flags_b & 0x40) != 0;
    let flagb7 = (flags_b & 0x80) != 0;
    let flagb8 = (flags_b & 0x100) != 0;
    let flagb9 = (flags_b & 0x200) != 0;
    let flagb11 = (flags_b & 0x800) != 0;
    let flagb12 = (flags_b & 0x1000) != 0;
    let flagb13 = (flags_b & 0x2000) != 0;
    let flagb14 = (flags_b & 0x4000) != 0;
    let flagb15 = (flags_b & 0x8000) != 0;
    let flagb16 = (flags_b & 0x10000) != 0;
    let flagb17 = (flags_b & 0x20000) != 0;
    let flagb18 = (flags_b & 0x40000) != 0;
    let flagb19 = (flags_b & 0x80000) != 0;
    let flagb20 = (flags_b & 0x0010_0000) != 0;
    let flagb21 = (flags_b & 0x0020_0000) != 0;

    for (label, bit) in [
        ("flagsb0", 1),
        ("flagsb1", 2),
        ("flagsb2", 4),
        ("flagsb3", 8),
        ("flagsb4", 0x10),
        ("flagsb10", 0x400),
    ] {
        if (flags_b & bit) != 0 {
            ops.push(format!("{label}=yes"));
        }
    }
    if flagb21 {
        ops.push(String::from("flagsb21=yes"));
    }
    if flagb20 {
        ops.push(String::from("flagsb20=yes"));
    }

    if flagb5 {
        ops.push(format!("unknown19={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb6 {
        ops.push(format!("unknown20={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb7 {
        ops.push(format!("unknown21={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb18 {
        ops.push(format!("unknown4={}", packet.g4s()?));
    }
    if flagb19 {
        ops.push(format!(
            "unknown5={},{},{},{},{}",
            packet.g4s()?,
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            packet.g4s()?,
            packet.g4s()?
        ));
    }
    if flagb12 {
        ops.push(format!("unknown6={}", gfloat_be(packet)?));
    }
    if flagb13 {
        ops.push(format!("unknown7={}", packet.g4s()?));
    }
    if flagb14 {
        ops.push(format!("unknown22={}", gfloat_be(packet)?));
    }
    if flagb15 {
        ops.push(format!("unknown23={}", packet.g4s()?));
    }
    if flagb6 {
        ops.push(format!("unknown24={}", gfloat_be(packet)?));
    }
    if flagb11 {
        ops.push(format!(
            "unknown9={},{},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            gfloat_be(packet)?
        ));
    }
    if flagb16 {
        ops.push(format!("unknown25={}", gfloat_be(packet)?));
    }
    if flagb17 {
        ops.push(format!("unknown26={}", gfloat_be(packet)?));
    }
    if flagb8 {
        ops.push(format!("speedu={}", packet.g2s()?));
    }
    if flagb9 {
        ops.push(format!("speedv={}", packet.g2s()?));
    }

    let repeat = packet.g1()?;
    ops.push(format!("repeat={},{}", repeat & 7, (repeat >> 3) & 7));
    ops.push(format!("facetmode={}", packet.g1()?));
    ops.push(format!("qualitymode={}", packet.g1()?));
    match packet.g1()? {
        0 => ops.push(String::from("alphamode=none")),
        1 => ops.push(format!("alphamode=test,{}", packet.g1()?)),
        2 => ops.push(String::from("alphamode=multiply")),
        value => bail!("unknown material alphamode {value}"),
    }
    ops.push(format!("averagecolour={}", packet.g2()?));
    ops.push(format!("size={}", packet.g1()?));
    Ok(())
}

fn decode_normal_map_params(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    opcode: u8,
) -> Result<()> {
    let offset = usize::from(opcode - 33);
    let target = (offset / 8) + 1;
    match offset % 8 {
        0 => ops.push(format!(
            "normal_map_params{target}_unknown33={}",
            yes_no(packet.g1()? == 1)
        )),
        1 => ops.push(format!(
            "normal_map_params{target}_unknown34={}",
            gfloat_be(packet)?
        )),
        2 => ops.push(format!(
            "normal_map_params{target}_unknown35={}",
            gfloat_be(packet)?
        )),
        3 => ops.push(format!(
            "normal_map_params{target}_unknown36={}",
            gfloat_be(packet)?
        )),
        4 => ops.push(format!(
            "normal_map_params{target}_unknown37={},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?
        )),
        5 => ops.push(format!(
            "normal_map_params{target}_unknown38={},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?
        )),
        6 => ops.push(format!(
            "normal_map_params{target}_unknown39={}",
            gfloat_be(packet)?
        )),
        7 => ops.push(format!(
            "normal_map_params{target}_unknown40={}",
            gfloat_be(packet)?
        )),
        _ => bail!("invalid normal map opcode {opcode}"),
    }
    Ok(())
}

fn parse_achievement_stat_var_list(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    key: &str,
    uses_g1_second: bool,
) -> Result<()> {
    let count = usize::from(packet.gsmart1or2()?);
    for _ in 0..count {
        let a = packet.g1()?;
        let b = if uses_g1_second {
            i32::from(packet.g1()?)
        } else {
            packet.gsmart2or4null()?
        };
        let c = packet.gjstr2()?;
        let count2 = usize::from(packet.gsmart1or2()?);
        let mut line = format!("{key}={a},{b},{c}");
        for _ in 0..count2 {
            let _ = write!(line, ",{}", packet.g2()?);
        }
        ops.push(line);
    }
    Ok(())
}

fn parse_achievement_var_list(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    key: &str,
) -> Result<()> {
    let count = usize::from(packet.gsmart1or2()?);
    for _ in 0..count {
        let mut line = format!("{key}={}", packet.g1()?);
        let _ = write!(line, ",{}", gsmart2or4s(packet)?);
        let string = packet.gjstr2()?;
        line.push(',');
        line.push_str(&string);
        let count2 = usize::from(packet.gsmart1or2()?);
        for _ in 0..count2 {
            let _ = write!(line, ",{}", packet.g2()?);
        }
        ops.push(line);
    }
    Ok(())
}

fn parse_achievement_ref_list(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    key: &str,
) -> Result<()> {
    let count = usize::from(packet.gsmart1or2()?);
    for _ in 0..count {
        ops.push(format!("{key}={},{}", packet.g1()?, packet.g2()?));
    }
    Ok(())
}

fn parse_achievement_testbit_list(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    key: &str,
) -> Result<()> {
    let count = usize::from(packet.gsmart1or2()?);
    for _ in 0..count {
        ops.push(format!(
            "{key}={},{},{},{},{}",
            packet.g1()?,
            packet.g2()?,
            packet.g1()?,
            packet.gjstr2()?,
            packet.g1()?
        ));
    }
    Ok(())
}

fn parse_multimel(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    with_default: bool,
) -> Result<()> {
    let varbit = packet.g2null()?;
    if varbit != -1 {
        ops.push(format!("multivar=varbit:{varbit}"));
    }
    let varp = packet.g2null()?;
    if varp != -1 {
        ops.push(format!("multivar=varp:{varp}"));
    }
    if with_default {
        let default_id = packet.g2null()?;
        if default_id != -1 {
            ops.push(format!("multimel=default,{default_id}"));
        }
    }
    let count = usize::from(packet.g1()?);
    for i in 0..=count {
        let multi = packet.g2null()?;
        if multi != -1 {
            ops.push(format!("multimel={i},{multi}"));
        }
    }
    Ok(())
}

fn parse_param_ops(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    let count = usize::from(packet.g1()?);
    for _ in 0..count {
        let is_string = packet.g1()? == 1;
        let param = packet.g3()?;
        if is_string {
            ops.push(format!("param={param},{}", packet.gjstr()?));
        } else {
            ops.push(format!("param={param},{}", packet.g4s()?));
        }
    }
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn parse_empty_config(kind: &str, id: u32, data: &[u8]) -> Result<()> {
    let mut packet = Packet::new(data);
    let opcode = packet.g1()?;
    if opcode != 0 {
        bail!("unknown {kind} opcode {opcode} in {id}");
    }
    if !packet.is_done() {
        bail!("{kind} {id} did not consume full payload");
    }
    Ok(())
}

fn format_template_zone(value: i32) -> Result<String> {
    let value_u = u32::try_from(value).context("negative worldarea template value")?;
    if (value_u >> 26) != 0 {
        bail!("invalid template zone {value_u}");
    }
    let level = (value_u >> 24) & 0x3;
    let x = ((value_u >> 14) & 0x3ff) * 8;
    let z = ((value_u >> 3) & 0x7ff) * 8;
    let angle = (value_u >> 1) & 0x3;
    let unknown = value_u & 1;
    Ok(format!(
        "{}_{}_{}_{}_{},{},{}",
        level,
        x / 64,
        z / 64,
        x % 64,
        z % 64,
        angle,
        unknown
    ))
}

fn gfloat_be(packet: &mut Packet<'_>) -> Result<f32> {
    Ok(f32::from_bits(packet.g4s()? as u32))
}

fn gsmart1or2s(packet: &mut Packet<'_>) -> Result<i32> {
    let first = packet
        .slice(packet.pos(), packet.pos() + 1)
        .context("gsmart1or2s out of bounds")?[0];
    if first < 128 {
        Ok(i32::from(packet.g1()?) - 64)
    } else {
        Ok(i32::from(packet.g2()?) - 49_152)
    }
}

fn gsmart1or2null(packet: &mut Packet<'_>) -> Result<i32> {
    let first = packet
        .slice(packet.pos(), packet.pos() + 1)
        .context("gsmart1or2null out of bounds")?[0];
    if first < 128 {
        Ok(i32::from(packet.g1()?) - 1)
    } else {
        Ok(i32::from(packet.g2()?) - 32_769)
    }
}

fn gsmart2or4s(packet: &mut Packet<'_>) -> Result<i32> {
    let first = packet
        .slice(packet.pos(), packet.pos() + 1)
        .context("gsmart2or4s out of bounds")?[0];
    if (first as i8) < 0 {
        Ok(packet.g4s()? & i32::MAX)
    } else {
        Ok(i32::from(packet.g2()?))
    }
}

fn read_u16_list_g1_count(packet: &mut Packet<'_>) -> Result<Vec<u16>> {
    let count = usize::from(packet.g1()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(packet.g2()?);
    }
    Ok(values)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_param_payload() {
        let bytes = [1, b'i', 2, 0, 0, 0, 123, 4, 0];
        let parsed = parse_param(10, &bytes).expect("parse param");
        assert_eq!(10, parsed.id);
        assert_eq!(Some(b'i'), parsed.type_char);
        assert!(!parsed.autodisable);
        assert!(matches!(parsed.default, Some(ScalarValue::Int(123))));
    }

    #[test]
    fn parses_enum_sparse_string_values() {
        let bytes = [1, b'i', 2, b's', 5, 0, 1, 0, 0, 0, 1, b'x', 0, 0];
        let parsed = parse_enum(20, &bytes).expect("parse enum");
        assert_eq!(Some(b'i'), parsed.input_type_char);
        assert_eq!(Some(b's'), parsed.output_type_char);
        assert_eq!(1, parsed.values.len());
        assert_eq!(1, parsed.values[0].key);
        assert!(matches!(parsed.values[0].value, ScalarValue::Str(ref s) if s == "x"));
    }

    #[test]
    fn parses_dbtable_column_schema() {
        let bytes = [1, 1, 0, 1, 0, 255, 0];
        let parsed = parse_dbtable(40, &bytes).expect("parse dbtable");
        assert_eq!(40, parsed.id);
        assert_eq!(1, parsed.columns.len());
        assert_eq!(0, parsed.columns[0].column);
        assert_eq!(vec![0], parsed.columns[0].tuple_types);
    }

    #[test]
    fn parses_dbrow_table_and_column_values() {
        let bytes = [3, 0, 0, 1, 0, 1, 0, 0, 0, 5, 255, 4, 42, 0];
        let parsed = parse_dbrow(41, &bytes).expect("parse dbrow");
        assert_eq!(41, parsed.id);
        assert_eq!(Some(42), parsed.table);
        assert_eq!(1, parsed.columns.len());
        assert_eq!(0, parsed.columns[0].column);
        assert_eq!(1, parsed.columns[0].rows.len());
        assert!(matches!(parsed.columns[0].rows[0][0], ScalarValue::Int(5)));
    }

    #[test]
    fn parses_struct_params() {
        let bytes = [249, 2, 1, 0, 0, 5, b'a', 0, 0, 0, 0, 7, 0, 0, 0, 123, 0];
        let parsed = parse_struct(55, &bytes).expect("parse struct");
        assert_eq!(55, parsed.id);
        assert_eq!(2, parsed.params.len());
        assert_eq!(5, parsed.params[0].param_id);
        assert!(matches!(
            parsed.params[0].value,
            ScalarValue::Str(ref value) if value == "a"
        ));
        assert_eq!(7, parsed.params[1].param_id);
        assert!(matches!(parsed.params[1].value, ScalarValue::Int(123)));
    }

    #[test]
    fn parses_inv_size_and_stock() {
        let bytes = [2, 0, 28, 4, 1, 0, 10, 0, 99, 0];
        let parsed = parse_inv(9, &bytes).expect("parse inv");
        assert_eq!(9, parsed.id);
        assert_eq!(Some(28), parsed.size);
        assert_eq!(1, parsed.stocks.len());
        assert_eq!(10, parsed.stocks[0].obj_id);
        assert_eq!(99, parsed.stocks[0].count);
    }

    #[test]
    fn parses_cursor_graphic_and_hotspot() {
        let bytes = [1, 0, 42, 2, 3, 4, 0];
        let parsed = parse_cursor(12, &bytes).expect("parse cursor");
        assert_eq!(12, parsed.id);
        assert_eq!(Some(42), parsed.graphic);
        let hotspot = parsed.hotspot.expect("missing hotspot");
        assert_eq!(3, hotspot.x);
        assert_eq!(4, hotspot.y);
    }

    #[test]
    fn parses_seqgroup_sections() {
        let bytes = [2, 2, 1, 2, 3, 9, 1, 5, 7, 4, 8, 1, 6, 10, 0];
        let parsed = parse_seqgroup(77, &bytes).expect("parse seqgroup");
        assert_eq!(77, parsed.id);
        assert_eq!(vec![1, 2], parsed.walkmerge);
        assert_eq!(Some(9), parsed.unknown3_default);
        assert_eq!(1, parsed.unknown3.len());
        assert_eq!(5, parsed.unknown3[0].label);
        assert_eq!(7, parsed.unknown3[0].value);
        assert_eq!(Some(8), parsed.unknown4_default);
        assert_eq!(1, parsed.unknown4.len());
        assert_eq!(6, parsed.unknown4[0].label);
        assert_eq!(10, parsed.unknown4[0].value);
    }

    #[test]
    fn parses_empty_controller_and_category() {
        let controller = parse_controller(1, &[0]).expect("parse controller");
        assert_eq!(1, controller.id);
        let category = parse_category(2, &[0]).expect("parse category");
        assert_eq!(2, category.id);
    }

    #[test]
    fn parses_empty_family_entries() {
        assert_eq!(3, parse_area(3, &[0]).expect("parse area").id);
        assert_eq!(4, parse_hunt(4, &[0]).expect("parse hunt").id);
        assert_eq!(5, parse_mesanim(5, &[0]).expect("parse mesanim").id);
        assert_eq!(6, parse_itemcode(6, &[0]).expect("parse itemcode").id);
        assert_eq!(
            7,
            parse_gamelogevent(7, &[0]).expect("parse gamelogevent").id
        );
        assert_eq!(8, parse_bugtemplate(8, &[0]).expect("parse bugtemplate").id);
        assert_eq!(9, parse_var_npc_bit(9, &[0]).expect("parse varnbit").id);
        assert_eq!(10, parse_var_shared(10, &[0]).expect("parse vars").id);
        assert_eq!(
            11,
            parse_var_shared_string(11, &[0]).expect("parse varsstr").id
        );
    }

    #[test]
    fn parses_var_client_string_scope() {
        let scoped = parse_var_client_string(12, &[2, 0]).expect("parse varcstr");
        assert_eq!(12, scoped.id);
        assert!(scoped.scope_perm);
        let unscoped = parse_var_client_string(13, &[0]).expect("parse varcstr");
        assert!(!unscoped.scope_perm);
    }

    #[test]
    fn parses_bas_opcode52_pre916_shape() {
        let bytes = [52, 1, 0, 5, 7, 0];
        let parsed = parse_bas(10, &bytes, 910).expect("parse bas pre916");
        assert_eq!(10, parsed.id);
        assert_eq!(1, parsed.ops.len());
        assert_eq!("randomreadyanim=5,7", parsed.ops[0]);
    }

    #[test]
    fn parses_bas_opcode52_post916_shape() {
        let bytes = [52, 1, 0, 5, 7, 0, 0];
        let parsed = parse_bas(11, &bytes, 947).expect("parse bas post916");
        assert_eq!(11, parsed.id);
        assert_eq!(1, parsed.ops.len());
        assert_eq!("randomreadyanim=5,7", parsed.ops[0]);
    }

    #[test]
    fn parses_underlay_fields() {
        let bytes = [1, 1, 2, 3, 2, 0, 5, 3, 0, 44, 4, 5, 0];
        let parsed = parse_underlay(1, &bytes).expect("parse underlay");
        assert_eq!(1, parsed.id);
        assert_eq!(Some(0x0001_0203), parsed.colour);
        assert_eq!(Some(5), parsed.material);
        assert_eq!(Some(44), parsed.texture_scale);
        assert!(!parsed.hardshadow);
        assert!(!parsed.occlude);
    }

    #[test]
    fn parses_overlay_fields() {
        let bytes = [
            1, 0xAA, 0xBB, 0xCC, 3, 0xFF, 0xFF, 5, 6, b'n', 0, 7, 1, 2, 3, 8, 9, 0, 10, 10, 11, 3,
            12, 13, 0x10, 0x20, 0x30, 14, 7, 15, 0, 99, 16, 8, 20, 0, 5, 21, 9, 22, 0, 6, 0,
        ];
        let parsed = parse_overlay(2, &bytes).expect("parse overlay");
        assert_eq!(2, parsed.id);
        assert_eq!(Some(0x00AA_BBCC), parsed.colour);
        assert_eq!(Some(-1), parsed.material);
        assert!(!parsed.occlude);
        assert_eq!(Some("n".to_string()), parsed.debugname);
        assert_eq!(Some(0x0001_0203), parsed.mapcolour);
        assert!(parsed.toggles.unknown8);
        assert_eq!(Some(10), parsed.texture_scale);
        assert!(!parsed.hardshadow);
        assert_eq!(Some(3), parsed.priority);
        assert!(parsed.toggles.smoothedges);
        assert_eq!(Some(0x0010_2030), parsed.waterfog_colour);
        assert_eq!(Some(7), parsed.waterfog_scale);
        assert_eq!(Some(99), parsed.unknown15);
        assert_eq!(Some(8), parsed.waterfog_offset);
        assert_eq!(Some(5), parsed.waterfog_unknown_a);
        assert_eq!(Some(9), parsed.waterfog_unknown_b);
        assert_eq!(Some(6), parsed.waterfog_unknown_c);
    }

    #[test]
    fn parses_msi_fields() {
        let bytes = [1, 0, 3, 2, 0, 0, 1, 3, 4, 5, 0];
        let parsed = parse_msi(3, &bytes).expect("parse msi");
        assert_eq!(3, parsed.id);
        assert_eq!(Some(3), parsed.graphic);
        assert_eq!(Some(1), parsed.unknown2);
        assert!(parsed.unknown3);
        assert!(parsed.unknown4);
        assert!(parsed.unknown5);
    }

    #[test]
    fn parses_skybox_fields() {
        let bytes = [1, 0, 2, 2, 2, 0, 3, 0, 4, 3, 9, 4, 7, 5, 0, 11, 6, 0, 12, 0];
        let parsed = parse_skybox(4, &bytes).expect("parse skybox");
        assert_eq!(4, parsed.id);
        assert_eq!(Some(2), parsed.material);
        assert_eq!(vec![3, 4], parsed.unknown2);
        assert_eq!(Some(9), parsed.unknown3);
        assert_eq!(Some(7), parsed.fillmode);
        assert_eq!(Some(11), parsed.unknown5);
        assert_eq!(Some(12), parsed.unknown6);
    }

    #[test]
    fn parses_worldarea_fields() {
        let bytes = [
            2, 1, 2, 3, 3, 0, 0, 0, 1, 0, 0, 0, 2, 4, 0, 0, 0, 3, 0, 0, 0, 0, 0,
        ];
        let parsed = parse_worldarea(5, &bytes).expect("parse worldarea");
        assert_eq!(5, parsed.id);
        assert_eq!(Some(0x0001_0203), parsed.colour);
        assert_eq!(1, parsed.impostor_squares.len());
        assert_eq!(1, parsed.impostor_squares[0].start);
        assert_eq!(2, parsed.impostor_squares[0].end);
        assert_eq!(1, parsed.impostor_zones.len());
        assert_eq!(3, parsed.impostor_zones[0].anchor);
        assert_eq!("0_0_0_0_0,0,0", parsed.impostor_zones[0].template);
    }

    #[test]
    fn parses_quickchatcat_fields() {
        let bytes = [1, b'd', 0, 2, 1, 0, 5, 255, 3, 1, 0, 6, 1, 4, 0];
        let parsed = parse_quickchatcat(6, &bytes).expect("parse quickchatcat");
        assert_eq!(6, parsed.id);
        assert_eq!(Some("d".to_string()), parsed.desc);
        assert_eq!(1, parsed.subcats.len());
        assert_eq!(5, parsed.subcats[0].id);
        assert_eq!(-1, parsed.subcats[0].shortcut);
        assert_eq!(1, parsed.phrases.len());
        assert_eq!(6, parsed.phrases[0].id);
        assert_eq!(1, parsed.phrases[0].shortcut);
        assert!(parsed.unknown4);
    }

    #[test]
    fn parses_headbar_fields() {
        let bytes = [
            2, 5, 3, 6, 4, 5, 0, 7, 7, 0, 8, 8, 0, 9, 11, 0, 9, 16, 17, 4, 0,
        ];
        let parsed = parse_headbar(7, &bytes).expect("parse headbar");
        assert_eq!(7, parsed.id);
        assert_eq!(Some(5), parsed.showpriority);
        assert_eq!(Some(6), parsed.hidepriority);
        assert!(parsed.fadeout_disabled);
        assert_eq!(Some(7), parsed.sticktime);
        assert_eq!(Some(8), parsed.full);
        assert_eq!(Some(9), parsed.empty);
        assert_eq!(Some(9), parsed.fadeout);
        assert!(parsed.unknown16);
        assert_eq!(Some(4), parsed.unknown17);
    }

    #[test]
    fn parses_hitmark_fields() {
        let bytes = [
            1, 0, 10, 2, 1, 2, 3, 7, 0, 1, 8, 0, b'x', 0, 11, 14, 0, 7, 17, 0, 1, 0, 2, 1, 0, 5, 0,
            6, 19, 0, 30, 20, 0, 15, 0,
        ];
        let parsed = parse_hitmark(8, &bytes).expect("parse hitmark");
        assert_eq!(8, parsed.id);
        assert_eq!(Some(10), parsed.damagefont);
        assert_eq!(Some(0x0001_0203), parsed.damagecolour);
        assert_eq!(Some(1), parsed.scrolltooffsetx);
        assert_eq!(Some("x".to_string()), parsed.damageformat);
        assert!(parsed.fadeout_disabled);
        assert_eq!(Some(7), parsed.fadeout);
        assert_eq!(Some(1), parsed.multivarbit);
        assert_eq!(Some(2), parsed.multivarp);
        assert_eq!(2, parsed.multimarks.len());
        assert!(matches!(
            parsed.multimarks[0],
            HitmarkMulti::Indexed {
                index: 0,
                hitmark_id: 5
            }
        ));
        assert_eq!(Some(30), parsed.damagescaleto);
        assert_eq!(Some(15), parsed.damagescalefrom);
    }

    #[test]
    fn parses_light_fields() {
        let bytes = [
            1, 3, 2, 0, 4, 3, 0, 5, 4, 0, 6, 5, 0, 0, 0, 1, 8, 0x3F, 0x80, 0x00, 0x00, 9, 10, 0, 0,
            0, 2, 12, 0x40, 0x00, 0x00, 0x00, 0,
        ];
        let parsed = parse_light(9, &bytes).expect("parse light");
        assert_eq!(9, parsed.id);
        assert_eq!(Some(3), parsed.function);
        assert_eq!(Some(4), parsed.frequency);
        assert_eq!(Some(5), parsed.amplitude);
        assert_eq!(Some(6), parsed.offset);
        assert_eq!(Some(1), parsed.unknown5);
        assert_eq!(Some(1.0), parsed.swayamount);
        assert!(parsed.swayamountrandom);
        assert_eq!(Some(2), parsed.swayduration);
        assert_eq!(Some(2.0), parsed.swayeasing);
    }

    #[test]
    fn parses_quickchatphrase_fields() {
        let bytes = [1, b't', 0, 2, 1, 0, 9, 3, 2, 0, 4, 0, 7, 0, 15, 4, 0];
        let parsed = parse_quickchatphrase(10, &bytes).expect("parse quickchatphrase");
        assert_eq!(10, parsed.id);
        assert_eq!(Some("t".to_string()), parsed.template);
        assert_eq!(vec![9], parsed.autoresponses);
        assert_eq!(2, parsed.dynamic_commands.len());
        assert!(matches!(
            parsed.dynamic_commands[0],
            QuickChatDynamicCommand::StatBase { stat_id: 7 }
        ));
        assert!(matches!(
            parsed.dynamic_commands[1],
            QuickChatDynamicCommand::CombatLevel
        ));
        assert!(parsed.unknown4_no);
    }

    #[test]
    fn parses_billboard_fields() {
        let bytes = [1, 0, 5, 2, 0, 1, 0, 2, 3, 255, 4, 7, 5, 8, 6, 7, 0];
        let parsed = parse_billboard(11, &bytes).expect("parse billboard");
        assert_eq!(11, parsed.id);
        assert_eq!(Some(5), parsed.material);
        assert_eq!(Some((1, 2)), parsed.unknown2);
        assert_eq!(Some(-1), parsed.unknown3);
        assert_eq!(Some(7), parsed.unknown4);
        assert_eq!(Some(8), parsed.unknown5);
        assert!(parsed.unknown6);
        assert!(parsed.unknown7);
    }

    #[test]
    fn parses_particle_effector_fields() {
        let bytes = [
            1, 0, 9, 2, 3, 3, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 4, 5, 0, 0, 0, 4, 6, 7, 8, 9, 10,
            0,
        ];
        let parsed = parse_particle_effector(12, &bytes).expect("parse particleeffector");
        assert_eq!(12, parsed.id);
        assert_eq!(8, parsed.ops.len());
        assert!(matches!(parsed.ops[0], ParticleEffectorOp::Unknown1(9)));
        assert!(matches!(
            parsed.ops[2],
            ParticleEffectorOp::Unknown3 { x: 1, y: 2, z: 3 }
        ));
    }

    #[test]
    fn parses_particle_emitter_fields() {
        let bytes = [
            1, 0, 1, 0, 2, 0, 3, 0, 4, 9, 2, 0, 5, 0, 6, 29, 0, 0, 7, 35, 1, 0, 8, 0, 9, 10, 0,
        ];
        let parsed = parse_particle_emitter(13, &bytes).expect("parse particleemitter");
        assert_eq!(13, parsed.id);
        assert_eq!(4, parsed.ops.len());
        assert!(matches!(
            parsed.ops[0],
            ParticleEmitterOp::Unknown1 {
                a: 1,
                b: 2,
                c: 3,
                d: 4
            }
        ));
        assert!(matches!(
            parsed.ops[2],
            ParticleEmitterOp::AngularVelocity(7)
        ));
    }

    #[test]
    fn parses_texture_fields() {
        let bytes = [0, 5, 1, 1, 0, 9, 0, 0, 0, 1, 2, 3];
        let parsed = parse_texture(14, &bytes).expect("parse texture");
        assert_eq!(14, parsed.id);
        assert_eq!(5, parsed.averagecolour);
        assert!(parsed.opaque);
        assert_eq!(9, parsed.sprite);
        assert_eq!(1, parsed.unknown1);
        assert_eq!((2, 3), parsed.animation);
    }

    #[test]
    fn parses_stylesheet_fields() {
        let bytes = [0, 7, 0, 1, 2, 0, 0, 0, 3, 0, 0, 0, 4];
        let parsed = parse_stylesheet(15, &bytes).expect("parse stylesheet");
        assert_eq!(15, parsed.id);
        assert_eq!(7, parsed.parent);
        assert_eq!(1, parsed.entries.len());
        assert_eq!(2, parsed.entries[0].unknown);
        assert_eq!(3, parsed.entries[0].key_hash);
        assert_eq!(4, parsed.entries[0].value);
    }
}
