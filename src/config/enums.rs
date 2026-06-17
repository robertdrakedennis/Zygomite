use super::ScalarValue;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct EnumPair {
    pub key: i32,
    pub value: ScalarValue,
    pub dense: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnumEntry {
    pub id: u32,
    pub input_type_char: Option<u8>,
    pub output_type_char: Option<u8>,
    pub input_type_id: Option<u16>,
    pub output_type_id: Option<u16>,
    pub default: Option<ScalarValue>,
    pub values: Vec<EnumPair>,
}

pub fn parse_enum(id: u32, data: &[u8]) -> Result<EnumEntry> {
    let mut packet = Packet::new(data);
    let mut entry = EnumEntry {
        id,
        input_type_char: None,
        output_type_char: None,
        input_type_id: None,
        output_type_id: None,
        default: None,
        values: Vec::new(),
    };

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("enum {id} did not consume full payload");
                }
                return Ok(entry);
            }
            1 => entry.input_type_char = Some(packet.g1()?),
            2 => entry.output_type_char = Some(packet.g1()?),
            3 => entry.default = Some(ScalarValue::Str(packet.gjstr()?)),
            4 => entry.default = Some(ScalarValue::Int(packet.g4s()?)),
            5 => {
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: packet.g4s()?,
                        value: ScalarValue::Str(packet.gjstr()?),
                        dense: false,
                    });
                }
            }
            6 => {
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: packet.g4s()?,
                        value: ScalarValue::Int(packet.g4s()?),
                        dense: false,
                    });
                }
            }
            7 => {
                let _capacity = packet.g2()?;
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: i32::from(packet.g2()?),
                        value: ScalarValue::Str(packet.gjstr()?),
                        dense: true,
                    });
                }
            }
            8 => {
                let _capacity = packet.g2()?;
                let count = usize::from(packet.g2()?);
                for _ in 0..count {
                    entry.values.push(EnumPair {
                        key: i32::from(packet.g2()?),
                        value: ScalarValue::Int(packet.g4s()?),
                        dense: true,
                    });
                }
            }
            101 => entry.input_type_id = Some(packet.gsmart1or2()?),
            102 => entry.output_type_id = Some(packet.gsmart1or2()?),
            opcode => bail!("unknown enum opcode {opcode} in {id}"),
        }
    }
}
