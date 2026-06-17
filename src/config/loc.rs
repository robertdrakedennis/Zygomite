use super::{OpListEntry, gfloat_be, parse_multi_variants_block, parse_param_ops};
use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use std::fmt::Write;

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
