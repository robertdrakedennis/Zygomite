use super::{parse_param_ops, OpListEntry};
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;

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
