use super::ScalarValue;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ParamEntry {
    pub id: u32,
    pub type_char: Option<u8>,
    pub type_id: Option<u16>,
    pub default: Option<ScalarValue>,
    pub autodisable: bool,
}

pub fn parse_param(id: u32, data: &[u8]) -> Result<ParamEntry> {
    let mut packet = Packet::new(data);
    let mut entry = ParamEntry {
        id,
        type_char: None,
        type_id: None,
        default: None,
        autodisable: true,
    };

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("param {id} did not consume full payload");
                }
                return Ok(entry);
            }
            1 => entry.type_char = Some(packet.g1()?),
            2 => entry.default = Some(ScalarValue::Int(packet.g4s()?)),
            4 => entry.autodisable = false,
            5 => entry.default = Some(ScalarValue::Str(packet.gjstr()?)),
            101 => entry.type_id = Some(packet.gsmart1or2()?),
            opcode => bail!("unknown param opcode {opcode} in {id}"),
        }
    }
}
