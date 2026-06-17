use super::{OpListEntry, parse_multi_variants_block, parse_param_ops};
use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;

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
