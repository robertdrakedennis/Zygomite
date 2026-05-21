use crate::packet::Packet;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct MapSquare {
    pub environment: Option<Environment>,
    pub lights: Vec<PointLight>,
    pub water: Vec<WaterPatch>,
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

pub fn decode_map_square(files: &BTreeMap<u32, Vec<u8>>, build: u32) -> Result<MapSquare> {
    let environment = files
        .get(&6)
        .map(|data| decode_environment(&mut Packet::new(data), build))
        .transpose()?;

    let lights = if let Some(data) = files.get(&7) {
        decode_lights(&mut Packet::new(data), build)?
    } else {
        Vec::new()
    };

    let water = if let Some(data) = files.get(&8) {
        decode_water(&mut Packet::new(data))?
    } else {
        Vec::new()
    };

    Ok(MapSquare {
        environment,
        lights,
        water,
    })
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

#[cfg(test)]
mod tests {
    use super::decode_map_square;
    use std::collections::BTreeMap;

    #[test]
    fn decodes_empty_mapsquare() {
        let files = BTreeMap::new();
        let decoded = match decode_map_square(&files, 947) {
            Ok(value) => value,
            Err(error) => panic!("mapsquare decode should succeed: {error}"),
        };
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

        let decoded = match decode_map_square(&files, 947) {
            Ok(value) => value,
            Err(error) => panic!("mapsquare decode should succeed: {error}"),
        };
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
