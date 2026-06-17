use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

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
