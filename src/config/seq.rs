use super::ScalarValue;
use super::StructParamEntry;
use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;

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
