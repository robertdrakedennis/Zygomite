use super::ScalarValue;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum QuestOp {
    Name(String),
    SortName(String),
    ProgressVarp {
        varp_id: u16,
        a: i32,
        b: i32,
    },
    ProgressVarbit {
        varbit_id: u16,
        a: i32,
        b: i32,
    },
    Unknown5(u16),
    Type(u8),
    Difficulty(u8),
    Members,
    Points(u8),
    Unknown10(i32),
    Unknown12(i32),
    QuestReq(u16),
    StatReq {
        stat_id: u8,
        level: u8,
    },
    PointsReq(u16),
    Icon(i32),
    VarpReq {
        varp_id: i32,
        min: i32,
        max: i32,
        text: String,
    },
    VarbitReq {
        varbit_id: i32,
        min: i32,
        max: i32,
        text: String,
    },
    Param {
        param_id: u32,
        value: ScalarValue,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct QuestEntry {
    pub id: u32,
    pub ops: Vec<QuestOp>,
}

pub fn parse_quest(id: u32, data: &[u8]) -> Result<QuestEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("quest {id} did not consume full payload");
                }
                return Ok(QuestEntry { id, ops });
            }
            1 => ops.push(QuestOp::Name(packet.gjstr2()?)),
            2 => ops.push(QuestOp::SortName(packet.gjstr2()?)),
            3 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::ProgressVarp {
                        varp_id: packet.g2()?,
                        a: packet.g4s()?,
                        b: packet.g4s()?,
                    });
                }
            }
            4 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::ProgressVarbit {
                        varbit_id: packet.g2()?,
                        a: packet.g4s()?,
                        b: packet.g4s()?,
                    });
                }
            }
            5 => ops.push(QuestOp::Unknown5(packet.g2()?)),
            6 => ops.push(QuestOp::Type(packet.g1()?)),
            7 => ops.push(QuestOp::Difficulty(packet.g1()?)),
            8 => ops.push(QuestOp::Members),
            9 => ops.push(QuestOp::Points(packet.g1()?)),
            10 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::Unknown10(packet.g4s()?));
                }
            }
            12 => ops.push(QuestOp::Unknown12(packet.g4s()?)),
            13 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::QuestReq(packet.g2()?));
                }
            }
            14 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::StatReq {
                        stat_id: packet.g1()?,
                        level: packet.g1()?,
                    });
                }
            }
            15 => ops.push(QuestOp::PointsReq(packet.g2()?)),
            17 => ops.push(QuestOp::Icon(packet.gsmart2or4null()?)),
            18 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::VarpReq {
                        varp_id: packet.g4s()?,
                        min: packet.g4s()?,
                        max: packet.g4s()?,
                        text: packet.gjstr()?,
                    });
                }
            }
            19 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(QuestOp::VarbitReq {
                        varbit_id: packet.g4s()?,
                        min: packet.g4s()?,
                        max: packet.g4s()?,
                        text: packet.gjstr()?,
                    });
                }
            }
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
                    ops.push(QuestOp::Param { param_id, value });
                }
            }
            opcode => bail!("unknown quest opcode {opcode} in {id}"),
        }
    }
}
