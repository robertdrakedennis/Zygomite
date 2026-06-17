use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct QuickChatCategoryLink {
    pub id: u16,
    pub shortcut: i8,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuickChatCategoryEntry {
    pub id: u32,
    pub desc: Option<String>,
    pub subcats: Vec<QuickChatCategoryLink>,
    pub phrases: Vec<QuickChatCategoryLink>,
    pub unknown4: bool,
}

pub fn parse_quickchatcat(id: u32, data: &[u8]) -> Result<QuickChatCategoryEntry> {
    let mut packet = Packet::new(data);
    let mut desc = None;
    let mut subcats = Vec::new();
    let mut phrases = Vec::new();
    let mut unknown4 = false;

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("quickchatcat {id} did not consume full payload");
                }
                return Ok(QuickChatCategoryEntry {
                    id,
                    desc,
                    subcats,
                    phrases,
                    unknown4,
                });
            }
            1 => desc = Some(packet.gjstr()?),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    subcats.push(QuickChatCategoryLink {
                        id: packet.g2()?,
                        shortcut: packet.g1s()?,
                    });
                }
            }
            3 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    phrases.push(QuickChatCategoryLink {
                        id: packet.g2()?,
                        shortcut: packet.g1s()?,
                    });
                }
            }
            4 => unknown4 = true,
            opcode => bail!("unknown quickchatcat opcode {opcode} in {id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuickChatDynamicCommand {
    ListDialog { enum_id: u16 },
    ObjDialog,
    CountDialog,
    StatBase { stat_id: u16 },
    EnumString { enum_id: u16, var_player_id: u16 },
    EnumStringClan { enum_id: u16 },
    VarPlayerInt { var_player_id: u16 },
    VarPlayerBit { varbit_id: u16 },
    ObjTradeDialog,
    EnumStringStatBase { enum_id: u16, stat_id: u16 },
    Unknown12,
    Unknown13,
    VarWorldInt { var_world_id: u16 },
    CombatLevel,
    EnumStringVarPlayerBit { enum_id: u16, varbit_id: u16 },
}

#[derive(Clone, Debug, Serialize)]
pub struct QuickChatPhraseEntry {
    pub id: u32,
    pub template: Option<String>,
    pub autoresponses: Vec<u16>,
    pub dynamic_commands: Vec<QuickChatDynamicCommand>,
    pub unknown4_no: bool,
}

pub fn parse_quickchatphrase(id: u32, data: &[u8]) -> Result<QuickChatPhraseEntry> {
    let mut packet = Packet::new(data);
    let mut template = None;
    let mut autoresponses = Vec::new();
    let mut dynamic_commands = Vec::new();
    let mut unknown4_no = false;
    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("quickchatphrase {id} did not consume full payload");
                }
                return Ok(QuickChatPhraseEntry {
                    id,
                    template,
                    autoresponses,
                    dynamic_commands,
                    unknown4_no,
                });
            }
            1 => template = Some(packet.gjstr()?),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    autoresponses.push(packet.g2()?);
                }
            }
            3 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    let command = packet.g2()?;
                    dynamic_commands.push(match command {
                        0 => QuickChatDynamicCommand::ListDialog {
                            enum_id: packet.g2()?,
                        },
                        1 => QuickChatDynamicCommand::ObjDialog,
                        2 => QuickChatDynamicCommand::CountDialog,
                        4 => QuickChatDynamicCommand::StatBase {
                            stat_id: packet.g2()?,
                        },
                        6 => QuickChatDynamicCommand::EnumString {
                            enum_id: packet.g2()?,
                            var_player_id: packet.g2()?,
                        },
                        7 => QuickChatDynamicCommand::EnumStringClan {
                            enum_id: packet.g2()?,
                        },
                        8 => QuickChatDynamicCommand::VarPlayerInt {
                            var_player_id: packet.g2()?,
                        },
                        9 => QuickChatDynamicCommand::VarPlayerBit {
                            varbit_id: packet.g2()?,
                        },
                        10 => QuickChatDynamicCommand::ObjTradeDialog,
                        11 => QuickChatDynamicCommand::EnumStringStatBase {
                            enum_id: packet.g2()?,
                            stat_id: packet.g2()?,
                        },
                        12 => QuickChatDynamicCommand::Unknown12,
                        13 => QuickChatDynamicCommand::Unknown13,
                        14 => QuickChatDynamicCommand::VarWorldInt {
                            var_world_id: packet.g2()?,
                        },
                        15 => QuickChatDynamicCommand::CombatLevel,
                        16 => QuickChatDynamicCommand::EnumStringVarPlayerBit {
                            enum_id: packet.g2()?,
                            varbit_id: packet.g2()?,
                        },
                        value => bail!("invalid quickchat dynamiccommand {value} in {id}"),
                    });
                }
            }
            4 => unknown4_no = true,
            opcode => bail!("unknown quickchatphrase opcode {opcode} in {id}"),
        }
    }
}
