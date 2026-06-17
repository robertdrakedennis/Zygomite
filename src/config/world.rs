use super::parse_empty_config;
use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;

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
