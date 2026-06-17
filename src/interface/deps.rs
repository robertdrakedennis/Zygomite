//! Component dependency-graph extraction.
//!
//! Walks the same component wire format as [`super::parse_component`] but, instead of
//! formatting human-readable lines, records the ids each component references
//! ([`ComponentDeps`]). Used to build the interface dependency graph.

use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;
use std::collections::HashSet;

use super::{TransmitListType, format_if_type};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(tag = "domain", content = "id")]
pub enum VarTransmitRef {
    Player(u32),
    Npc(u32),
    Client(u32),
    World(u32),
    Region(u32),
    Object(u32),
    Clan(u32),
    ClanSetting(u32),
    Controller(u32),
    Global(u32),
    PlayerGroup(u32),
    VarClientString(u32),
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentDeps {
    pub component_type: String,
    pub name: Option<String>,
    pub children: Vec<u32>,
    pub scripts: HashSet<u32>,
    /// Script ids from the component `onload` hook only.
    pub onload_scripts: HashSet<u32>,
    pub varps: HashSet<VarTransmitRef>,
    pub varbits: HashSet<u32>,
    pub invs: HashSet<u32>,
    pub stats: HashSet<u32>,
    pub graphics: HashSet<u32>,
    pub models: HashSet<u32>,
    pub cursors: HashSet<u32>,
    pub stylesheets: HashSet<u32>,
    pub params: HashSet<u32>,
    pub seqs: HashSet<u32>,
    pub fontmetrics: HashSet<u32>,
    pub textures: HashSet<u32>,
    pub enums: HashSet<u32>,
}

pub fn parse_component_deps(component_id: u32, data: &[u8], build: u32) -> Result<ComponentDeps> {
    let mut packet = Packet::new(data);
    let mut deps = ComponentDeps {
        component_type: "unknown".to_string(),
        name: None,
        children: Vec::new(),
        scripts: HashSet::new(),
        onload_scripts: HashSet::new(),
        varps: HashSet::new(),
        varbits: HashSet::new(),
        invs: HashSet::new(),
        stats: HashSet::new(),
        graphics: HashSet::new(),
        models: HashSet::new(),
        cursors: HashSet::new(),
        stylesheets: HashSet::new(),
        params: HashSet::new(),
        seqs: HashSet::new(),
        fontmetrics: HashSet::new(),
        textures: HashSet::new(),
        enums: HashSet::new(),
    };

    let mut version = i16::from(packet.g1()?);
    if build < 566 && version != i16::from(u8::MAX) {
        return Ok(deps);
    }
    if version == i16::from(u8::MAX) {
        version = -1;
    }

    let mut if_type = packet.g1()?;
    deps.component_type = format_if_type(i32::from(if_type & 0x7F)).to_string();

    if (if_type & 128) != 0 {
        if_type &= 127;
        let name = packet.gjstr()?;
        if !name.is_empty() {
            deps.name = Some(name);
        }
    }

    let _contenttype = packet.g2()?;
    let _x = packet.g2s()?;
    let _y = packet.g2s()?;
    let _width = packet.g2()?;
    let _height = packet.g2()?;

    let mut width_mode: i8 = 0;
    let mut height_mode: i8 = 0;
    if build >= 493 {
        width_mode = packet.g1s()?;
        height_mode = packet.g1s()?;
        let _x_mode = packet.g1s()?;
        let _y_mode = packet.g1s()?;
    }

    if width_mode == 4 || height_mode == 4 {
        let _aspect_width = packet.g2()?;
        let _aspect_height = packet.g2()?;
    }

    let layer = packet.g2null()?;
    if layer != -1 {
        deps.children.push(layer as u32);
    }

    let _flags = packet.g1()?;

    // Use a closure to catch errors and return partial results
    let parse_result = (|| -> Result<()> {
        let parsed_body = match if_type {
            6 => {
                collect_model_deps(
                    &mut deps,
                    &mut packet,
                    build,
                    width_mode != 0,
                    height_mode != 0,
                )?;
                true
            }
            0 => {
                let _scroll_width = packet.g2()?;
                let _scroll_height = packet.g2()?;
                if version == -1 && build >= 495 {
                    let _ = packet.g1()?;
                } else if version >= 9 || version >= 6 {
                    if version >= 9 {
                        let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                    } else {
                        let _ = [packet.g2()?, packet.g2()?, packet.g2()?, packet.g2()?];
                    }
                }
                true
            }
            3 => {
                let _ = packet.g4s()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                true
            }
            4 => {
                collect_text_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            5 => {
                collect_graphic_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            9 => {
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                if build >= 493 {
                    let _ = packet.g1()?;
                }
                true
            }
            10 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            11 => {
                let _ = packet.g2()?;
                let _ = packet.g2()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                true
            }
            12 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            13 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g2()?;
                if version >= 9 {
                    let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                }
                if version >= 7 {
                    let _ = packet.g1()?;
                }
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                collect_scrollbar_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            15 => {
                let _ = packet.g2()?;
                let _ = packet.g2()?;
                let _ = packet.g1()?;
                let _ = packet.g2()?;
                let _ = packet.g2()?;
                let _ = packet.g1()?;
                true
            }
            16 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                if version >= 9 {
                    let _ = packet.g1()?;
                }
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                for _ in 0..packet.g2()? {
                    let _ = packet.gjstr()?;
                }
                for _ in 0..packet.g2()? {
                    let _ = packet.g4s()?;
                }
                for _ in 0..packet.g2()? {
                    let _ = packet.g2()?;
                }
                if version >= 9 {
                    let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                    let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                }
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                collect_scrollbar_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            26 => {
                let _ = packet.g1()?;
                for _ in 0..packet.g2()? {
                    let _ = packet.g2()?;
                }
                let _ = packet.gjstr()?;
                let _ = packet.g1()?;
                for _ in 0..packet.g2()? {
                    let _ = packet.gjstr()?;
                }
                for _ in 0..packet.g2()? {
                    let _ = packet.g4s()?;
                }
                true
            }
            _ => false,
        };

        if parsed_body {
            collect_common_tail_deps(&mut deps, &mut packet, version, build)?;
        }

        Ok(())
    })();

    // Return partial results even if parsing failed
    if let Err(e) = parse_result {
        eprintln!(
            "parse_component_deps partial failure for comp {component_id} (type={}): {e}",
            deps.component_type
        );
    }

    Ok(deps)
}

fn collect_text_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    collect_text_part_deps(deps, packet, version, build)
}

fn collect_graphic_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    collect_sprite_part_deps(deps, packet, version, build)
}

fn collect_model_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    build: u32,
    has_width_mode: bool,
    has_height_mode: bool,
) -> Result<()> {
    let model = if build < 681 {
        packet.g2null()?
    } else {
        packet.gsmart2or4null()?
    };
    if model != -1 {
        deps.models.insert(model as u32);
    }

    if build < 619 {
        let _ = packet.g2s()?;
        let _ = packet.g2s()?;
        let _ = packet.g2()?;
        let _ = packet.g2()?;
        let _ = packet.g2()?;
        let _ = packet.g2()?;
        let modelanim = if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        };
        if modelanim != -1 {
            deps.seqs.insert(modelanim as u32);
        }
        let _ = packet.g1()?;
        if build >= 493 {
            let _ = packet.g2()?;
        }
        if build >= 501 {
            let _ = packet.g2()?;
            let _ = packet.g1()?;
        }
    } else {
        let model_flags = packet.g1()?;
        let has_transform = (model_flags & 1) != 0;
        let has_precise_zoom = (model_flags & 2) != 0;
        if has_transform {
            let _ = packet.g2s()?;
            let _ = packet.g2s()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
        } else if has_precise_zoom {
            let _ = packet.g2s()?;
            let _ = packet.g2s()?;
            let _ = packet.g2s()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
        }
        let modelanim = if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        };
        if modelanim != -1 {
            deps.seqs.insert(modelanim as u32);
        }
    }

    if has_width_mode {
        let _ = packet.g2()?;
    }
    if has_height_mode {
        let _ = packet.g2()?;
    }

    Ok(())
}

fn collect_text_part_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    let textfont = if build < 800 {
        if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        }
    } else {
        packet.gsmart2or4null()?
    };
    if textfont != -1 {
        deps.fontmetrics.insert(textfont as u32);
    }

    if version >= 2 {
        let _ = packet.g1()?;
    }
    let _ = packet.gjstr()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g4s()?;
    if build >= 582 {
        let _ = packet.g1()?;
    }
    if version >= 0 {
        let _ = packet.g1()?;
    }
    Ok(())
}

fn collect_sprite_part_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    let graphic = packet.g4s()?;
    if graphic != -1 {
        deps.graphics.insert(graphic as u32);
    }
    let _ = packet.g2()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g4s()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    if build >= 537 {
        let _ = packet.g4s()?;
    }
    if version >= 3 {
        let _ = packet.g1()?;
    }
    if version >= 6 {
        let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
    }
    Ok(())
}

fn collect_scrollbar_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    collect_sprite_part_deps(deps, packet, version, build)?;
    collect_sprite_part_deps(deps, packet, version, build)?;
    collect_sprite_part_deps(deps, packet, version, build)?;
    Ok(())
}

fn collect_common_tail_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    if version >= 6 {
        let stylesheet = packet.g4s()?;
        if stylesheet != -1 {
            deps.stylesheets.insert(stylesheet as u32);
        }
    }

    if version >= 9 {
        let _ = packet.g1()?;
    }

    let events = if version < 6 {
        packet.g3()?
    } else {
        packet.g4s()? as u32
    };

    let targetmask = (events >> 11) & 0x7F;

    if build < 499 {
    } else if build < 530 {
        let keycount = packet.g1()?;
        for _ in 0..keycount {
            let _ = packet.g1s()?;
        }
        if keycount > 0 && build >= 509 {
            let modcount = packet.g1()?;
            for _ in 0..modcount {
                let _ = packet.g1s()?;
            }
        }
    } else {
        let mut value = packet.g1()?;
        while value != 0 {
            let _index = (value >> 4).checked_sub(1);
            let _ = packet.g1()?;
            let _ = packet.g1s()?;
            let _ = packet.g1s()?;
            value = packet.g1()?;
        }
    }

    let _opbase = packet.gjstr()?;

    let opinfo = packet.g1()?;
    let opcount = opinfo & 15;
    let opcursorcount = opinfo >> 4;

    for _ in 0..opcount {
        let _ = packet.gjstr()?;
    }

    if opcursorcount > 0 {
        let _ = packet.g1()?;
        let cursor = packet.g2()?;
        if cursor != 0xFFFF {
            deps.cursors.insert(u32::from(cursor));
        }
    }
    if opcursorcount > 1 {
        let _ = packet.g1()?;
        let cursor = packet.g2()?;
        if cursor != 0xFFFF {
            deps.cursors.insert(u32::from(cursor));
        }
    }

    if build >= 537 {
        let _ = packet.gjstr()?;
    }

    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;

    let _targetverb = packet.gjstr()?;

    if build >= 530 && targetmask != 0 {
        let _ = packet.g2null()?;
        let _ = packet.g2null()?;
        let _ = packet.g2null()?;
    }

    if version >= 0 {
        let _ = packet.g2null()?;
    }

    if version >= 0 {
        let intparamcount = packet.g1()?;
        for _ in 0..intparamcount {
            let param_id = packet.g3()?;
            deps.params.insert(param_id);
            let _ = packet.g4s()?;
        }

        let stringparamcount = packet.g1()?;
        for _ in 0..stringparamcount {
            let param_id = packet.g3()?;
            deps.params.insert(param_id);
            let _ = packet.gjstr2()?;
        }
    }

    collect_onload_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;

    if version >= 0 {
        collect_hook_deps(deps, packet)?;
    }

    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;

    if build < 459 {
        collect_hook_deps(deps, packet)?;
    }

    collect_hook_deps(deps, packet)?;

    if build >= 506 {
        collect_hook_deps(deps, packet)?;
        collect_hook_deps(deps, packet)?;
    }

    if version >= 6 {
        collect_hook_deps(deps, packet)?;
        collect_hook_deps(deps, packet)?;
        collect_hook_deps(deps, packet)?;
    }

    if version >= 8 {
        collect_hook_deps(deps, packet)?;
    }

    if build >= 459 {
        collect_transmit_list_deps(deps, packet, TransmitListType::VarPlayer)?;
        collect_transmit_list_deps(deps, packet, TransmitListType::Inv)?;
        collect_transmit_list_deps(deps, packet, TransmitListType::Stat)?;
    }

    if build >= 506 {
        collect_transmit_list_deps(deps, packet, TransmitListType::VarClient)?;
        collect_transmit_list_deps(deps, packet, TransmitListType::VarClientString)?;
    }

    Ok(())
}

fn collect_onload_hook_deps(deps: &mut ComponentDeps, packet: &mut Packet<'_>) -> Result<()> {
    collect_hook_deps_inner(deps, packet, true)
}

fn collect_hook_deps(deps: &mut ComponentDeps, packet: &mut Packet<'_>) -> Result<()> {
    collect_hook_deps_inner(deps, packet, false)
}

fn collect_hook_deps_inner(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    onload: bool,
) -> Result<()> {
    let count = usize::from(packet.g1()?);
    if count == 0 {
        return Ok(());
    }

    let _unknown = packet.g1()?;
    let script = packet.g4s()?;
    if script != -1 {
        let script_id = script as u32;
        deps.scripts.insert(script_id);
        if onload {
            deps.onload_scripts.insert(script_id);
        }
    }
    for _ in 0..(count - 1) {
        match packet.g1()? {
            0 => {
                let _ = packet.g4s()?;
            }
            1 => {
                let _ = packet.gjstr()?;
            }
            value => bail!("unexpected hook argument type {value}"),
        }
    }

    Ok(())
}

fn collect_transmit_list_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    kind: TransmitListType,
) -> Result<()> {
    let count = usize::from(packet.g1()?);
    if count == 0 {
        return Ok(());
    }

    for _ in 0..count {
        let id = packet.g4s()?;
        match kind {
            TransmitListType::VarPlayer => {
                deps.varps.insert(VarTransmitRef::Player(id as u32));
            }
            TransmitListType::Inv => {
                deps.invs.insert(id as u32);
            }
            TransmitListType::Stat => {
                deps.stats.insert(id as u32);
            }
            TransmitListType::VarClient => {
                deps.varps.insert(VarTransmitRef::Client(id as u32));
            }
            TransmitListType::VarClientString => {
                deps.varps
                    .insert(VarTransmitRef::VarClientString(id as u32));
            }
        }
    }
    Ok(())
}
