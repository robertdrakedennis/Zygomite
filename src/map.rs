use crate::packet::Packet;
use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct MapSquare {
    pub landscape: Option<LandscapeData>,
    #[serde(rename = "underwaterLandscape")]
    pub underwater_landscape: Option<LandscapeData>,
    pub locs: Vec<MapLoc>,
    #[serde(rename = "underwaterLocs")]
    pub underwater_locs: Vec<MapLoc>,
    pub environment: Option<Environment>,
    pub lights: Vec<PointLight>,
    pub water: Vec<WaterPatch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LandscapeData {
    #[serde(rename = "sceneFlags")]
    pub scene_flags: Vec<Vec<Vec<i8>>>,
    pub heights: Vec<Vec<Vec<i32>>>,
    #[serde(rename = "underlayIds")]
    pub underlay_ids: Vec<Vec<Vec<u16>>>,
    #[serde(rename = "overlayIds")]
    pub overlay_ids: Vec<Vec<Vec<u16>>>,
    #[serde(rename = "overlayShapes")]
    pub overlay_shapes: Vec<Vec<Vec<u8>>>,
    #[serde(rename = "overlayRotations")]
    pub overlay_rotations: Vec<Vec<Vec<u8>>>,
    #[serde(rename = "nonMemberAreas")]
    pub non_member_areas: Option<Vec<u8>>,
    pub extras: Option<Vec<serde_json::Value>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapLoc {
    pub id: i32,
    pub x: u8,
    pub z: u8,
    pub level: u8,
    pub shape: u8,
    pub angle: u8,
    pub transform: Option<LocTransform>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocTransform {
    pub rotation: [f32; 4],
    pub translation: [i32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct Environment {
    pub lighting: EnvLightingSettings,
    pub fog: EnvFogSettings,
    pub scattering: EnvScatteringSettings,
    pub volumetrics: EnvVolumetricSettings,
    pub unknown: f32,
    #[serde(rename = "toneMap")]
    pub tone_map: EnvToneMapSettings,
    pub unknown6: EnvBloomSettings,
    pub skybox: EnvSkySettings,
    #[serde(rename = "colourRemap")]
    pub colour_remap: EnvColourGradingSettings,
    #[serde(rename = "lightProbe")]
    pub light_probe: EnvLightProbeSettings,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvLightingSettings {
    pub colour: i32,
    #[serde(rename = "sunX")]
    pub sun_x: u16,
    #[serde(rename = "sunY")]
    pub sun_y: u16,
    #[serde(rename = "sunZ")]
    pub sun_z: u16,
    #[serde(rename = "ambientIntensity")]
    pub ambient_intensity: u16,
    #[serde(rename = "sunlightIntensity")]
    pub sunlight_intensity: u16,
    pub unknown7: u16,
    pub unknown8: f32,
    pub unknown9: f32,
    #[serde(rename = "shadowStrength")]
    pub shadow_strength: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvFogSettings {
    pub colour: i32,
    pub depth: u16,
    pub enabled: bool,
    #[serde(rename = "maxFogFarPlane")]
    pub max_fog_far_plane: f32,
    #[serde(rename = "depthAngleFalloff")]
    pub depth_angle_falloff: f32,
    #[serde(rename = "depthAngleOffset")]
    pub depth_angle_offset: f32,
    #[serde(rename = "angleOffset")]
    pub angle_offset: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvScatteringSettings {
    pub enabled: bool,
    pub density: f32,
    #[serde(rename = "startDistance")]
    pub start_distance: f32,
    pub unknown4: f32,
    pub unknown5: f32,
    #[serde(rename = "scatteringTint")]
    pub scattering_tint: Vector3,
    #[serde(rename = "outscatteringAmount")]
    pub outscattering_amount: Vector3,
    #[serde(rename = "inscatteringAmount")]
    pub inscattering_amount: Vector3,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvVolumetricSettings {
    pub density: f32,
    pub inscattering: f32,
    pub outscattering: f32,
    #[serde(rename = "skyboxDensityMultiplier")]
    pub skybox_density_multiplier: f32,
    pub exaggeration: f32,
    pub g: f32,
    #[serde(rename = "skyG")]
    pub sky_g: f32,
    #[serde(rename = "bilateralBlurDepth")]
    pub bilateral_blur_depth: f32,
    #[serde(rename = "litFogColour")]
    pub lit_fog_colour: i32,
    #[serde(rename = "unlitFogColour")]
    pub unlit_fog_colour: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvToneMapSettings {
    #[serde(rename = "toneMapEnabled")]
    pub tone_map_enabled: bool,
    #[serde(rename = "toneMapOperator")]
    pub tone_map_operator: u8,
    #[serde(rename = "minBlackLum")]
    pub min_black_lum: f32,
    #[serde(rename = "maxWhiteLum")]
    pub max_white_lum: f32,
    #[serde(rename = "exposureKey")]
    pub exposure_key: f32,
    #[serde(rename = "minAutoExposure")]
    pub min_auto_exposure: f32,
    #[serde(rename = "maxAutoExposure")]
    pub max_auto_exposure: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvBloomSettings {
    pub unknown1: f32,
    pub unknown2: f32,
    pub unknown3: f32,
    pub unknown4: f32,
    pub unknown5: f32,
    pub unknown6: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvSkySettings {
    pub unknown1: u16,
    pub unknown2: u16,
    pub unknown3: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvColourGradingSettings {
    pub tex1: i16,
    pub weighting1: f32,
    pub tex2: i16,
    pub weighting2: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvLightProbeSettings {
    pub unknown1: f32,
    pub unknown2: Vector3,
}

#[derive(Clone, Debug, Serialize)]
// RS3 light flags are inherently many independent booleans (scattering, shadows, etc.)
#[allow(clippy::struct_excessive_bools)]
pub struct PointLight {
    pub level: u8,
    pub x: u16,
    pub y: u16,
    pub z: u16,
    pub enabled: bool,
    #[serde(rename = "extendAbove")]
    pub extend_above: bool,
    #[serde(rename = "extendBelow")]
    pub extend_below: bool,
    pub radius: u8,
    pub mask: Vec<u16>,
    #[serde(rename = "type")]
    pub type_name: String,
    pub phase: u8,
    pub colour: u16,
    pub id: String,
    pub intensity: f32,
    #[serde(rename = "attenuationFalloff")]
    pub attenuation_falloff: f32,
    pub something: f32,
    pub shadow: bool,
    #[serde(rename = "shadowFactor")]
    pub shadow_factor: f32,
    pub unknown1: f32,
    pub unknown2: f32,
    pub unknown3: f32,
    pub unknown4: f32,
    pub unknown5: u16,
    pub unknown6: u16,
    pub unknown7: u16,
    pub unknown8: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct WaterPatch {
    pub x: u8,
    pub z: u8,
    pub width: u8,
    pub length: u8,
    pub unknown1: u16,
    pub unknown2: Vector4,
    pub unknown4: u16,
    pub unknown5: u8,
    pub unknown6: u8,
    #[serde(rename = "type")]
    pub type_id: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

pub fn decode_map_square(files: &BTreeMap<u32, Vec<u8>>, build: u32) -> MapSquare {
    macro_rules! try_decode {
        ($decoder:expr) => {
            match $decoder {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("map decode warning: {e}");
                    None
                }
            }
        };
    }

    let landscape = files
        .get(&3)
        .and_then(|data| try_decode!(decode_landscape(&mut Packet::new(data), false, build)));

    let underwater_landscape = files
        .get(&4)
        .and_then(|data| try_decode!(decode_landscape(&mut Packet::new(data), true, build)));

    let locs = files
        .get(&0)
        .and_then(|data| try_decode!(decode_locs(&mut Packet::new(data))))
        .unwrap_or_default();

    let underwater_locs = files
        .get(&1)
        .and_then(|data| try_decode!(decode_locs(&mut Packet::new(data))))
        .unwrap_or_default();

    let environment = files
        .get(&6)
        .and_then(|data| try_decode!(decode_environment(&mut Packet::new(data), build)));

    let lights = files
        .get(&7)
        .and_then(|data| try_decode!(decode_lights(&mut Packet::new(data), build)))
        .unwrap_or_default();

    let water = files
        .get(&8)
        .and_then(|data| try_decode!(decode_water(&mut Packet::new(data))))
        .unwrap_or_default();

    MapSquare {
        landscape,
        underwater_landscape,
        locs,
        underwater_locs,
        environment,
        lights,
        water,
    }
}

// ── Landscape decoder ──

fn has_jagx_header(packet: &mut Packet<'_>) -> Result<bool> {
    if packet.len() < 5 {
        return Ok(false);
    }
    let pos = packet.pos();
    if packet.g1()? != b'j'
        || packet.g1()? != b'a'
        || packet.g1()? != b'g'
        || packet.g1()? != b'x'
    {
        packet.set_pos(pos)?;
        return Ok(false);
    }
    if packet.g1()? != 1 {
        bail!("unsupported jagx landscape version");
    }
    Ok(true)
}

fn decode_landscape(packet: &mut Packet<'_>, underwater: bool, _build: u32) -> Result<LandscapeData> {
    let has_jagx = has_jagx_header(packet)?;
    let level_count: usize = if underwater { 1 } else { 4 };

    let mut scene_flags = vec![vec![vec![0i8; 64]; 64]; level_count];
    let mut heights = vec![vec![vec![0i32; 64]; 64]; level_count];
    let mut underlay_ids = vec![vec![vec![0u16; 64]; 64]; level_count];
    let mut overlay_ids = vec![vec![vec![0u16; 64]; 64]; level_count];
    let mut overlay_shapes = vec![vec![vec![0u8; 64]; 64]; level_count];
    let mut overlay_rotations = vec![vec![vec![0u8; 64]; 64]; level_count];

    for level in 0..level_count {
        for x in 0..64 {
            for z in 0..64 {
                let opcode = packet.g1()?;
                if (opcode & 0xF0) != 0 {
                    bail!("unsupported landscape tile opcode 0x{opcode:02x}");
                }
                if (opcode & 0x1) != 0 {
                    let overlay_info = packet.g1()?;
                    overlay_ids[level][x][z] = packet.gsmart1or2()?;
                    overlay_shapes[level][x][z] = overlay_info >> 2;
                    overlay_rotations[level][x][z] = overlay_info & 0x3;
                }
                if (opcode & 0x2) != 0 {
                    scene_flags[level][x][z] = packet.g1s()?;
                }
                if (opcode & 0x4) != 0 {
                    underlay_ids[level][x][z] = packet.gsmart1or2()?;
                }
                if (opcode & 0x8) != 0 {
                    let height = i32::from(if has_jagx { packet.g2()? } else { u16::from(packet.g1()?) });
                    if underwater {
                        heights[0][x][z] = height * 8 << 2;
                    } else {
                        let h = if height == 1 { 0 } else { height };
                        if level == 0 {
                            heights[0][x][z] = -(h * 8) << 2;
                        } else {
                            heights[level][x][z] = heights[level - 1][x][z] - (h * 8 << 2);
                        }
                    }
                } else if underwater {
                    heights[0][x][z] = 0;
                } else if level == 0 {
                    heights[0][x][z] = -(perlin(x as i32 + 932731, z as i32 + 556238) * 8) << 2;
                } else {
                    heights[level][x][z] = heights[level - 1][x][z] - 960;
                }
            }
        }
    }

    let non_member_areas = if has_jagx && !underwater && packet.pos().saturating_add(8) <= packet.len() {
        let mut areas = vec![0u8; 8];
        for a in areas.iter_mut() {
            *a = packet.g1()?;
        }
        Some(areas)
    } else {
        None
    };

    // Try extras only if there are 2+ bytes remaining (minimum for opcode + data)
    let extras = if has_jagx && packet.pos().saturating_add(2) <= packet.len() {
        match decode_landscape_extras(packet) {
            Ok(e) if !e.is_empty() => Some(e),
            _ => None,
        }
    } else {
        None
    };

    // Skip remaining unconsumed bytes matching Java's strict check
    if packet.pos() != packet.len() && packet.pos().saturating_add(2) <= packet.len() {
        // Log but continue — partial decode is better than no decode
    }

    Ok(LandscapeData {
        scene_flags,
        heights,
        underlay_ids,
        overlay_ids,
        overlay_shapes,
        overlay_rotations,
        non_member_areas,
        extras,
    })
}

fn decode_landscape_extras(packet: &mut Packet<'_>) -> Result<Vec<serde_json::Value>> {
    let mut extras = Vec::new();
    while packet.pos() < packet.len() {
        let opcode = packet.g1()?;
        let mut entry = serde_json::Map::new();
        entry.insert("opcode".into(), opcode.into());
        match opcode {
            0x00 => decode_extra_00(packet, &mut entry)?,
            0x01 => decode_extra_01(packet, &mut entry)?,
            0x02 => {
                entry.insert("name".into(), "unk02".into());
                entry.insert("values".into(), serde_json::json!([
                    read_f32_be(packet)?,
                    read_f32_be(packet)?,
                    read_f32_be(packet)?
                ]));
            }
            0x03 => {
                entry.insert("name".into(), "unk03".into());
                entry.insert("short".into(), packet.g2s()?.into());
                entry.insert("float".into(), read_f32_be(packet)?.into());
            }
            0x80 => {
                entry.insert("name".into(), "unk80".into());
                entry.insert("environment".into(), packet.g2()?.into());
                entry.insert("always00".into(), read_bytes(packet, 8)?.into());
            }
            0x81 => decode_extra_81(packet, &mut entry)?,
            0x82 => {
                entry.insert("name".into(), "unk82".into());
            }
            _ => bail!("unknown landscape extra opcode 0x{opcode:02x}"),
        }
        extras.push(serde_json::Value::Object(entry));
    }
    Ok(extras)
}

fn decode_extra_00(packet: &mut Packet<'_>, entry: &mut serde_json::Map<String, serde_json::Value>) -> Result<()> {
    entry.insert("name".into(), "unk00".into());
    let flags = packet.g1()?;
    entry.insert("flags".into(), flags.into());
    if (flags & 0x01) != 0 { entry.insert("unk01".into(), read_bytes(packet, 4)?.into()); }
    if (flags & 0x02) != 0 { entry.insert("unk02".into(), packet.g2()?.into()); }
    if (flags & 0x04) != 0 { entry.insert("unk04".into(), packet.g2()?.into()); }
    if (flags & 0x08) != 0 { entry.insert("unk08".into(), packet.g2()?.into()); }
    if (flags & 0x10) != 0 {
        entry.insert("unk10".into(), serde_json::json!([
            packet.g2()?, packet.g2()?, packet.g2()?
        ]));
    }
    if (flags & 0x20) != 0 { entry.insert("unk20".into(), read_bytes(packet, 4)?.into()); }
    if (flags & 0x40) != 0 { entry.insert("unk40".into(), packet.g2()?.into()); }
    if (flags & 0x80) != 0 { entry.insert("unk80".into(), packet.g2()?.into()); }
    Ok(())
}

fn decode_extra_01(packet: &mut Packet<'_>, entry: &mut serde_json::Map<String, serde_json::Value>) -> Result<()> {
    entry.insert("name".into(), "unk01".into());
    let count = usize::from(packet.g1()?);
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let mut e = serde_json::Map::new();
        e.insert("byte2".into(), packet.g1()?.into());
        e.insert("short0".into(), packet.g2()?.into());
        e.insert("short1".into(), packet.g2()?.into());
        e.insert("short2".into(), packet.g2()?.into());
        let array_count = usize::from(packet.g1()?);
        let mut arrays: Vec<Vec<u8>> = Vec::with_capacity(array_count);
        for _ in 0..array_count {
            arrays.push(read_bytes(packet, 4)?);
        }
        e.insert("array5".into(), serde_json::to_value(arrays).unwrap_or_default());
        e.insert("short3".into(), packet.g2()?.into());
        e.insert("short4".into(), packet.g2()?.into());
        let extra_flags = packet.g1()?;
        e.insert("extraflags".into(), extra_flags.into());
        e.insert("extra08".into(), packet.g2()?.into());
        if (extra_flags & 0x1F) == 0x1F {
            e.insert("extra1f".into(), packet.g2()?.into());
        }
        entries.push(serde_json::Value::Object(e));
    }
    entry.insert("entries".into(), entries.into());
    Ok(())
}

fn decode_extra_81(packet: &mut Packet<'_>, entry: &mut serde_json::Map<String, serde_json::Value>) -> Result<()> {
    entry.insert("name".into(), "unk81".into());
    let mut entries = Vec::with_capacity(4);
    for _ in 0..4 {
        let mut e = serde_json::Map::new();
        let flag = packet.g1()?;
        e.insert("flag".into(), flag.into());
        if (flag & 0x01) != 0 {
            e.insert("data".into(), read_bytes(packet, 256)?.into());
        }
        entries.push(serde_json::Value::Object(e));
    }
    entry.insert("entries".into(), entries.into());
    Ok(())
}

fn read_bytes(packet: &mut Packet<'_>, count: usize) -> Result<Vec<u8>> {
    packet.gdata(count)
}

// ── Locs decoder ──

fn decode_locs(packet: &mut Packet<'_>) -> Result<Vec<MapLoc>> {
    let mut entries = Vec::new();
    let mut id: i32 = -1;
    let mut id_offset = packet.g_extended_1or2()?;

    while id_offset != 0 {
        id += id_offset;
        let mut packed: i32 = 0;
        let mut packed_offset = i32::from(packet.gsmart1or2()?);

        while packed_offset != 0 {
            packed += packed_offset - 1;
            let z = (packed & 0x3F) as u8;
            let x = ((packed >> 6) & 0x3F) as u8;
            let level = (packed >> 12) as u8;
            let info = packet.g1()?;
            let transform = if (info & 0x80) != 0 {
                Some(decode_loc_transform(packet)?)
            } else {
                None
            };
            let shape = (info >> 2) & 0x1F;
            let angle = info & 0x3;
            entries.push(MapLoc { id, x, z, level, shape, angle, transform });
            packed_offset = i32::from(packet.gsmart1or2()?);
        }
        id_offset = packet.g_extended_1or2()?;
    }

    Ok(entries)
}

fn decode_loc_transform(packet: &mut Packet<'_>) -> Result<LocTransform> {
    let flags = packet.g1()?;
    let mut rotation = [0.0f32, 0.0, 0.0, 1.0];
    let mut translation = [0i32; 3];
    let mut scale = [1.0f32; 3];

    if (flags & 0x1) != 0 {
        rotation[0] = packet.g2s()? as f32 / 32768.0;
        rotation[1] = packet.g2s()? as f32 / 32768.0;
        rotation[2] = packet.g2s()? as f32 / 32768.0;
        rotation[3] = packet.g2s()? as f32 / 32768.0;
    }
    if (flags & 0x2) != 0 { translation[0] = i32::from(packet.g2s()?); }
    if (flags & 0x4) != 0 { translation[1] = i32::from(packet.g2s()?); }
    if (flags & 0x8) != 0 { translation[2] = i32::from(packet.g2s()?); }
    if (flags & 0x10) != 0 {
        let s = packet.g2s()? as f32 / 128.0;
        scale = [s, s, s];
    } else {
        if (flags & 0x20) != 0 { scale[0] = packet.g2s()? as f32 / 128.0; }
        if (flags & 0x40) != 0 { scale[1] = packet.g2s()? as f32 / 128.0; }
        if (flags & 0x80) != 0 { scale[2] = packet.g2s()? as f32 / 128.0; }
    }

    Ok(LocTransform { rotation, translation, scale })
}

fn decode_environment(packet: &mut Packet<'_>, build: u32) -> Result<Environment> {
    let lighting = EnvLightingSettings {
        colour: packet.g4s()?,
        sun_x: packet.g2()?,
        sun_y: packet.g2()?,
        sun_z: packet.g2()?,
        ambient_intensity: packet.g2()?,
        sunlight_intensity: packet.g2()?,
        unknown7: packet.g2()?,
        unknown8: read_f32_be(packet)?,
        unknown9: read_f32_be(packet)?,
        shadow_strength: read_f32_be(packet)?,
    };

    let fog = EnvFogSettings {
        colour: packet.g4s()?,
        depth: packet.g2()?,
        enabled: packet.g1()? == 1,
        max_fog_far_plane: read_f32_be(packet)?,
        depth_angle_falloff: read_f32_be(packet)?,
        depth_angle_offset: read_f32_be(packet)?,
        angle_offset: read_f32_be(packet)?,
    };

    let scattering = EnvScatteringSettings {
        enabled: packet.g1()? == 1,
        density: read_f32_be(packet)?,
        start_distance: read_f32_be(packet)?,
        unknown4: read_f32_be(packet)?,
        unknown5: read_f32_be(packet)?,
        scattering_tint: read_vec3(packet)?,
        outscattering_amount: read_vec3(packet)?,
        inscattering_amount: read_vec3(packet)?,
    };

    let volumetrics = EnvVolumetricSettings {
        density: read_f32_be(packet)?,
        inscattering: read_f32_be(packet)?,
        outscattering: read_f32_be(packet)?,
        skybox_density_multiplier: read_f32_be(packet)?,
        exaggeration: read_f32_be(packet)?,
        g: read_f32_be(packet)?,
        sky_g: read_f32_be(packet)?,
        bilateral_blur_depth: read_f32_be(packet)?,
        lit_fog_colour: packet.g4s()?,
        unlit_fog_colour: packet.g4s()?,
    };

    let unknown = if build >= 942 {
        read_f32_be(packet)?
    } else {
        (1.0_f32 - 512.0_f32) * 10.0_f32
    };

    let tone_map = EnvToneMapSettings {
        tone_map_enabled: packet.g1()? == 1,
        tone_map_operator: packet.g1()?,
        min_black_lum: read_f32_be(packet)?,
        max_white_lum: read_f32_be(packet)?,
        exposure_key: read_f32_be(packet)?,
        min_auto_exposure: read_f32_be(packet)?,
        max_auto_exposure: read_f32_be(packet)?,
    };

    let unknown6 = EnvBloomSettings {
        unknown1: read_f32_be(packet)?,
        unknown2: read_f32_be(packet)?,
        unknown3: read_f32_be(packet)?,
        unknown4: read_f32_be(packet)?,
        unknown5: read_f32_be(packet)?,
        unknown6: read_f32_be(packet)?,
    };

    let skybox = EnvSkySettings {
        unknown1: packet.g2()?,
        unknown2: packet.g2()?,
        unknown3: packet.g1()? == 1,
    };

    let colour_remap = EnvColourGradingSettings {
        tex1: packet.g2s()?,
        weighting1: read_f32_be(packet)?,
        tex2: packet.g2s()?,
        weighting2: read_f32_be(packet)?,
    };

    let light_probe = EnvLightProbeSettings {
        unknown1: read_f32_be(packet)?,
        unknown2: read_vec3(packet)?,
    };

    Ok(Environment {
        lighting,
        fog,
        scattering,
        volumetrics,
        unknown,
        tone_map,
        unknown6,
        skybox,
        colour_remap,
        light_probe,
    })
}

fn decode_lights(packet: &mut Packet<'_>, build: u32) -> Result<Vec<PointLight>> {
    let count = usize::from(packet.g1()?);
    let mut lights = Vec::with_capacity(count);
    for _ in 0..count {
        lights.push(decode_point_light(packet, build)?);
    }
    Ok(lights)
}

fn decode_point_light(packet: &mut Packet<'_>, build: u32) -> Result<PointLight> {
    let mut level = packet.g1()?;
    let extend_above = (level & 8) != 0;
    let extend_below = (level & 16) != 0;
    level &= 7;

    let x = packet.g2()?;
    let z = packet.g2()?;
    let y = packet.g2()?;
    let radius = packet.g1()?;

    let mut mask = Vec::with_capacity(usize::from(radius) * 2 + 1);
    for _ in 0..=(usize::from(radius) * 2) {
        mask.push(packet.g2()?);
    }

    let colour = packet.g2()?;
    let light_data = packet.g1()?;
    let light_type = light_data & 0b1_1111;
    let phase = light_data >> 5;

    let type_name = if light_type == 31 {
        format_reference("light", packet.g2null()?)
    } else {
        format!("builtin_{light_type}")
    };
    let id = format_reference("pointlight", packet.g2null()?);
    let intensity = read_f32_be(packet)?;
    let attenuation_falloff = read_f32_be(packet)?;
    let something = read_f32_be(packet)?;
    let shadow = packet.g1()? == 1;
    let shadow_factor = read_f32_be(packet)?;
    let enabled = packet.g1()? == 1;

    let (unknown1, unknown2, unknown3, unknown4, unknown5, unknown6, unknown7, unknown8) =
        if build >= 942 {
            (
                read_f32_be(packet)?,
                read_f32_be(packet)?,
                read_f32_be(packet)?,
                read_f32_be(packet)?,
                packet.g2()?,
                packet.g2()?,
                packet.g2()?,
                packet.g1()?,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0, 0, 0, 0)
        };

    Ok(PointLight {
        level,
        x,
        y,
        z,
        enabled,
        extend_above,
        extend_below,
        radius,
        mask,
        type_name,
        phase,
        colour,
        id,
        intensity,
        attenuation_falloff,
        something,
        shadow,
        shadow_factor,
        unknown1,
        unknown2,
        unknown3,
        unknown4,
        unknown5,
        unknown6,
        unknown7,
        unknown8,
    })
}

fn decode_water(packet: &mut Packet<'_>) -> Result<Vec<WaterPatch>> {
    let count = usize::from(packet.g1()?);
    let mut water = Vec::with_capacity(count);
    for _ in 0..count {
        water.push(WaterPatch {
            x: packet.g1()?,
            z: packet.g1()?,
            width: packet.g1()?,
            length: packet.g1()?,
            unknown1: packet.g2()?,
            unknown2: read_vec4(packet)?,
            unknown4: packet.g2()?,
            unknown5: packet.g1()?,
            unknown6: packet.g1()?,
            type_id: packet.g2()?,
        });
    }
    Ok(water)
}

fn format_reference(prefix: &str, value: i32) -> String {
    if value == -1 {
        String::from("null")
    } else {
        format!("{prefix}_{value}")
    }
}

fn read_vec3(packet: &mut Packet<'_>) -> Result<Vector3> {
    Ok(Vector3 {
        x: read_f32_be(packet)?,
        y: read_f32_be(packet)?,
        z: read_f32_be(packet)?,
    })
}

fn read_vec4(packet: &mut Packet<'_>) -> Result<Vector4> {
    Ok(Vector4 {
        x: read_f32_be(packet)?,
        y: read_f32_be(packet)?,
        z: read_f32_be(packet)?,
        w: read_f32_be(packet)?,
    })
}

fn read_f32_be(packet: &mut Packet<'_>) -> Result<f32> {
    Ok(f32::from_bits(packet.g4s()? as u32))
}

// ── Perlin noise (for default landscape heights) ──

fn perlin(x: i32, z: i32) -> i32 {
    let mut value = perlin_scale(x + 45365, z + 91923, 4) - 128;
    value += (perlin_scale(x + 10294, z + 37821, 2) - 128) >> 1;
    value += (perlin_scale(x, z, 1) - 128) >> 2;
    let result = (value as f64 * 0.3) as i32 + 35;
    result.clamp(10, 60)
}

fn perlin_scale(x: i32, z: i32, scale: i32) -> i32 {
    let sx = x / scale;
    let fx = x & (scale - 1);
    let sz = z / scale;
    let fz = z & (scale - 1);
    let a = smooth_noise(sx, sz);
    let b = smooth_noise(sx + 1, sz);
    let c = smooth_noise(sx, sz + 1);
    let d = smooth_noise(sx + 1, sz + 1);
    let xb = interpolate(a, b, fx, scale);
    let zb = interpolate(c, d, fx, scale);
    interpolate(xb, zb, fz, scale)
}

fn interpolate(a: i32, b: i32, frac: i32, scale: i32) -> i32 {
    let theta = frac as f64 * std::f64::consts::PI / scale as f64;
    let cosine = (theta.cos() * 16384.0) as i64;
    let weight = (65536 - cosine) >> 1;
    let a = a as i64;
    let b = b as i64;
    (((65536 - weight) * a >> 16) + (b * weight >> 16)) as i32
}

fn smooth_noise(x: i32, z: i32) -> i32 {
    let corners = noise(x - 1, z - 1) + noise(x + 1, z - 1) + noise(x - 1, z + 1) + noise(x + 1, z + 1);
    let sides = noise(x - 1, z) + noise(x + 1, z) + noise(x, z - 1) + noise(x, z + 1);
    let center = noise(x, z);
    center / 4 + corners / 16 + sides / 8
}

fn noise(x: i32, z: i32) -> i32 {
    let mixed = z.wrapping_mul(57).wrapping_add(x);
    let hashed = (mixed << 13) ^ mixed;
    let value = (hashed as i64 * hashed as i64 * 15731 + 789221) * hashed as i64 + 1376312589;
    ((value & i64::from(i32::MAX)) >> 19) as i32 & 0xFF
}

#[cfg(test)]
mod tests {
    use super::decode_map_square;
    use std::collections::BTreeMap;

    #[test]
    fn decodes_empty_mapsquare() {
        let files = BTreeMap::new();
        let decoded = decode_map_square(&files, 947);
        assert!(decoded.environment.is_none());
        assert!(decoded.lights.is_empty());
        assert!(decoded.water.is_empty());
    }

    #[test]
    fn decodes_water_patch() {
        let mut files = BTreeMap::new();
        let mut payload = Vec::new();
        push_u8(&mut payload, 1);
        push_u8(&mut payload, 1);
        push_u8(&mut payload, 2);
        push_u8(&mut payload, 3);
        push_u8(&mut payload, 4);
        push_u16_be(&mut payload, 5);
        push_f32_be(&mut payload, 1.0);
        push_f32_be(&mut payload, 2.0);
        push_f32_be(&mut payload, 3.0);
        push_f32_be(&mut payload, 4.0);
        push_u16_be(&mut payload, 6);
        push_u8(&mut payload, 7);
        push_u8(&mut payload, 8);
        push_u16_be(&mut payload, 9);
        files.insert(8, payload);

        let decoded = decode_map_square(&files, 947);
        assert_eq!(1, decoded.water.len());
        assert_eq!(1, decoded.water[0].x);
        assert_eq!(9, decoded.water[0].type_id);
    }

    fn push_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    fn push_u16_be(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_f32_be(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_bits().to_be_bytes());
    }
}
