use super::{OpListEntry, gfloat_be, yes_no};
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;

pub fn parse_water(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("water {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!("unknown1={}", packet.g2()?)),
            2 => ops.push(format!("normal_map_material1_scale={}", packet.g2()?)),
            3 => ops.push(format!("unknown3={}", packet.g2()?)),
            4 => ops.push(format!("normal_map_material2_scale={}", packet.g2()?)),
            5 => ops.push(format!("reflection_strength={}", packet.g2()?)),
            6 => ops.push(format!("unknown6=0x{:06x}", packet.g3()?)),
            7 => ops.push(format!("unknown7={},{}", packet.g2()?, packet.g2()?)),
            8 => ops.push(format!("unknown8={}", packet.g2()?)),
            9 => ops.push(format!("water_foam_scale={}", packet.g2()?)),
            10 => ops.push(format!("foam_material_scale={}", packet.g2()?)),
            11 => ops.push(format!("unknown11={}", packet.g2()?)),
            12 => ops.push(format!("basergba=0x{:08x}", packet.g4s()? as u32)),
            13 => ops.push(format!("unknown13={}", packet.g2()?)),
            14 => ops.push(format!("water_depth_foam={}", packet.g2()?)),
            15 => ops.push(format!("unknown15={}", packet.g4s()?)),
            16 => ops.push(format!("unknown16={}", packet.g2()?)),
            17 => ops.push(format!("unknown17={}", packet.g2()?)),
            18 => ops.push(format!("unknown18={}", packet.g1()?)),
            19 => ops.push(format!("unknown19={}", packet.g1()?)),
            20 => ops.push(format!("unknown20={}", packet.g2()?)),
            21 => ops.push(format!("unknown21={}", packet.g2()?)),
            22 => ops.push(format!("unknown22={}", packet.g2()?)),
            23 => ops.push(format!("unknown23={}", packet.g2()?)),
            24 => ops.push(format!("unknown24={}", packet.g1()?)),
            25 => ops.push(format!("specular_shininess={}", packet.g2()?)),
            26 => ops.push(format!(
                "unknown26={},{},{}",
                packet.g2()?,
                packet.g2()?,
                packet.g2()?
            )),
            27 => ops.push(format!("specular_factor={}", packet.g2()?)),
            28 => ops.push(format!("unknown28={}", packet.g2()?)),
            29 => ops.push(format!("normal_map_material1={}", packet.g2()?)),
            30 => ops.push(format!("normal_map_material2={}", packet.g2()?)),
            31 => ops.push(format!("normal_map_material3={}", packet.g2()?)),
            32 => ops.push(format!("normal_map_material3_scale={}", packet.g2()?)),
            opcode @ 33..=80 => decode_normal_map_params(&mut packet, &mut ops, opcode)?,
            81 => ops.push(format!(
                "still_water_normal_strength={}",
                gfloat_be(&mut packet)?
            )),
            82 => ops.push(format!("flow_noise={}", gfloat_be(&mut packet)?)),
            83 => ops.push(format!("fresnel_bias={}", gfloat_be(&mut packet)?)),
            84 => ops.push(format!("unknown84={}", gfloat_be(&mut packet)?)),
            85 => ops.push(format!("override_default_water_type={}", packet.g1()?)),
            86 => ops.push(format!("emisive_map_material={}", packet.g2()?)),
            87 => ops.push(format!("emissive_map_material_scale={}", packet.g2()?)),
            88 => ops.push(format!(
                "emissive_uv_scale={},{}",
                gfloat_be(&mut packet)?,
                gfloat_be(&mut packet)?
            )),
            89 => ops.push(format!("emissive_rgb={}", packet.g4s()?)),
            90 => ops.push(format!("emissive_scale={}", gfloat_be(&mut packet)?)),
            91 => ops.push(format!(
                "emissive_map_refraction_depth={}",
                gfloat_be(&mut packet)?
            )),
            92 => ops.push(format!("emissive_map_mode={}", packet.g1()?)),
            93 => ops.push(format!("emissive_source={}", gfloat_be(&mut packet)?)),
            94 => ops.push(format!("emissive_flow_speed={}", gfloat_be(&mut packet)?)),
            95 => ops.push(format!(
                "emissive_flow_rotation_degrees={}",
                gfloat_be(&mut packet)?
            )),
            96 => ops.push(format!("emissive_uv_mode={}", packet.g1()?)),
            97 => ops.push(format!(
                "extinction_rgb_depth_metres={},{},{}",
                gfloat_be(&mut packet)?,
                gfloat_be(&mut packet)?,
                gfloat_be(&mut packet)?
            )),
            98 => ops.push(format!("extinction_opaque_water_colour={}", packet.g4s()?)),
            99 => ops.push(format!(
                "extinction_visibility_metres={}",
                gfloat_be(&mut packet)?
            )),
            100 => ops.push(format!("caustics_scale={}", gfloat_be(&mut packet)?)),
            101 => ops.push(format!(
                "caustics_refraction_scale={}",
                gfloat_be(&mut packet)?
            )),
            102 => ops.push(format!(
                "caustics_depth_fade_cutoff={}",
                gfloat_be(&mut packet)?
            )),
            103 => ops.push(format!(
                "caustics_depth_fade_scale={}",
                gfloat_be(&mut packet)?
            )),
            104 => ops.push(format!(
                "caustics_edge_fade_start={}",
                gfloat_be(&mut packet)?
            )),
            105 => ops.push(format!(
                "caustics_edge_fade_end={}",
                gfloat_be(&mut packet)?
            )),
            106 => ops.push(format!(
                "caustics_over_water_fade_start={}",
                gfloat_be(&mut packet)?
            )),
            107 => ops.push(format!(
                "caustics_over_water_fade_end={}",
                gfloat_be(&mut packet)?
            )),
            108 => ops.push(format!("emissive_blend={}", gfloat_be(&mut packet)?)),
            opcode => bail!("unknown water opcode {opcode} in {id}"),
        }
    }
}

fn decode_normal_map_params(
    packet: &mut Packet<'_>,
    ops: &mut Vec<String>,
    opcode: u8,
) -> Result<()> {
    let offset = usize::from(opcode - 33);
    let target = (offset / 8) + 1;
    match offset % 8 {
        0 => ops.push(format!(
            "normal_map_params{target}_unknown33={}",
            yes_no(packet.g1()? == 1)
        )),
        1 => ops.push(format!(
            "normal_map_params{target}_unknown34={}",
            gfloat_be(packet)?
        )),
        2 => ops.push(format!(
            "normal_map_params{target}_unknown35={}",
            gfloat_be(packet)?
        )),
        3 => ops.push(format!(
            "normal_map_params{target}_unknown36={}",
            gfloat_be(packet)?
        )),
        4 => ops.push(format!(
            "normal_map_params{target}_unknown37={},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?
        )),
        5 => ops.push(format!(
            "normal_map_params{target}_unknown38={},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?
        )),
        6 => ops.push(format!(
            "normal_map_params{target}_unknown39={}",
            gfloat_be(packet)?
        )),
        7 => ops.push(format!(
            "normal_map_params{target}_unknown40={}",
            gfloat_be(packet)?
        )),
        _ => bail!("invalid normal map opcode {opcode}"),
    }
    Ok(())
}
