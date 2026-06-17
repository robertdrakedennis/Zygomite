use serde::Serialize;

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
