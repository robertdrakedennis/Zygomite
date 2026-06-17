use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParticleEffectorOp {
    Unknown1(u16),
    Unknown2(u8),
    Unknown3 { x: i32, y: i32, z: i32 },
    Unknown4 { a: u8, b: i32 },
    Unknown6(u8),
    Unknown8,
    Unknown9,
    Unknown10,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParticleEffectorEntry {
    pub id: u32,
    pub ops: Vec<ParticleEffectorOp>,
}

pub fn parse_particle_effector(id: u32, data: &[u8]) -> Result<ParticleEffectorEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("particleeffector {id} did not consume full payload");
                }
                return Ok(ParticleEffectorEntry { id, ops });
            }
            1 => ops.push(ParticleEffectorOp::Unknown1(packet.g2()?)),
            2 => ops.push(ParticleEffectorOp::Unknown2(packet.g1()?)),
            3 => ops.push(ParticleEffectorOp::Unknown3 {
                x: packet.g4s()?,
                y: packet.g4s()?,
                z: packet.g4s()?,
            }),
            4 => ops.push(ParticleEffectorOp::Unknown4 {
                a: packet.g1()?,
                b: packet.g4s()?,
            }),
            6 => ops.push(ParticleEffectorOp::Unknown6(packet.g1()?)),
            8 => ops.push(ParticleEffectorOp::Unknown8),
            9 => ops.push(ParticleEffectorOp::Unknown9),
            10 => ops.push(ParticleEffectorOp::Unknown10),
            opcode => bail!("unknown particleeffector opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParticleEmitterOp {
    Unknown1 { a: u16, b: u16, c: u16, d: u16 },
    Unknown2(u8),
    Unknown3 { a: i32, b: i32 },
    Unknown4 { a: u8, b: i8 },
    Unknown5(u16),
    Unknown6 { a: i32, b: i32 },
    Unknown7 { a: u16, b: u16 },
    Unknown8 { a: u16, b: u16 },
    Unknown9(Vec<u16>),
    Unknown10(Vec<u16>),
    Unknown12(i8),
    Unknown13(i8),
    Unknown14(u16),
    Unknown15(u16),
    Unknown16 { a: u8, b: u16, c: u16, d: u8 },
    Unknown17(u16),
    Unknown18(i32),
    Unknown19(u8),
    Unknown20(u8),
    Unknown21(u8),
    Unknown22(i32),
    Unknown23(u8),
    Unknown24No,
    Unknown25(Vec<u16>),
    Unknown26No,
    Unknown27(u16),
    Unknown28(u8),
    AngularVelocity(i16),
    AngularVelocityRange { min: i16, max: i16 },
    Unknown30Yes,
    Unknown31 { a: u16, b: u16 },
    LightingNo,
    Unknown33Yes,
    Unknown34No,
    Unknown35(i16),
    Unknown35Range { min: i16, max: i16, mode: u8 },
    Unknown36Yes,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParticleEmitterEntry {
    pub id: u32,
    pub ops: Vec<ParticleEmitterOp>,
}

pub fn parse_particle_emitter(id: u32, data: &[u8]) -> Result<ParticleEmitterEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("particleemitter {id} did not consume full payload");
                }
                return Ok(ParticleEmitterEntry { id, ops });
            }
            1 => ops.push(ParticleEmitterOp::Unknown1 {
                a: packet.g2()?,
                b: packet.g2()?,
                c: packet.g2()?,
                d: packet.g2()?,
            }),
            2 => ops.push(ParticleEmitterOp::Unknown2(packet.g1()?)),
            3 => ops.push(ParticleEmitterOp::Unknown3 {
                a: packet.g4s()?,
                b: packet.g4s()?,
            }),
            4 => ops.push(ParticleEmitterOp::Unknown4 {
                a: packet.g1()?,
                b: packet.g1s()?,
            }),
            5 => ops.push(ParticleEmitterOp::Unknown5(packet.g2()?)),
            6 => ops.push(ParticleEmitterOp::Unknown6 {
                a: packet.g4s()?,
                b: packet.g4s()?,
            }),
            7 => ops.push(ParticleEmitterOp::Unknown7 {
                a: packet.g2()?,
                b: packet.g2()?,
            }),
            8 => ops.push(ParticleEmitterOp::Unknown8 {
                a: packet.g2()?,
                b: packet.g2()?,
            }),
            9 => ops.push(ParticleEmitterOp::Unknown9(read_u16_list_g1_count(
                &mut packet,
            )?)),
            10 => ops.push(ParticleEmitterOp::Unknown10(read_u16_list_g1_count(
                &mut packet,
            )?)),
            12 => ops.push(ParticleEmitterOp::Unknown12(packet.g1s()?)),
            13 => ops.push(ParticleEmitterOp::Unknown13(packet.g1s()?)),
            14 => ops.push(ParticleEmitterOp::Unknown14(packet.g2()?)),
            15 => ops.push(ParticleEmitterOp::Unknown15(packet.g2()?)),
            16 => ops.push(ParticleEmitterOp::Unknown16 {
                a: packet.g1()?,
                b: packet.g2()?,
                c: packet.g2()?,
                d: packet.g1()?,
            }),
            17 => ops.push(ParticleEmitterOp::Unknown17(packet.g2()?)),
            18 => ops.push(ParticleEmitterOp::Unknown18(packet.g4s()?)),
            19 => ops.push(ParticleEmitterOp::Unknown19(packet.g1()?)),
            20 => ops.push(ParticleEmitterOp::Unknown20(packet.g1()?)),
            21 => ops.push(ParticleEmitterOp::Unknown21(packet.g1()?)),
            22 => ops.push(ParticleEmitterOp::Unknown22(packet.g4s()?)),
            23 => ops.push(ParticleEmitterOp::Unknown23(packet.g1()?)),
            24 => ops.push(ParticleEmitterOp::Unknown24No),
            25 => ops.push(ParticleEmitterOp::Unknown25(read_u16_list_g1_count(
                &mut packet,
            )?)),
            26 => ops.push(ParticleEmitterOp::Unknown26No),
            27 => ops.push(ParticleEmitterOp::Unknown27(packet.g2()?)),
            28 => ops.push(ParticleEmitterOp::Unknown28(packet.g1()?)),
            29 => {
                let mode = packet.g1()?;
                if mode == 0 {
                    ops.push(ParticleEmitterOp::AngularVelocity(packet.g2s()?));
                } else {
                    ops.push(ParticleEmitterOp::AngularVelocityRange {
                        min: packet.g2s()?,
                        max: packet.g2s()?,
                    });
                }
            }
            30 => ops.push(ParticleEmitterOp::Unknown30Yes),
            31 => ops.push(ParticleEmitterOp::Unknown31 {
                a: packet.g2()?,
                b: packet.g2()?,
            }),
            32 => ops.push(ParticleEmitterOp::LightingNo),
            33 => ops.push(ParticleEmitterOp::Unknown33Yes),
            34 => ops.push(ParticleEmitterOp::Unknown34No),
            35 => {
                let mode = packet.g1()?;
                if mode == 0 {
                    ops.push(ParticleEmitterOp::Unknown35(packet.g2s()?));
                } else {
                    ops.push(ParticleEmitterOp::Unknown35Range {
                        min: packet.g2s()?,
                        max: packet.g2s()?,
                        mode: packet.g1()?,
                    });
                }
            }
            36 => ops.push(ParticleEmitterOp::Unknown36Yes),
            opcode => bail!("unknown particleemitter opcode {opcode} in {id}"),
        }
    }
}

fn read_u16_list_g1_count(packet: &mut Packet<'_>) -> Result<Vec<u16>> {
    let count = usize::from(packet.g1()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(packet.g2()?);
    }
    Ok(values)
}
