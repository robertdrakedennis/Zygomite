use super::{OpListEntry, gsmart2or4s};
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use std::fmt::Write;

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
