use super::parse_empty_config;
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct VarClientStringEntry {
    pub id: u32,
    pub scope_perm: bool,
}

pub fn parse_var_client_string(id: u32, data: &[u8]) -> Result<VarClientStringEntry> {
    let mut packet = Packet::new(data);
    let mut scope_perm = false;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("varcstr {id} did not consume full payload");
                }
                return Ok(VarClientStringEntry { id, scope_perm });
            }
            2 => scope_perm = true,
            opcode => bail!("unknown varcstr opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VarNpcBitEntry {
    pub id: u32,
}

pub fn parse_var_npc_bit(id: u32, data: &[u8]) -> Result<VarNpcBitEntry> {
    parse_empty_config("varnbit", id, data)?;
    Ok(VarNpcBitEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct VarSharedEntry {
    pub id: u32,
}

pub fn parse_var_shared(id: u32, data: &[u8]) -> Result<VarSharedEntry> {
    parse_empty_config("vars", id, data)?;
    Ok(VarSharedEntry { id })
}

#[derive(Clone, Debug, Serialize)]
pub struct VarSharedStringEntry {
    pub id: u32,
}

pub fn parse_var_shared_string(id: u32, data: &[u8]) -> Result<VarSharedStringEntry> {
    parse_empty_config("varsstr", id, data)?;
    Ok(VarSharedStringEntry { id })
}
