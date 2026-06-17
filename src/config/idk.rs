use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IdkOp {
    BodyPart(u8),
    Model(i32),
    Disable,
    RecolPair { src: u16, dst: u16 },
    RetexPair { src: u16, dst: u16 },
    Recol3(u16),
    Recol4(u16),
    RecolIndices(u16),
    RetexIndices(u16),
    Recol5Src(u16),
    Recol6Src(u16),
    Recol7Src(u16),
    Recol8Src(u16),
    Recol9Src(u16),
    Recol10Src(u16),
    Recol1Dst(u16),
    Recol2Dst(u16),
    Recol3Dst(u16),
    Recol4Dst(u16),
    Recol5Dst(u16),
    Recol6Dst(u16),
    Recol7Dst(u16),
    Recol8Dst(u16),
    Recol9Dst(u16),
    Recol10Dst(u16),
    HeadModel { slot: u8, model: i32 },
}

#[derive(Clone, Debug, Serialize)]
pub struct IdkEntry {
    pub id: u32,
    pub ops: Vec<IdkOp>,
}

pub fn parse_idk(id: u32, data: &[u8]) -> Result<IdkEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("idk {id} did not consume full payload");
                }
                return Ok(IdkEntry { id, ops });
            }
            1 => ops.push(IdkOp::BodyPart(packet.g1()?)),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(IdkOp::Model(packet.gsmart2or4null()?));
                }
            }
            3 => ops.push(IdkOp::Disable),
            40 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(IdkOp::RecolPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(IdkOp::RetexPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            42 => ops.push(IdkOp::Recol3(packet.g2()?)),
            43 => ops.push(IdkOp::Recol4(packet.g2()?)),
            44 => ops.push(IdkOp::RecolIndices(packet.g2()?)),
            45 => ops.push(IdkOp::RetexIndices(packet.g2()?)),
            46 => ops.push(IdkOp::Recol7Src(packet.g2()?)),
            47 => ops.push(IdkOp::Recol8Src(packet.g2()?)),
            48 => ops.push(IdkOp::Recol9Src(packet.g2()?)),
            49 => ops.push(IdkOp::Recol10Src(packet.g2()?)),
            50 => ops.push(IdkOp::Recol1Dst(packet.g2()?)),
            51 => ops.push(IdkOp::Recol2Dst(packet.g2()?)),
            52 => ops.push(IdkOp::Recol3Dst(packet.g2()?)),
            53 => ops.push(IdkOp::Recol4Dst(packet.g2()?)),
            54 => ops.push(IdkOp::Recol5Dst(packet.g2()?)),
            55 => ops.push(IdkOp::Recol6Dst(packet.g2()?)),
            56 => ops.push(IdkOp::Recol7Dst(packet.g2()?)),
            57 => ops.push(IdkOp::Recol8Dst(packet.g2()?)),
            58 => ops.push(IdkOp::Recol9Dst(packet.g2()?)),
            59 => ops.push(IdkOp::Recol10Dst(packet.g2()?)),
            code @ 60..=69 => {
                let slot = code - 59;
                ops.push(IdkOp::HeadModel {
                    slot,
                    model: packet.gsmart2or4null()?,
                });
            }
            opcode => bail!("unknown idk opcode {opcode} in {id}"),
        }
    }
}
