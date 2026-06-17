use super::OpListEntry;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use std::fmt::Write;

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
