use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SpotOp {
    Model(i32),
    Anim(i32),
    HasAlpha,
    ResizeH(u16),
    ResizeV(u16),
    Rotation(u16),
    Ambient(u8),
    Contrast(u8),
    AllowLoop,
    HillChangeRotate,
    HillChangeRotateG2(u16),
    HillChangeRotateG4(i32),
    RecolPair { src: u16, dst: u16 },
    RetexPair { src: u16, dst: u16 },
    Recol3(u16),
    Recol4(u16),
    RecolIndices(u16),
    RetexIndices(u16),
    Unknown46,
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
}

#[derive(Clone, Debug, Serialize)]
pub struct SpotEntry {
    pub id: u32,
    pub ops: Vec<SpotOp>,
}

pub fn parse_spot(id: u32, data: &[u8]) -> Result<SpotEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("spot {id} did not consume full payload");
                }
                return Ok(SpotEntry { id, ops });
            }
            1 => ops.push(SpotOp::Model(packet.gsmart2or4null()?)),
            2 => ops.push(SpotOp::Anim(packet.gsmart2or4null()?)),
            3 => ops.push(SpotOp::HasAlpha),
            4 => ops.push(SpotOp::ResizeH(packet.g2()?)),
            5 => ops.push(SpotOp::ResizeV(packet.g2()?)),
            6 => ops.push(SpotOp::Rotation(packet.g2()?)),
            7 => ops.push(SpotOp::Ambient(packet.g1()?)),
            8 => ops.push(SpotOp::Contrast(packet.g1()?)),
            10 => ops.push(SpotOp::AllowLoop),
            9 => ops.push(SpotOp::HillChangeRotate),
            15 => ops.push(SpotOp::HillChangeRotateG2(packet.g2()?)),
            16 => ops.push(SpotOp::HillChangeRotateG4(packet.g4s()?)),
            40 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(SpotOp::RecolPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(SpotOp::RetexPair {
                        src: packet.g2()?,
                        dst: packet.g2()?,
                    });
                }
            }
            42 => ops.push(SpotOp::Recol3(packet.g2()?)),
            43 => ops.push(SpotOp::Recol4(packet.g2()?)),
            44 => ops.push(SpotOp::RecolIndices(packet.g2()?)),
            45 => ops.push(SpotOp::RetexIndices(packet.g2()?)),
            46 => ops.push(SpotOp::Unknown46),
            47 => ops.push(SpotOp::Recol8Src(packet.g2()?)),
            48 => ops.push(SpotOp::Recol9Src(packet.g2()?)),
            49 => ops.push(SpotOp::Recol10Src(packet.g2()?)),
            50 => ops.push(SpotOp::Recol1Dst(packet.g2()?)),
            51 => ops.push(SpotOp::Recol2Dst(packet.g2()?)),
            52 => ops.push(SpotOp::Recol3Dst(packet.g2()?)),
            53 => ops.push(SpotOp::Recol4Dst(packet.g2()?)),
            54 => ops.push(SpotOp::Recol5Dst(packet.g2()?)),
            55 => ops.push(SpotOp::Recol6Dst(packet.g2()?)),
            56 => ops.push(SpotOp::Recol7Dst(packet.g2()?)),
            57 => ops.push(SpotOp::Recol8Dst(packet.g2()?)),
            58 => ops.push(SpotOp::Recol9Dst(packet.g2()?)),
            59 => ops.push(SpotOp::Recol10Dst(packet.g2()?)),
            opcode => bail!("unknown spot opcode {opcode} in {id}"),
        }
    }
}
