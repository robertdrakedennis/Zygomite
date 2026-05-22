use crate::packet::Packet;
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VarDomain {
    Player = 0,
    Npc = 1,
    Client = 2,
    World = 3,
    Region = 4,
    Object = 5,
    Clan = 6,
    ClanSetting = 7,
    Controller = 8,
    PlayerGroup = 9,
    Global = 10,
}

impl VarDomain {
    pub fn from_id(id: u8) -> Result<Self> {
        match id {
            0 => Ok(Self::Player),
            1 => Ok(Self::Npc),
            2 => Ok(Self::Client),
            3 => Ok(Self::World),
            4 => Ok(Self::Region),
            5 => Ok(Self::Object),
            6 => Ok(Self::Clan),
            7 => Ok(Self::ClanSetting),
            8 => Ok(Self::Controller),
            9 => Ok(Self::PlayerGroup),
            10 => Ok(Self::Global),
            _ => bail!("unknown var domain id: {id}"),
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Npc => "npc",
            Self::Client => "client",
            Self::World => "world",
            Self::Region => "region",
            Self::Object => "object",
            Self::Clan => "clan",
            Self::ClanSetting => "clan_setting",
            Self::Controller => "controller",
            Self::PlayerGroup => "player_group",
            Self::Global => "global",
        }
    }

    pub fn from_label(label: &str) -> Result<Self> {
        match label {
            "player" => Ok(Self::Player),
            "npc" => Ok(Self::Npc),
            "client" => Ok(Self::Client),
            "world" => Ok(Self::World),
            "region" => Ok(Self::Region),
            "object" => Ok(Self::Object),
            "clan" => Ok(Self::Clan),
            "clan_setting" => Ok(Self::ClanSetting),
            "controller" => Ok(Self::Controller),
            "player_group" => Ok(Self::PlayerGroup),
            "global" => Ok(Self::Global),
            _ => bail!("unknown var domain label: {label}"),
        }
    }

    pub fn type_str(&self) -> &'static str {
        match self {
            Self::Player
            | Self::Npc
            | Self::Client
            | Self::World
            | Self::Region
            | Self::Object
            | Self::Clan
            | Self::ClanSetting
            | Self::Controller
            | Self::PlayerGroup
            | Self::Global => "number",
        }
    }
}

impl From<VarDomain> for u64 {
    fn from(domain: VarDomain) -> Self {
        domain as Self
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VarEntry {
    pub id: u32,
    pub domain: VarDomain,
    pub var_name: String,
    pub type_id: Option<u8>,
    pub lifetime: Option<&'static str>,
    pub transmit_level: Option<&'static str>,
    pub client_code: Option<u16>,
    pub domain_default: bool,
    pub wiki_sync: bool,
}

pub fn parse_var(domain: VarDomain, id: u32, data: &[u8]) -> Result<VarEntry> {
    let mut packet = Packet::new(data);
    let mut entry = VarEntry {
        id,
        domain,
        var_name: format!("var{}_{}", domain.as_label(), id),
        type_id: None,
        lifetime: None,
        transmit_level: None,
        client_code: None,
        domain_default: true,
        wiki_sync: false,
    };

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("var {id} did not consume full payload");
                }
                return Ok(entry);
            }
            3 => entry.type_id = Some(packet.g1()?),
            4 => entry.lifetime = Some(parse_lifetime(packet.g1()?)),
            5 => entry.transmit_level = Some(parse_transmit_level(packet.g1()?)),
            110 => entry.client_code = Some(packet.g2()?),
            7 => entry.domain_default = false,
            8 => entry.wiki_sync = true,
            opcode => bail!("unknown var opcode {opcode} for var {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VarBitEntry {
    pub id: u32,
    pub varbit_name: String,
    pub domain: Option<VarDomain>,
    pub base_var: Option<u32>,
    pub start_bit: Option<u8>,
    pub end_bit: Option<u8>,
    pub wiki_sync: bool,
}

pub fn parse_varbit(id: u32, data: &[u8]) -> Result<VarBitEntry> {
    let mut packet = Packet::new(data);
    let mut entry = VarBitEntry {
        id,
        varbit_name: format!("varbit_{id}"),
        domain: None,
        base_var: None,
        start_bit: None,
        end_bit: None,
        wiki_sync: false,
    };

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("varbit {id} did not consume full payload");
                }
                if let Some(domain) = entry.domain {
                    entry.varbit_name = format!("var{}bit_{}", domain.as_label(), id);
                }
                return Ok(entry);
            }
            1 => {
                let domain = VarDomain::from_id(packet.g1()?)?;
                entry.domain = Some(domain);
                let base = packet.gsmart2or4null()?;
                if base < 0 {
                    bail!("varbit {id} has null base var");
                }
                entry.base_var = Some(base as u32);
            }
            2 => {
                entry.start_bit = Some(packet.g1()?);
                entry.end_bit = Some(packet.g1()?);
            }
            16 => entry.wiki_sync = true,
            opcode => bail!("unknown varbit opcode {opcode} for varbit {id}"),
        }
    }
}

fn parse_lifetime(value: u8) -> &'static str {
    match value {
        0 => "temp",
        1 => "perm",
        2 => "serverperm",
        _ => "unknown",
    }
}

fn parse_transmit_level(value: u8) -> &'static str {
    match value {
        0 => "never",
        1 => "on_set_different",
        2 => "on_set_always",
        _ => "unknown",
    }
}
