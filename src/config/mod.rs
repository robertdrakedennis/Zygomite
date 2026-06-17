use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;
use std::fmt::Write;

mod achievement;
mod bas;
mod db;
mod enums;
mod floor;
mod gfx;
mod hitmark;
mod idk;
mod loc;
mod material;
mod mel;
mod npc;
mod obj;
mod param;
mod particle;
mod quest;
mod quickchat;
mod seq;
mod spot;
mod structs;
mod texture;
mod vars;
mod water;
mod world;
pub use achievement::*;
pub use bas::*;
pub use db::*;
pub use enums::*;
pub use floor::*;
pub use gfx::*;
pub use hitmark::*;
pub use idk::*;
pub use loc::*;
pub use material::*;
pub use mel::*;
pub use npc::*;
pub use obj::*;
pub use param::*;
pub use particle::*;
pub use quest::*;
pub use quickchat::*;
pub use seq::*;
pub use spot::*;
pub use structs::*;
pub use texture::*;
pub use vars::*;
pub use water::*;
pub use world::*;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum ScalarValue {
    Int(i32),
    Long(i64),
    Str(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct StructParamEntry {
    pub param_id: u32,
    pub value: ScalarValue,
}

#[derive(Clone, Debug, Serialize)]
pub struct OpListEntry {
    pub id: u32,
    pub ops: Vec<String>,
}

pub(crate) fn parse_multi_variants_block(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    let _unused = packet.g2()?;
    let varbit = packet.g2null()?;
    let varplayer = packet.g2null()?;

    if varbit != -1 {
        ops.push(format!("multivar=varbit:{varbit}"));
    }
    if varplayer != -1 {
        ops.push(format!("multivar=varp:{varplayer}"));
    }

    let flags = packet.g1()?;

    if (flags & 1) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                let mut line = format!(
                    "multimodel={value},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    gsmart2or4s(packet)?
                );
                let n = packet.g1()?;
                if n >= 1 {
                    let _ = write!(line, ",{}", packet.g1()?);
                }
                if n >= 2 {
                    let _ = write!(line, ",{}", packet.g1()?);
                }
                if n >= 3 {
                    let _ = write!(line, ",{}", packet.g1()?);
                }
                ops.push(line);
            }
        }
    }

    if (flags & 2) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                ops.push(format!(
                    "multiheadmodel={value},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    gsmart2or4s(packet)?
                ));
            }
        }
    }

    if (flags & 4) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                ops.push(format!(
                    "multiretex={value},{},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?
                ));
            }
        }
    }

    if (flags & 8) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            let length2 = usize::from(packet.g1()?);
            for _ in 0..length2 {
                ops.push(format!(
                    "multirecol={value},{},{},{},{}",
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?,
                    packet.g2()?
                ));
            }
        }
    }

    if (flags & 16) != 0 {
        let length = usize::from(packet.g1()?);
        for _ in 0..length {
            let value = packet.g1()?;
            ops.push(format!(
                "multitint={value},{},{},{},{},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g1()?,
                packet.g1()?,
                packet.g1()?,
                packet.g1()?
            ));
        }
    }

    ops.push(format!("multidefault={}", packet.g2()?));
    Ok(())
}

pub(crate) fn parse_param_ops(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    let count = usize::from(packet.g1()?);
    for _ in 0..count {
        let is_string = packet.g1()? == 1;
        let param = packet.g3()?;
        if is_string {
            ops.push(format!("param={param},{}", packet.gjstr()?));
        } else {
            ops.push(format!("param={param},{}", packet.g4s()?));
        }
    }
    Ok(())
}

pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

pub(crate) fn parse_empty_config(kind: &str, id: u32, data: &[u8]) -> Result<()> {
    let mut packet = Packet::new(data);
    let opcode = packet.g1()?;
    if opcode != 0 {
        bail!("unknown {kind} opcode {opcode} in {id}");
    }
    if !packet.is_done() {
        bail!("{kind} {id} did not consume full payload");
    }
    Ok(())
}

pub(crate) fn gfloat_be(packet: &mut Packet<'_>) -> Result<f32> {
    Ok(f32::from_bits(packet.g4s()? as u32))
}

pub(crate) fn gsmart2or4s(packet: &mut Packet<'_>) -> Result<i32> {
    let first = packet
        .slice(packet.pos(), packet.pos() + 1)
        .context("gsmart2or4s out of bounds")?[0];
    if (first as i8) < 0 {
        Ok(packet.g4s()? & i32::MAX)
    } else {
        Ok(i32::from(packet.g2()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_param_payload() {
        let bytes = [1, b'i', 2, 0, 0, 0, 123, 4, 0];
        let parsed = parse_param(10, &bytes).expect("parse param");
        assert_eq!(10, parsed.id);
        assert_eq!(Some(b'i'), parsed.type_char);
        assert!(!parsed.autodisable);
        assert!(matches!(parsed.default, Some(ScalarValue::Int(123))));
    }

    #[test]
    fn parses_enum_sparse_string_values() {
        let bytes = [1, b'i', 2, b's', 5, 0, 1, 0, 0, 0, 1, b'x', 0, 0];
        let parsed = parse_enum(20, &bytes).expect("parse enum");
        assert_eq!(Some(b'i'), parsed.input_type_char);
        assert_eq!(Some(b's'), parsed.output_type_char);
        assert_eq!(1, parsed.values.len());
        assert_eq!(1, parsed.values[0].key);
        assert!(matches!(parsed.values[0].value, ScalarValue::Str(ref s) if s == "x"));
    }

    #[test]
    fn parses_dbtable_column_schema() {
        let bytes = [1, 1, 0, 1, 0, 255, 0];
        let parsed = parse_dbtable(40, &bytes).expect("parse dbtable");
        assert_eq!(40, parsed.id);
        assert_eq!(1, parsed.columns.len());
        assert_eq!(0, parsed.columns[0].column);
        assert_eq!(vec![0], parsed.columns[0].tuple_types);
    }

    #[test]
    fn parses_dbrow_table_and_column_values() {
        let bytes = [3, 0, 0, 1, 0, 1, 0, 0, 0, 5, 255, 4, 42, 0];
        let parsed = parse_dbrow(41, &bytes).expect("parse dbrow");
        assert_eq!(41, parsed.id);
        assert_eq!(Some(42), parsed.table);
        assert_eq!(1, parsed.columns.len());
        assert_eq!(0, parsed.columns[0].column);
        assert_eq!(1, parsed.columns[0].rows.len());
        assert!(matches!(parsed.columns[0].rows[0][0], ScalarValue::Int(5)));
    }

    #[test]
    fn parses_struct_params() {
        let bytes = [249, 2, 1, 0, 0, 5, b'a', 0, 0, 0, 0, 7, 0, 0, 0, 123, 0];
        let parsed = parse_struct(55, &bytes).expect("parse struct");
        assert_eq!(55, parsed.id);
        assert_eq!(2, parsed.params.len());
        assert_eq!(5, parsed.params[0].param_id);
        assert!(matches!(
            parsed.params[0].value,
            ScalarValue::Str(ref value) if value == "a"
        ));
        assert_eq!(7, parsed.params[1].param_id);
        assert!(matches!(parsed.params[1].value, ScalarValue::Int(123)));
    }

    #[test]
    fn parses_inv_size_and_stock() {
        let bytes = [2, 0, 28, 4, 1, 0, 10, 0, 99, 0];
        let parsed = parse_inv(9, &bytes).expect("parse inv");
        assert_eq!(9, parsed.id);
        assert_eq!(Some(28), parsed.size);
        assert_eq!(1, parsed.stocks.len());
        assert_eq!(10, parsed.stocks[0].obj_id);
        assert_eq!(99, parsed.stocks[0].count);
    }

    #[test]
    fn parses_cursor_graphic_and_hotspot() {
        let bytes = [1, 0, 42, 2, 3, 4, 0];
        let parsed = parse_cursor(12, &bytes).expect("parse cursor");
        assert_eq!(12, parsed.id);
        assert_eq!(Some(42), parsed.graphic);
        let hotspot = parsed.hotspot.expect("missing hotspot");
        assert_eq!(3, hotspot.x);
        assert_eq!(4, hotspot.y);
    }

    #[test]
    fn parses_seqgroup_sections() {
        let bytes = [2, 2, 1, 2, 3, 9, 1, 5, 7, 4, 8, 1, 6, 10, 0];
        let parsed = parse_seqgroup(77, &bytes).expect("parse seqgroup");
        assert_eq!(77, parsed.id);
        assert_eq!(vec![1, 2], parsed.walkmerge);
        assert_eq!(Some(9), parsed.unknown3_default);
        assert_eq!(1, parsed.unknown3.len());
        assert_eq!(5, parsed.unknown3[0].label);
        assert_eq!(7, parsed.unknown3[0].value);
        assert_eq!(Some(8), parsed.unknown4_default);
        assert_eq!(1, parsed.unknown4.len());
        assert_eq!(6, parsed.unknown4[0].label);
        assert_eq!(10, parsed.unknown4[0].value);
    }

    #[test]
    fn parses_empty_controller_and_category() {
        let controller = parse_controller(1, &[0]).expect("parse controller");
        assert_eq!(1, controller.id);
        let category = parse_category(2, &[0]).expect("parse category");
        assert_eq!(2, category.id);
    }

    #[test]
    fn parses_empty_family_entries() {
        assert_eq!(3, parse_area(3, &[0]).expect("parse area").id);
        assert_eq!(4, parse_hunt(4, &[0]).expect("parse hunt").id);
        assert_eq!(5, parse_mesanim(5, &[0]).expect("parse mesanim").id);
        assert_eq!(6, parse_itemcode(6, &[0]).expect("parse itemcode").id);
        assert_eq!(
            7,
            parse_gamelogevent(7, &[0]).expect("parse gamelogevent").id
        );
        assert_eq!(8, parse_bugtemplate(8, &[0]).expect("parse bugtemplate").id);
        assert_eq!(9, parse_var_npc_bit(9, &[0]).expect("parse varnbit").id);
        assert_eq!(10, parse_var_shared(10, &[0]).expect("parse vars").id);
        assert_eq!(
            11,
            parse_var_shared_string(11, &[0]).expect("parse varsstr").id
        );
    }

    #[test]
    fn parses_var_client_string_scope() {
        let scoped = parse_var_client_string(12, &[2, 0]).expect("parse varcstr");
        assert_eq!(12, scoped.id);
        assert!(scoped.scope_perm);
        let unscoped = parse_var_client_string(13, &[0]).expect("parse varcstr");
        assert!(!unscoped.scope_perm);
    }

    #[test]
    fn parses_bas_opcode52_pre916_shape() {
        let bytes = [52, 1, 0, 5, 7, 0];
        let parsed = parse_bas(10, &bytes, 910).expect("parse bas pre916");
        assert_eq!(10, parsed.id);
        assert_eq!(1, parsed.ops.len());
        assert_eq!("randomreadyanim=5,7", parsed.ops[0]);
    }

    #[test]
    fn parses_bas_opcode52_post916_shape() {
        let bytes = [52, 1, 0, 5, 7, 0, 0];
        let parsed = parse_bas(11, &bytes, 947).expect("parse bas post916");
        assert_eq!(11, parsed.id);
        assert_eq!(1, parsed.ops.len());
        assert_eq!("randomreadyanim=5,7", parsed.ops[0]);
    }

    #[test]
    fn parses_underlay_fields() {
        let bytes = [1, 1, 2, 3, 2, 0, 5, 3, 0, 44, 4, 5, 0];
        let parsed = parse_underlay(1, &bytes).expect("parse underlay");
        assert_eq!(1, parsed.id);
        assert_eq!(Some(0x0001_0203), parsed.colour);
        assert_eq!(Some(5), parsed.material);
        assert_eq!(Some(44), parsed.texture_scale);
        assert!(!parsed.hardshadow);
        assert!(!parsed.occlude);
    }

    #[test]
    fn parses_overlay_fields() {
        let bytes = [
            1, 0xAA, 0xBB, 0xCC, 3, 0xFF, 0xFF, 5, 6, b'n', 0, 7, 1, 2, 3, 8, 9, 0, 10, 10, 11, 3,
            12, 13, 0x10, 0x20, 0x30, 14, 7, 15, 0, 99, 16, 8, 20, 0, 5, 21, 9, 22, 0, 6, 0,
        ];
        let parsed = parse_overlay(2, &bytes).expect("parse overlay");
        assert_eq!(2, parsed.id);
        assert_eq!(Some(0x00AA_BBCC), parsed.colour);
        assert_eq!(Some(-1), parsed.material);
        assert!(!parsed.occlude);
        assert_eq!(Some("n".to_string()), parsed.debugname);
        assert_eq!(Some(0x0001_0203), parsed.mapcolour);
        assert!(parsed.toggles.unknown8);
        assert_eq!(Some(10), parsed.texture_scale);
        assert!(!parsed.hardshadow);
        assert_eq!(Some(3), parsed.priority);
        assert!(parsed.toggles.smoothedges);
        assert_eq!(Some(0x0010_2030), parsed.waterfog_colour);
        assert_eq!(Some(7), parsed.waterfog_scale);
        assert_eq!(Some(99), parsed.unknown15);
        assert_eq!(Some(8), parsed.waterfog_offset);
        assert_eq!(Some(5), parsed.waterfog_unknown_a);
        assert_eq!(Some(9), parsed.waterfog_unknown_b);
        assert_eq!(Some(6), parsed.waterfog_unknown_c);
    }

    #[test]
    fn parses_msi_fields() {
        let bytes = [1, 0, 3, 2, 0, 0, 1, 3, 4, 5, 0];
        let parsed = parse_msi(3, &bytes).expect("parse msi");
        assert_eq!(3, parsed.id);
        assert_eq!(Some(3), parsed.graphic);
        assert_eq!(Some(1), parsed.unknown2);
        assert!(parsed.unknown3);
        assert!(parsed.unknown4);
        assert!(parsed.unknown5);
    }

    #[test]
    fn parses_skybox_fields() {
        let bytes = [1, 0, 2, 2, 2, 0, 3, 0, 4, 3, 9, 4, 7, 5, 0, 11, 6, 0, 12, 0];
        let parsed = parse_skybox(4, &bytes).expect("parse skybox");
        assert_eq!(4, parsed.id);
        assert_eq!(Some(2), parsed.material);
        assert_eq!(vec![3, 4], parsed.unknown2);
        assert_eq!(Some(9), parsed.unknown3);
        assert_eq!(Some(7), parsed.fillmode);
        assert_eq!(Some(11), parsed.unknown5);
        assert_eq!(Some(12), parsed.unknown6);
    }

    #[test]
    fn parses_worldarea_fields() {
        let bytes = [
            2, 1, 2, 3, 3, 0, 0, 0, 1, 0, 0, 0, 2, 4, 0, 0, 0, 3, 0, 0, 0, 0, 0,
        ];
        let parsed = parse_worldarea(5, &bytes).expect("parse worldarea");
        assert_eq!(5, parsed.id);
        assert_eq!(Some(0x0001_0203), parsed.colour);
        assert_eq!(1, parsed.impostor_squares.len());
        assert_eq!(1, parsed.impostor_squares[0].start);
        assert_eq!(2, parsed.impostor_squares[0].end);
        assert_eq!(1, parsed.impostor_zones.len());
        assert_eq!(3, parsed.impostor_zones[0].anchor);
        assert_eq!("0_0_0_0_0,0,0", parsed.impostor_zones[0].template);
    }

    #[test]
    fn parses_quickchatcat_fields() {
        let bytes = [1, b'd', 0, 2, 1, 0, 5, 255, 3, 1, 0, 6, 1, 4, 0];
        let parsed = parse_quickchatcat(6, &bytes).expect("parse quickchatcat");
        assert_eq!(6, parsed.id);
        assert_eq!(Some("d".to_string()), parsed.desc);
        assert_eq!(1, parsed.subcats.len());
        assert_eq!(5, parsed.subcats[0].id);
        assert_eq!(-1, parsed.subcats[0].shortcut);
        assert_eq!(1, parsed.phrases.len());
        assert_eq!(6, parsed.phrases[0].id);
        assert_eq!(1, parsed.phrases[0].shortcut);
        assert!(parsed.unknown4);
    }

    #[test]
    fn parses_headbar_fields() {
        let bytes = [
            2, 5, 3, 6, 4, 5, 0, 7, 7, 0, 8, 8, 0, 9, 11, 0, 9, 16, 17, 4, 0,
        ];
        let parsed = parse_headbar(7, &bytes).expect("parse headbar");
        assert_eq!(7, parsed.id);
        assert_eq!(Some(5), parsed.showpriority);
        assert_eq!(Some(6), parsed.hidepriority);
        assert!(parsed.fadeout_disabled);
        assert_eq!(Some(7), parsed.sticktime);
        assert_eq!(Some(8), parsed.full);
        assert_eq!(Some(9), parsed.empty);
        assert_eq!(Some(9), parsed.fadeout);
        assert!(parsed.unknown16);
        assert_eq!(Some(4), parsed.unknown17);
    }

    #[test]
    fn parses_hitmark_fields() {
        let bytes = [
            1, 0, 10, 2, 1, 2, 3, 7, 0, 1, 8, 0, b'x', 0, 11, 14, 0, 7, 17, 0, 1, 0, 2, 1, 0, 5, 0,
            6, 19, 0, 30, 20, 0, 15, 0,
        ];
        let parsed = parse_hitmark(8, &bytes).expect("parse hitmark");
        assert_eq!(8, parsed.id);
        assert_eq!(Some(10), parsed.damagefont);
        assert_eq!(Some(0x0001_0203), parsed.damagecolour);
        assert_eq!(Some(1), parsed.scrolltooffsetx);
        assert_eq!(Some("x".to_string()), parsed.damageformat);
        assert!(parsed.fadeout_disabled);
        assert_eq!(Some(7), parsed.fadeout);
        assert_eq!(Some(1), parsed.multivarbit);
        assert_eq!(Some(2), parsed.multivarp);
        assert_eq!(2, parsed.multimarks.len());
        assert!(matches!(
            parsed.multimarks[0],
            HitmarkMulti::Indexed {
                index: 0,
                hitmark_id: 5
            }
        ));
        assert_eq!(Some(30), parsed.damagescaleto);
        assert_eq!(Some(15), parsed.damagescalefrom);
    }

    #[test]
    fn parses_light_fields() {
        let bytes = [
            1, 3, 2, 0, 4, 3, 0, 5, 4, 0, 6, 5, 0, 0, 0, 1, 8, 0x3F, 0x80, 0x00, 0x00, 9, 10, 0, 0,
            0, 2, 12, 0x40, 0x00, 0x00, 0x00, 0,
        ];
        let parsed = parse_light(9, &bytes).expect("parse light");
        assert_eq!(9, parsed.id);
        assert_eq!(Some(3), parsed.function);
        assert_eq!(Some(4), parsed.frequency);
        assert_eq!(Some(5), parsed.amplitude);
        assert_eq!(Some(6), parsed.offset);
        assert_eq!(Some(1), parsed.unknown5);
        assert_eq!(Some(1.0), parsed.swayamount);
        assert!(parsed.swayamountrandom);
        assert_eq!(Some(2), parsed.swayduration);
        assert_eq!(Some(2.0), parsed.swayeasing);
    }

    #[test]
    fn parses_quickchatphrase_fields() {
        let bytes = [1, b't', 0, 2, 1, 0, 9, 3, 2, 0, 4, 0, 7, 0, 15, 4, 0];
        let parsed = parse_quickchatphrase(10, &bytes).expect("parse quickchatphrase");
        assert_eq!(10, parsed.id);
        assert_eq!(Some("t".to_string()), parsed.template);
        assert_eq!(vec![9], parsed.autoresponses);
        assert_eq!(2, parsed.dynamic_commands.len());
        assert!(matches!(
            parsed.dynamic_commands[0],
            QuickChatDynamicCommand::StatBase { stat_id: 7 }
        ));
        assert!(matches!(
            parsed.dynamic_commands[1],
            QuickChatDynamicCommand::CombatLevel
        ));
        assert!(parsed.unknown4_no);
    }

    #[test]
    fn parses_billboard_fields() {
        let bytes = [1, 0, 5, 2, 0, 1, 0, 2, 3, 255, 4, 7, 5, 8, 6, 7, 0];
        let parsed = parse_billboard(11, &bytes).expect("parse billboard");
        assert_eq!(11, parsed.id);
        assert_eq!(Some(5), parsed.material);
        assert_eq!(Some((1, 2)), parsed.unknown2);
        assert_eq!(Some(-1), parsed.unknown3);
        assert_eq!(Some(7), parsed.unknown4);
        assert_eq!(Some(8), parsed.unknown5);
        assert!(parsed.unknown6);
        assert!(parsed.unknown7);
    }

    #[test]
    fn parses_particle_effector_fields() {
        let bytes = [
            1, 0, 9, 2, 3, 3, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 4, 5, 0, 0, 0, 4, 6, 7, 8, 9, 10,
            0,
        ];
        let parsed = parse_particle_effector(12, &bytes).expect("parse particleeffector");
        assert_eq!(12, parsed.id);
        assert_eq!(8, parsed.ops.len());
        assert!(matches!(parsed.ops[0], ParticleEffectorOp::Unknown1(9)));
        assert!(matches!(
            parsed.ops[2],
            ParticleEffectorOp::Unknown3 { x: 1, y: 2, z: 3 }
        ));
    }

    #[test]
    fn parses_particle_emitter_fields() {
        let bytes = [
            1, 0, 1, 0, 2, 0, 3, 0, 4, 9, 2, 0, 5, 0, 6, 29, 0, 0, 7, 35, 1, 0, 8, 0, 9, 10, 0,
        ];
        let parsed = parse_particle_emitter(13, &bytes).expect("parse particleemitter");
        assert_eq!(13, parsed.id);
        assert_eq!(4, parsed.ops.len());
        assert!(matches!(
            parsed.ops[0],
            ParticleEmitterOp::Unknown1 {
                a: 1,
                b: 2,
                c: 3,
                d: 4
            }
        ));
        assert!(matches!(
            parsed.ops[2],
            ParticleEmitterOp::AngularVelocity(7)
        ));
    }

    #[test]
    fn parses_texture_fields() {
        let bytes = [0, 5, 1, 1, 0, 9, 0, 0, 0, 1, 2, 3];
        let parsed = parse_texture(14, &bytes).expect("parse texture");
        assert_eq!(14, parsed.id);
        assert_eq!(5, parsed.averagecolour);
        assert!(parsed.opaque);
        assert_eq!(9, parsed.sprite);
        assert_eq!(1, parsed.unknown1);
        assert_eq!((2, 3), parsed.animation);
    }

    #[test]
    fn parses_stylesheet_fields() {
        let bytes = [0, 7, 0, 1, 2, 0, 0, 0, 3, 0, 0, 0, 4];
        let parsed = parse_stylesheet(15, &bytes).expect("parse stylesheet");
        assert_eq!(15, parsed.id);
        assert_eq!(7, parsed.parent);
        assert_eq!(1, parsed.entries.len());
        assert_eq!(2, parsed.entries[0].unknown);
        assert_eq!(3, parsed.entries[0].key_hash);
        assert_eq!(4, parsed.entries[0].value);
    }
}
