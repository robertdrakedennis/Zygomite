use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;

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
