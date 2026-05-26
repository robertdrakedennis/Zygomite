use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
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
    #[serde(rename = "terrainFile5")]
    pub terrain_file5: Option<MapFile5Terrain>,
    #[serde(rename = "mapFile5TerrainMetadata")]
    pub map_file5_terrain_metadata: Option<MapFile5Terrain>,
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
    #[serde(rename = "sourceShape")]
    pub source_shape: u8,
    #[serde(rename = "sourceAngle")]
    pub source_angle: u8,
    pub derived: bool,
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
    #[serde(rename = "skyboxId")]
    pub skybox_id: i16,
    #[serde(rename = "reflectionMaterialId")]
    pub reflection_material_id: i16,
    #[serde(rename = "reflectionEnabled")]
    pub reflection_enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvColourGradingSettings {
    pub entries: Vec<EnvColourGradingEntry>,
    #[serde(rename = "packetPairs")]
    pub packet_pairs: Vec<EnvColourGradingEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvColourGradingEntry {
    pub texture: i16,
    pub weighting: f32,
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
    #[serde(rename = "waterTypeId")]
    pub water_type_id: u16,
    #[serde(rename = "tailParameter")]
    pub tail_parameter: u16,
    #[serde(rename = "fineXCenter")]
    pub fine_x_center: i32,
    #[serde(rename = "fineZCenter")]
    pub fine_z_center: i32,
    #[serde(rename = "fineXExtent")]
    pub fine_x_extent: i32,
    #[serde(rename = "fineZExtent")]
    pub fine_z_extent: i32,
    #[serde(rename = "signedScalar")]
    pub signed_scalar: f32,
    pub rotation: Vector4,
    #[serde(rename = "tailTransformX")]
    pub tail_transform_x: f32,
    #[serde(rename = "tailTransformZ")]
    pub tail_transform_z: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapFile5Terrain {
    pub format: &'static str,
    #[serde(rename = "semanticStatus")]
    pub semantic_status: MapFile5SemanticStatus,
    pub summary: MapFile5Summary,
    #[serde(rename = "evidenceNotes")]
    pub evidence_notes: Vec<&'static str>,
    pub warnings: Vec<&'static str>,
    pub header: Vec<u8>,
    pub levels: Vec<MapFile5Level>,
    pub truncated: bool,
    #[serde(rename = "payloadBytes")]
    pub payload_bytes: usize,
    #[serde(rename = "trailingBytes")]
    pub trailing_bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapFile5SemanticStatus {
    pub role: &'static str,
    #[serde(rename = "evidenceLevel")]
    pub evidence_level: &'static str,
    #[serde(rename = "provenScope")]
    pub proven_scope: &'static str,
    #[serde(rename = "runtimeApplication")]
    pub runtime_application: &'static str,
    #[serde(rename = "buildTerrainPath")]
    pub build_terrain_path: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapFile5Summary {
    #[serde(rename = "levelCount")]
    pub level_count: usize,
    #[serde(rename = "tileRecordCount")]
    pub tile_record_count: usize,
    #[serde(rename = "extendedTileRecordCount")]
    pub extended_tile_record_count: usize,
    #[serde(rename = "slot1UniqueIdCount")]
    pub slot1_unique_id_count: usize,
    #[serde(rename = "slot4UniqueIdCount")]
    pub slot4_unique_id_count: usize,
    #[serde(rename = "slot1TileReferenceCount")]
    pub slot1_tile_reference_count: usize,
    #[serde(rename = "slot4TileReferenceCount")]
    pub slot4_tile_reference_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapFile5Level {
    pub level: u8,
    #[serde(rename = "slot1Ids")]
    pub slot1_ids: Vec<i32>,
    #[serde(rename = "slot4Ids")]
    pub slot4_ids: Vec<i32>,
    pub tiles: Vec<MapFile5Tile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapFile5Tile {
    pub x: u8,
    pub z: u8,
    pub flags: u8,
    pub extended: bool,
    #[serde(rename = "slot1Ids")]
    pub slot1_ids: Vec<i32>,
    #[serde(rename = "slot4Ids")]
    pub slot4_ids: Vec<i32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkInstanceStream {
    pub preamble: ChunkInstancePreamble,
    pub chunks: Vec<ChunkInstanceChunk>,
    pub records: Vec<ChunkInstanceRecord>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkInstancePreamble {
    pub first: Vec<u16>,
    pub second: Vec<u16>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkInstanceChunk {
    pub mode: u8,
    #[serde(rename = "chunkX")]
    pub chunk_x: u8,
    #[serde(rename = "chunkZ")]
    pub chunk_z: u8,
    #[serde(rename = "subchunkX")]
    pub subchunk_x: Option<u8>,
    #[serde(rename = "subchunkZ")]
    pub subchunk_z: Option<u8>,
    pub presence: Option<u8>,
    pub mask: Option<Vec<u8>>,
    pub occupancy: Option<Vec<u8>>,
    #[serde(rename = "recordStart")]
    pub record_start: usize,
    #[serde(rename = "recordEnd")]
    pub record_end: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkInstanceRecord {
    pub layer: u8,
    pub x: u16,
    pub z: u16,
    #[serde(rename = "locId")]
    pub loc_id: i32,
    pub info: i8,
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
    let landscape = files
        .get(&3)
        .map(|data| {
            decode_landscape(&mut Packet::new(data), false, build).context("decode map file 3")
        })
        .transpose()?;

    let underwater_landscape = files
        .get(&4)
        .map(|data| {
            decode_landscape(&mut Packet::new(data), true, build).context("decode map file 4")
        })
        .transpose()?;

    let locs = files
        .get(&0)
        .map(|data| decode_locs(&mut Packet::new(data)).context("decode map file 0"))
        .transpose()?
        .unwrap_or_default();

    let underwater_locs = files
        .get(&1)
        .map(|data| decode_locs(&mut Packet::new(data)).context("decode map file 1"))
        .transpose()?
        .unwrap_or_default();

    let map_file5_terrain_metadata = files
        .get(&5)
        .filter(|data| !data.is_empty())
        .map(|data| decode_map_file5_terrain(&mut Packet::new(data)).context("decode map file 5"))
        .transpose()?;
    let terrain_file5 = map_file5_terrain_metadata.clone();

    let environment = files
        .get(&6)
        .map(|data| decode_environment(&mut Packet::new(data), build).context("decode map file 6"))
        .transpose()?;

    let lights = files
        .get(&7)
        .map(|data| decode_lights(&mut Packet::new(data), build).context("decode map file 7"))
        .transpose()?
        .unwrap_or_default();

    let water = files
        .get(&8)
        .map(|data| decode_water(&mut Packet::new(data)).context("decode map file 8"))
        .transpose()?
        .unwrap_or_default();

    Ok(MapSquare {
        landscape,
        underwater_landscape,
        locs,
        underwater_locs,
        terrain_file5,
        map_file5_terrain_metadata,
        environment,
        lights,
        water,
    })
}

pub fn decode_map_square_best_effort(files: &BTreeMap<u32, Vec<u8>>, build: u32) -> MapSquare {
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

    let map_file5_terrain_metadata = files
        .get(&5)
        .filter(|data| !data.is_empty())
        .and_then(|data| try_decode!(decode_map_file5_terrain(&mut Packet::new(data))));
    let terrain_file5 = map_file5_terrain_metadata.clone();

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
        terrain_file5,
        map_file5_terrain_metadata,
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
    if packet.g1()? != b'j' || packet.g1()? != b'a' || packet.g1()? != b'g' || packet.g1()? != b'x'
    {
        packet.set_pos(pos)?;
        return Ok(false);
    }
    if packet.g1()? != 1 {
        bail!("unsupported jagx landscape version");
    }
    Ok(true)
}

fn decode_landscape(
    packet: &mut Packet<'_>,
    underwater: bool,
    _build: u32,
) -> Result<LandscapeData> {
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
                    let height = i32::from(if has_jagx {
                        packet.g2()?
                    } else {
                        u16::from(packet.g1()?)
                    });
                    if underwater {
                        heights[0][x][z] = (height * 8) << 2;
                    } else {
                        let h = if height == 1 { 0 } else { height };
                        if level == 0 {
                            heights[0][x][z] = -(h * 8) << 2;
                        } else {
                            heights[level][x][z] = heights[level - 1][x][z] - ((h * 8) << 2);
                        }
                    }
                } else if underwater {
                    heights[0][x][z] = 0;
                } else if level == 0 {
                    heights[0][x][z] = -(perlin(x as i32 + 932_731, z as i32 + 556_238) * 8) << 2;
                } else {
                    heights[level][x][z] = heights[level - 1][x][z] - 960;
                }
            }
        }
    }

    let non_member_areas =
        if has_jagx && !underwater && packet.pos().saturating_add(8) <= packet.len() {
            let mut areas = vec![0u8; 8];
            for area in &mut areas {
                *area = packet.g1()?;
            }
            Some(areas)
        } else {
            None
        };

    let extras = if has_jagx && packet.pos() < packet.len() {
        let entries = decode_landscape_extras(packet)?;
        if entries.is_empty() {
            None
        } else {
            Some(entries)
        }
    } else {
        None
    };

    if packet.pos() != packet.len() {
        bail!(
            "landscape decode left {} trailing bytes",
            packet.len().saturating_sub(packet.pos())
        );
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
                entry.insert(
                    "values".into(),
                    serde_json::json!([
                        read_f32_be(packet)?,
                        read_f32_be(packet)?,
                        read_f32_be(packet)?
                    ]),
                );
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

fn decode_extra_00(
    packet: &mut Packet<'_>,
    entry: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    entry.insert("name".into(), "unk00".into());
    let flags = packet.g1()?;
    entry.insert("flags".into(), flags.into());
    if (flags & 0x01) != 0 {
        entry.insert("unk01".into(), read_bytes(packet, 4)?.into());
    }
    if (flags & 0x02) != 0 {
        entry.insert("unk02".into(), packet.g2()?.into());
    }
    if (flags & 0x04) != 0 {
        entry.insert("unk04".into(), packet.g2()?.into());
    }
    if (flags & 0x08) != 0 {
        entry.insert("unk08".into(), packet.g2()?.into());
    }
    if (flags & 0x10) != 0 {
        entry.insert(
            "unk10".into(),
            serde_json::json!([packet.g2()?, packet.g2()?, packet.g2()?]),
        );
    }
    if (flags & 0x20) != 0 {
        entry.insert("unk20".into(), read_bytes(packet, 4)?.into());
    }
    if (flags & 0x40) != 0 {
        entry.insert("unk40".into(), packet.g2()?.into());
    }
    if (flags & 0x80) != 0 {
        entry.insert("unk80".into(), packet.g2()?.into());
    }
    Ok(())
}

fn decode_extra_01(
    packet: &mut Packet<'_>,
    entry: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
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
        e.insert(
            "array5".into(),
            serde_json::to_value(arrays).unwrap_or_default(),
        );
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

fn decode_extra_81(
    packet: &mut Packet<'_>,
    entry: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
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

// ── Current map file 5 terrain/config metadata ──

fn decode_map_file5_terrain(packet: &mut Packet<'_>) -> Result<MapFile5Terrain> {
    if packet.len() < 6 {
        bail!("map file 5 too short: {} bytes", packet.len());
    }
    let header = packet.slice(0, 5)?.to_vec();
    packet.set_pos(5)?;
    let payload_start = packet.pos();
    let mut truncated = false;
    let mut levels = Vec::new();

    while !packet.is_done() {
        let level = packet.g1()?;
        let mut slot1_ids = Vec::new();
        let mut slot4_ids = Vec::new();
        let mut tiles = Vec::new();
        let mut ended_at_eof = false;

        'tiles: for x in 0..66_u8 {
            for z in 0..66_u8 {
                if packet.is_done() {
                    ended_at_eof = true;
                    break 'tiles;
                }
                let tile_base = packet.pos();
                let flags = packet.g1()?;
                if flags == 0 {
                    continue;
                }

                let extended = (flags & 0x10) != 0;
                let first_smart_pos = tile_base + if extended { 5 } else { 3 };
                if first_smart_pos >= packet.len() {
                    truncated = true;
                    break 'tiles;
                }
                if packet.set_pos(first_smart_pos).is_err() {
                    truncated = true;
                    break 'tiles;
                }

                let mut tile_slot1_ids = Vec::new();
                let mut tile_slot4_ids = Vec::new();

                let Ok(first_id) = read_file5_id(packet) else {
                    truncated = true;
                    break 'tiles;
                };
                push_file5_id(first_id, &mut slot1_ids, &mut tile_slot1_ids);
                if first_id != -1 && skip_file5_bytes(packet, 2).is_err() {
                    truncated = true;
                    break 'tiles;
                }

                let Ok(second_id) = read_file5_id(packet) else {
                    truncated = true;
                    break 'tiles;
                };
                if extended {
                    let Ok(extra_slot4_id) = read_file5_id(packet) else {
                        truncated = true;
                        break 'tiles;
                    };
                    push_file5_id(extra_slot4_id, &mut slot4_ids, &mut tile_slot4_ids);
                }
                push_file5_id(second_id, &mut slot4_ids, &mut tile_slot4_ids);
                if second_id != -1 {
                    if skip_file5_bytes(packet, 1).is_err() {
                        truncated = true;
                        break 'tiles;
                    }
                    if extended {
                        let Ok(extra_slot1_id) = read_file5_id(packet) else {
                            truncated = true;
                            break 'tiles;
                        };
                        push_file5_id(extra_slot1_id, &mut slot1_ids, &mut tile_slot1_ids);
                    }
                }

                tiles.push(MapFile5Tile {
                    x,
                    z,
                    flags,
                    extended,
                    slot1_ids: tile_slot1_ids,
                    slot4_ids: tile_slot4_ids,
                });
            }
        }

        levels.push(MapFile5Level {
            level,
            slot1_ids,
            slot4_ids,
            tiles,
        });

        if truncated || ended_at_eof {
            break;
        }
    }
    let trailing_bytes = packet.len().saturating_sub(packet.pos());
    let summary = summarize_map_file5(&levels);

    Ok(MapFile5Terrain {
        format: "current947TerrainMetadata",
        semantic_status: map_file5_semantic_status(),
        summary,
        evidence_notes: map_file5_evidence_notes(),
        warnings: map_file5_warnings(),
        header,
        levels,
        truncated,
        payload_bytes: packet.len().saturating_sub(payload_start),
        trailing_bytes,
    })
}

fn summarize_map_file5(levels: &[MapFile5Level]) -> MapFile5Summary {
    MapFile5Summary {
        level_count: levels.len(),
        tile_record_count: levels.iter().map(|level| level.tiles.len()).sum(),
        extended_tile_record_count: levels
            .iter()
            .flat_map(|level| &level.tiles)
            .filter(|tile| tile.extended)
            .count(),
        slot1_unique_id_count: levels.iter().map(|level| level.slot1_ids.len()).sum(),
        slot4_unique_id_count: levels.iter().map(|level| level.slot4_ids.len()).sum(),
        slot1_tile_reference_count: levels
            .iter()
            .flat_map(|level| &level.tiles)
            .map(|tile| tile.slot1_ids.len())
            .sum(),
        slot4_tile_reference_count: levels
            .iter()
            .flat_map(|level| &level.tiles)
            .map(|tile| tile.slot4_ids.len())
            .sum(),
    }
}

fn map_file5_semantic_status() -> MapFile5SemanticStatus {
    MapFile5SemanticStatus {
        role: "direct maps archive file 5 terrain/config-id metadata",
        evidence_level: "current Linux 947 producer-side decode and config-resolution evidence",
        proven_scope: "packet decode, per-level slot 1/slot 4 id collection, producer-side post-resolution record setup",
        runtime_application: "not proven by current 947 evidence",
        build_terrain_path: "separate current path through LoadFiles slot 12, terrain resource loader, and terrain payload decoder",
    }
}

fn map_file5_evidence_notes() -> Vec<&'static str> {
    vec![
        "docs/nxt/maps/terrain-and-auxiliary-data.md: direct file 5 parses 66x66 flag/smart records into per-level slot 1 and slot 4 id vectors",
        "docs/nxt/maps/archive-files-and-loaders.md: direct maps archive file 5 is separate from LoadFiles auxiliary slot 12",
        "docs/nxt/maps/implementation-checklist.md: keep file 5 separate from BuildTerrain descriptor path",
    ]
}

fn map_file5_warnings() -> Vec<&'static str> {
    vec![
        "decoded metadata only; runtime application is not proven",
        "do not treat these records as final collision, render terrain, or loc placement semantics",
        "BuildTerrain runtime terrain uses config slot 12/resource loader/payload decoder evidence, not this direct file 5 output",
    ]
}

fn read_file5_id(packet: &mut Packet<'_>) -> Result<i32> {
    Ok(i32::from(packet.gsmart1or2()?) - 1)
}

fn skip_file5_bytes(packet: &mut Packet<'_>, count: usize) -> Result<()> {
    packet.set_pos(packet.pos().saturating_add(count))
}

fn push_file5_id(id: i32, level_ids: &mut Vec<i32>, tile_ids: &mut Vec<i32>) {
    if id == -1 {
        return;
    }
    if let Err(index) = level_ids.binary_search(&id) {
        level_ids.insert(index, id);
    }
    tile_ids.push(id);
}

// ── Locs decoder ──

fn decode_locs(packet: &mut Packet<'_>) -> Result<Vec<MapLoc>> {
    let mut entries = Vec::new();
    let mut id: i32 = -1;
    let mut id_offset = packet.g_extended_1or2()?;

    while id_offset != 0 {
        id += id_offset;
        let mut coord: i32 = 0;
        let mut packed_offset = i32::from(packet.gsmart1or2()?);

        while packed_offset != 0 {
            coord += packed_offset - 1;
            let z = (coord & 0x3F) as u8;
            let x = ((coord >> 6) & 0x3F) as u8;
            let level = (coord >> 12) as u8;
            let info = packet.g1()?;
            let transform = if (info & 0x80) != 0 {
                Some(decode_loc_transform(packet)?)
            } else {
                None
            };
            let shape = (info >> 2) & 0x1F;
            let angle = info & 0x3;
            entries.push(MapLoc {
                id,
                x,
                z,
                level,
                shape,
                angle,
                source_shape: shape,
                source_angle: angle,
                derived: false,
                transform: transform.clone(),
            });
            if shape == 2 || shape == 8 {
                entries.push(MapLoc {
                    id,
                    x,
                    z,
                    level,
                    shape: if shape == 2 { 0x17 } else { 0x18 },
                    angle: if shape == 2 {
                        (angle + 1) & 0x3
                    } else {
                        (angle + 2) & 0x3
                    },
                    source_shape: shape,
                    source_angle: angle,
                    derived: true,
                    transform,
                });
            }
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
        rotation[0] = f32::from(packet.g2s()?) / 32768.0;
        rotation[1] = f32::from(packet.g2s()?) / 32768.0;
        rotation[2] = f32::from(packet.g2s()?) / 32768.0;
        rotation[3] = f32::from(packet.g2s()?) / 32768.0;
    }
    if (flags & 0x2) != 0 {
        translation[0] = i32::from(packet.g2s()?);
    }
    if (flags & 0x4) != 0 {
        translation[1] = i32::from(packet.g2s()?);
    }
    if (flags & 0x8) != 0 {
        translation[2] = i32::from(packet.g2s()?);
    }
    if (flags & 0x10) != 0 {
        let s = f32::from(packet.g2s()?) / 128.0;
        scale = [s, s, s];
    } else {
        if (flags & 0x20) != 0 {
            scale[0] = f32::from(packet.g2s()?) / 128.0;
        }
        if (flags & 0x40) != 0 {
            scale[1] = f32::from(packet.g2s()?) / 128.0;
        }
        if (flags & 0x80) != 0 {
            scale[2] = f32::from(packet.g2s()?) / 128.0;
        }
    }

    Ok(LocTransform {
        rotation,
        translation,
        scale,
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

    let skybox_id = packet.g2s()?;
    let reflection_material_id = packet.g2s()?;
    let skybox = EnvSkySettings {
        skybox_id,
        reflection_material_id,
        reflection_enabled: reflection_material_id != -1,
    };

    let first_colour_pair = EnvColourGradingEntry {
        texture: packet.g2s()?,
        weighting: read_f32_be(packet)?,
    };
    let second_colour_pair = EnvColourGradingEntry {
        texture: packet.g2s()?,
        weighting: read_f32_be(packet)?,
    };
    let colour_remap = EnvColourGradingSettings {
        entries: vec![
            second_colour_pair.clone(),
            EnvColourGradingEntry {
                texture: -1,
                weighting: 0.0,
            },
            EnvColourGradingEntry {
                texture: -1,
                weighting: 0.0,
            },
        ],
        packet_pairs: vec![first_colour_pair, second_colour_pair],
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
        let tail_parameter = packet.g2()?;
        let water_type_id = packet.g2()?;
        let first_center = read_water_center(packet)?;
        let signed_scalar = f32::from(packet.g2s()?);
        let fine_center = [first_center, read_water_center(packet)?];
        let fine_extent = [
            read_water_fine_component(packet)?,
            read_water_fine_component(packet)?,
        ];
        let rotation = read_vec4(packet)?;
        let tail_transform_x = f32::from(packet.g1s()?) * 0.02;
        let tail_transform_z = f32::from(packet.g1s()?) * 0.02;
        water.push(WaterPatch {
            water_type_id,
            tail_parameter,
            fine_x_center: fine_center[0],
            fine_z_center: fine_center[1],
            fine_x_extent: fine_extent[0],
            fine_z_extent: fine_extent[1],
            signed_scalar,
            rotation,
            tail_transform_x,
            tail_transform_z,
        });
    }
    Ok(water)
}

fn read_water_center(packet: &mut Packet<'_>) -> Result<i32> {
    Ok(i32::from(packet.g1s()?) * 512 + 256)
}

fn read_water_fine_component(packet: &mut Packet<'_>) -> Result<i32> {
    Ok(i32::from(packet.g1s()?) << 9)
}

pub fn decode_chunk_instance_stream(data: &[u8]) -> Result<ChunkInstanceStream> {
    let mut packet = Packet::new(data);
    let preamble = ChunkInstancePreamble {
        first: decode_chunk_preamble_list(&mut packet)?,
        second: decode_chunk_preamble_list(&mut packet)?,
    };
    let mut chunks = Vec::new();
    let mut records = Vec::new();

    while !packet.is_done() {
        let mode = packet.g1()?;
        let record_start = records.len();
        let chunk_x = packet.g1()?;
        let chunk_z = packet.g1()?;
        if mode == 0 {
            let mask = packet.gdata(8)?;
            let occupancy = expand_chunk_occupancy(&mask);
            let base_x = u16::from(chunk_x) * 64;
            let base_z = u16::from(chunk_z) * 64;
            for x in 0..64_u16 {
                for z in 0..64_u16 {
                    decode_load_instances(&mut packet, base_x + x, base_z + z, &mut records)?;
                }
            }
            chunks.push(ChunkInstanceChunk {
                mode,
                chunk_x,
                chunk_z,
                subchunk_x: None,
                subchunk_z: None,
                presence: None,
                mask: Some(mask),
                occupancy: Some(occupancy),
                record_start,
                record_end: records.len(),
            });
        } else {
            let subchunk_x = packet.g1()?;
            let subchunk_z = packet.g1()?;
            let presence = packet.g1()?;
            let base_x = u16::from(chunk_x) * 64 + u16::from(subchunk_x) * 8;
            let base_z = u16::from(chunk_z) * 64 + u16::from(subchunk_z) * 8;
            for x in 0..8_u16 {
                for z in 0..8_u16 {
                    decode_load_instances(&mut packet, base_x + x, base_z + z, &mut records)?;
                }
            }
            chunks.push(ChunkInstanceChunk {
                mode,
                chunk_x,
                chunk_z,
                subchunk_x: Some(subchunk_x),
                subchunk_z: Some(subchunk_z),
                presence: Some(presence),
                mask: None,
                occupancy: Some(vec![u8::from(presence != 0)]),
                record_start,
                record_end: records.len(),
            });
        }
    }

    Ok(ChunkInstanceStream {
        preamble,
        chunks,
        records,
    })
}

fn decode_chunk_preamble_list(packet: &mut Packet<'_>) -> Result<Vec<u16>> {
    let count = usize::from(packet.g1()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(packet.gsmart1or2()?);
    }
    Ok(values)
}

fn expand_chunk_occupancy(mask: &[u8]) -> Vec<u8> {
    let mut occupancy = Vec::with_capacity(64);
    for &column in mask.iter().take(8) {
        for z in 0..8 {
            occupancy.push((column >> z) & 0x01);
        }
    }
    occupancy
}

fn decode_load_instances(
    packet: &mut Packet<'_>,
    x: u16,
    z: u16,
    records: &mut Vec<ChunkInstanceRecord>,
) -> Result<()> {
    let flag = packet.g1()?;
    if (flag & 0x01) == 0 {
        match flag >> 2 {
            0x3e => {}
            0x3f => {
                let _ = packet.gsmart1or2()?;
            }
            _ if (flag & 0x02) != 0 => {
                let _ = packet.gsmart1or2s()?;
            }
            _ => {}
        }
        return Ok(());
    }

    let outer_count = usize::from((flag >> 1) & 0x03) + 1;
    for layer in 0..outer_count {
        consume_chunk_slot(packet)?;
        if (flag & 0x08) != 0 {
            let _ = packet.gsmart1or2s()?;
            let _ = packet.g1()?;
        }
        if (flag & 0x10) != 0 {
            let count = usize::from(packet.g1()?);
            for _ in 0..count {
                records.push(ChunkInstanceRecord {
                    layer: layer as u8,
                    x,
                    z,
                    loc_id: packet.gsmart2or4()?,
                    info: packet.g1s()?,
                });
            }
        }
    }

    Ok(())
}

fn consume_chunk_slot(packet: &mut Packet<'_>) -> Result<()> {
    if (packet.peek1()? & 0x80) != 0 {
        let _ = packet.g2()?;
    } else {
        let _ = packet.g1()?;
    }
    Ok(())
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
    let mut value = perlin_scale(x + 45_365, z + 91_923, 4) - 128;
    value += (perlin_scale(x + 10_294, z + 37_821, 2) - 128) >> 1;
    value += (perlin_scale(x, z, 1) - 128) >> 2;
    let result = (f64::from(value) * 0.3) as i32 + 35;
    result.clamp(10, 60)
}

fn perlin_scale(x: i32, z: i32, scale: i32) -> i32 {
    let sx = x / scale;
    let fx = x & (scale - 1);
    let sz = z / scale;
    let fz = z & (scale - 1);
    let northwest = smooth_noise(sx, sz);
    let northeast = smooth_noise(sx + 1, sz);
    let southwest = smooth_noise(sx, sz + 1);
    let southeast = smooth_noise(sx + 1, sz + 1);
    let xb = interpolate(northwest, northeast, fx, scale);
    let zb = interpolate(southwest, southeast, fx, scale);
    interpolate(xb, zb, fz, scale)
}

fn interpolate(a: i32, b: i32, frac: i32, scale: i32) -> i32 {
    let theta = f64::from(frac) * std::f64::consts::PI / f64::from(scale);
    let cosine = (theta.cos() * 16384.0) as i64;
    let weight = (65536 - cosine) >> 1;
    let a = i64::from(a);
    let b = i64::from(b);
    ((((65536 - weight) * a) >> 16) + ((b * weight) >> 16)) as i32
}

fn smooth_noise(x: i32, z: i32) -> i32 {
    let corners =
        noise(x - 1, z - 1) + noise(x + 1, z - 1) + noise(x - 1, z + 1) + noise(x + 1, z + 1);
    let sides = noise(x - 1, z) + noise(x + 1, z) + noise(x, z - 1) + noise(x, z + 1);
    let center = noise(x, z);
    center / 4 + corners / 16 + sides / 8
}

fn noise(x: i32, z: i32) -> i32 {
    let mixed = z.wrapping_mul(57).wrapping_add(x);
    let hashed = (mixed << 13) ^ mixed;
    let hashed = i64::from(hashed);
    let value = hashed
        .wrapping_mul(hashed)
        .wrapping_mul(15_731)
        .wrapping_add(789_221)
        .wrapping_mul(hashed)
        .wrapping_add(1_376_312_589);
    ((value & i64::from(i32::MAX)) >> 19) as i32 & 0xFF
}

#[cfg(test)]
mod tests {
    use super::{decode_chunk_instance_stream, decode_map_square};
    use std::collections::BTreeMap;
    use std::iter::repeat_n;

    #[test]
    fn decodes_empty_mapsquare() {
        let files = BTreeMap::new();
        let decoded = decode_map_square(&files, 947).expect("empty mapsquare");
        assert!(decoded.environment.is_none());
        assert!(decoded.lights.is_empty());
        assert!(decoded.water.is_empty());
    }

    #[test]
    fn decodes_raw_loc_variant_shapes() {
        let mut files = BTreeMap::new();
        files.insert(0, vec![1, 1, (2 << 2) | 1, 0, 0]);

        let decoded = decode_map_square(&files, 947).expect("loc variants");

        assert_eq!(2, decoded.locs.len());
        assert_eq!(2, decoded.locs[0].shape);
        assert_eq!(1, decoded.locs[0].angle);
        assert!(!decoded.locs[0].derived);
        assert_eq!(0x17, decoded.locs[1].shape);
        assert_eq!(2, decoded.locs[1].angle);
        assert_eq!(2, decoded.locs[1].source_shape);
        assert_eq!(1, decoded.locs[1].source_angle);
        assert!(decoded.locs[1].derived);
    }

    #[test]
    fn decodes_map_file5_metadata_header() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 3];
        payload.extend(repeat_n(0, 66 * 66));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 metadata");
        let terrain = decoded.terrain_file5.expect("terrain file 5");
        let metadata = decoded
            .map_file5_terrain_metadata
            .expect("map file 5 terrain metadata");

        assert_eq!("current947TerrainMetadata", terrain.format);
        assert_eq!("current947TerrainMetadata", metadata.format);
        assert_eq!(
            "direct maps archive file 5 terrain/config-id metadata",
            terrain.semantic_status.role
        );
        assert_eq!(
            "not proven by current 947 evidence",
            terrain.semantic_status.runtime_application
        );
        assert_eq!(1, terrain.summary.level_count);
        assert_eq!(0, terrain.summary.tile_record_count);
        assert_eq!(0, terrain.summary.extended_tile_record_count);
        assert_eq!(0, terrain.summary.slot1_unique_id_count);
        assert_eq!(0, terrain.summary.slot4_unique_id_count);
        assert_eq!(0, terrain.summary.slot1_tile_reference_count);
        assert_eq!(0, terrain.summary.slot4_tile_reference_count);
        assert!(!terrain.evidence_notes.is_empty());
        assert!(
            terrain
                .warnings
                .iter()
                .any(|warning| warning.contains("decoded metadata only"))
        );
        assert_eq!(vec![b'j', b'a', b'g', b'x', 1], terrain.header);
        assert_eq!(1, terrain.levels.len());
        assert_eq!(3, terrain.levels[0].level);
        assert!(terrain.levels[0].tiles.is_empty());
        assert_eq!(66 * 66 + 1, terrain.payload_bytes);
    }

    #[test]
    fn decodes_map_file5_multiple_levels_until_eof() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 0];
        payload.extend(repeat_n(0, 66 * 66));
        payload.push(2);
        payload.extend(repeat_n(0, 66 * 66));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 multiple levels");
        let terrain = decoded.terrain_file5.expect("terrain file 5");

        assert_eq!(2, terrain.levels.len());
        assert_eq!(0, terrain.levels[0].level);
        assert_eq!(2, terrain.levels[1].level);
        assert!(!terrain.truncated);
        assert_eq!(0, terrain.trailing_bytes);
        assert_eq!((66 * 66 + 1) * 2, terrain.payload_bytes);
    }

    #[test]
    fn decodes_map_file5_partial_zero_suffix_until_eof() {
        let mut files = BTreeMap::new();
        let payload = vec![b'j', b'a', b'g', b'x', 1, 0, 1, 0xaa, 0xbb, 0, 0];
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 partial zero suffix");
        let terrain = decoded.terrain_file5.expect("terrain file 5");

        assert_eq!(1, terrain.levels.len());
        assert_eq!(1, terrain.levels[0].tiles.len());
        assert!(!terrain.truncated);
        assert_eq!(0, terrain.trailing_bytes);
    }

    #[test]
    fn decodes_map_file5_current_slot_ids() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 0];
        push_u8(&mut payload, 1);
        push_u8(&mut payload, 0xaa);
        push_u8(&mut payload, 0xbb);
        push_u8(&mut payload, 11);
        push_u8(&mut payload, 0xcc);
        push_u8(&mut payload, 0xdd);
        push_u8(&mut payload, 21);
        push_u8(&mut payload, 0xee);
        payload.extend(repeat_n(0, 66 * 66 - 1));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 ids");
        let terrain = decoded.terrain_file5.expect("terrain file 5");
        let level = &terrain.levels[0];

        assert_eq!(vec![10], level.slot1_ids);
        assert_eq!(vec![20], level.slot4_ids);
        assert_eq!(1, terrain.summary.tile_record_count);
        assert_eq!(1, terrain.summary.slot1_unique_id_count);
        assert_eq!(1, terrain.summary.slot4_unique_id_count);
        assert_eq!(1, terrain.summary.slot1_tile_reference_count);
        assert_eq!(1, terrain.summary.slot4_tile_reference_count);
        assert_eq!(1, level.tiles.len());
        assert_eq!(0, level.tiles[0].x);
        assert_eq!(0, level.tiles[0].z);
        assert_eq!(vec![10], level.tiles[0].slot1_ids);
        assert_eq!(vec![20], level.tiles[0].slot4_ids);
    }

    #[test]
    fn decodes_map_file5_extended_slot_ids() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 0];
        push_u8(&mut payload, 0x10);
        push_u8(&mut payload, 0xaa);
        push_u8(&mut payload, 0xbb);
        push_u8(&mut payload, 0xcc);
        push_u8(&mut payload, 0xdd);
        push_u8(&mut payload, 11);
        push_u8(&mut payload, 0xee);
        push_u8(&mut payload, 0xff);
        push_u8(&mut payload, 21);
        push_u8(&mut payload, 31);
        push_u8(&mut payload, 0x99);
        push_u8(&mut payload, 41);
        payload.extend(repeat_n(0, 66 * 66 - 1));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 extended ids");
        let terrain = decoded.terrain_file5.expect("terrain file 5");
        let level = &terrain.levels[0];

        assert_eq!(vec![10, 40], level.slot1_ids);
        assert_eq!(vec![20, 30], level.slot4_ids);
        assert_eq!(1, terrain.summary.extended_tile_record_count);
        assert_eq!(2, terrain.summary.slot1_unique_id_count);
        assert_eq!(2, terrain.summary.slot4_unique_id_count);
        assert_eq!(2, terrain.summary.slot1_tile_reference_count);
        assert_eq!(2, terrain.summary.slot4_tile_reference_count);
        assert_eq!(vec![10, 40], level.tiles[0].slot1_ids);
        assert_eq!(vec![30, 20], level.tiles[0].slot4_ids);
    }

    #[test]
    fn decodes_current_environment_sky_reflection_and_colour_grading() {
        let mut files = BTreeMap::new();
        let mut payload = Vec::new();

        push_i32_be(&mut payload, -1);
        push_zero_u16s(&mut payload, 6);
        push_zero_f32s(&mut payload, 3);
        push_i32_be(&mut payload, 0);
        push_u16_be(&mut payload, 0);
        push_u8(&mut payload, 1);
        push_zero_f32s(&mut payload, 4);
        push_u8(&mut payload, 1);
        push_zero_f32s(&mut payload, 4);
        push_zero_f32s(&mut payload, 9);
        push_zero_f32s(&mut payload, 8);
        push_i32_be(&mut payload, 0);
        push_i32_be(&mut payload, 0);
        push_f32_be(&mut payload, 0.0);
        push_u8(&mut payload, 1);
        push_u8(&mut payload, 3);
        push_zero_f32s(&mut payload, 5);
        push_zero_f32s(&mut payload, 6);
        push_i16_be(&mut payload, -1);
        push_i16_be(&mut payload, 77);
        push_i16_be(&mut payload, 11);
        push_f32_be(&mut payload, 0.25);
        push_i16_be(&mut payload, 22);
        push_f32_be(&mut payload, 0.75);
        push_f32_be(&mut payload, 0.5);
        push_f32_be(&mut payload, 1.0);
        push_f32_be(&mut payload, 2.0);
        push_f32_be(&mut payload, 3.0);
        files.insert(6, payload);

        let decoded = decode_map_square(&files, 947).expect("environment");
        let env = decoded.environment.expect("environment data");

        assert_eq!(-1, env.skybox.skybox_id);
        assert_eq!(77, env.skybox.reflection_material_id);
        assert!(env.skybox.reflection_enabled);
        assert_eq!(3, env.colour_remap.entries.len());
        assert_eq!(22, env.colour_remap.entries[0].texture);
        assert_f32_eq(0.75, env.colour_remap.entries[0].weighting);
        assert_eq!(-1, env.colour_remap.entries[1].texture);
        assert_eq!(2, env.colour_remap.packet_pairs.len());
        assert_eq!(11, env.colour_remap.packet_pairs[0].texture);
        assert_eq!(22, env.colour_remap.packet_pairs[1].texture);
        assert_f32_eq(0.5, env.light_probe.unknown1);
        assert_f32_eq(3.0, env.light_probe.unknown2.z);
    }

    #[test]
    fn decodes_water_patch() {
        let mut files = BTreeMap::new();
        let mut payload = Vec::new();
        push_u8(&mut payload, 1);
        push_u16_be(&mut payload, 5);
        push_u16_be(&mut payload, 9);
        push_u8(&mut payload, 1);
        push_i16_be(&mut payload, 128);
        push_u8(&mut payload, 0xff);
        push_u8(&mut payload, 3);
        push_u8(&mut payload, 0xfc);
        push_f32_be(&mut payload, 1.0);
        push_f32_be(&mut payload, 2.0);
        push_f32_be(&mut payload, 3.0);
        push_f32_be(&mut payload, 4.0);
        push_u8(&mut payload, 7);
        push_u8(&mut payload, 0xf8);
        files.insert(8, payload);

        let decoded = decode_map_square(&files, 947).expect("water patch");
        assert_eq!(1, decoded.water.len());
        assert_eq!(9, decoded.water[0].water_type_id);
        assert_eq!(5, decoded.water[0].tail_parameter);
        assert_eq!(768, decoded.water[0].fine_x_center);
        assert_eq!(-256, decoded.water[0].fine_z_center);
        assert_eq!(1536, decoded.water[0].fine_x_extent);
        assert_eq!(-2048, decoded.water[0].fine_z_extent);
        assert_f32_eq(128.0, decoded.water[0].signed_scalar);
        assert_f32_eq(1.0, decoded.water[0].rotation.x);
        assert!((0.14 - decoded.water[0].tail_transform_x).abs() < f32::EPSILON);
        assert!((-0.16 - decoded.water[0].tail_transform_z).abs() < f32::EPSILON);
    }

    #[test]
    fn decodes_chunk_instance_stream_records() {
        let mut payload = vec![0, 0, 1, 2, 3, 4, 5, 0xaa];
        push_u8(&mut payload, 0x11);
        push_u8(&mut payload, 0);
        push_u8(&mut payload, 1);
        push_u16_be(&mut payload, 42);
        push_u8(&mut payload, 0xfb);
        payload.extend(repeat_n(0xf8, 63));

        let decoded = decode_chunk_instance_stream(&payload).expect("chunk instance stream");

        assert_eq!(1, decoded.chunks.len());
        assert_eq!(1, decoded.records.len());
        assert_eq!(160, decoded.records[0].x);
        assert_eq!(232, decoded.records[0].z);
        assert_eq!(42, decoded.records[0].loc_id);
        assert_eq!(-5, decoded.records[0].info);
    }

    fn push_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    fn push_u16_be(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i16_be(out: &mut Vec<u8>, value: i16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i32_be(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_f32_be(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_bits().to_be_bytes());
    }

    fn assert_f32_eq(expected: f32, actual: f32) {
        assert!(
            (expected - actual).abs() < f32::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn push_zero_u16s(out: &mut Vec<u8>, count: usize) {
        for _ in 0..count {
            push_u16_be(out, 0);
        }
    }

    fn push_zero_f32s(out: &mut Vec<u8>, count: usize) {
        for _ in 0..count {
            push_f32_be(out, 0.0);
        }
    }
}
