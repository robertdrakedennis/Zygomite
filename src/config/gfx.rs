use super::gfloat_be;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

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
