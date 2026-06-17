use super::ScalarValue;
use super::StructParamEntry;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct StructEntry {
    pub id: u32,
    pub params: Vec<StructParamEntry>,
}

pub fn parse_struct(id: u32, data: &[u8]) -> Result<StructEntry> {
    let mut packet = Packet::new(data);
    let mut params = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("struct {id} did not consume full payload");
                }
                return Ok(StructEntry { id, params });
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
                    params.push(StructParamEntry { param_id, value });
                }
            }
            opcode => bail!("unknown struct opcode {opcode} in {id}"),
        }
    }
}
