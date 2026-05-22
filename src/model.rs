use crate::packet::Packet;
use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Model {
    pub format: u8,
    pub version: u8,
    pub always_0f: u8,
    pub mesh_count: u8,
    pub unk_count0: u8,
    pub unk_count1: u8,
    pub unk_count2: u8,
    pub unk_count3: u8,
    pub unk_count4: u8,
    pub meshes: Option<Vec<Mesh>>,
    pub meshdata: Option<MeshData>,
    pub unk1_buffer: Vec<String>,
    pub unk2_buffer: Vec<String>,
    pub unk3_buffer: Vec<String>,
    pub unk4_buffer: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
// RS3 model mesh flags are inherently many independent rendering booleans.
#[allow(clippy::struct_excessive_bools)]
pub struct Mesh {
    pub group_flags: u8,
    pub unkint: u32,
    pub material_argument: u16,
    pub face_count: u16,
    pub has_vertices: bool,
    pub has_vertex_alpha: bool,
    pub has_face_bones: bool,
    pub has_bone_ids: bool,
    pub is_hidden: bool,
    pub has_skin: bool,
    pub colour_buffer: Option<Vec<u16>>,
    pub alpha_buffer: Option<Vec<u8>>,
    pub facebone_id_buffer: Option<Vec<u16>>,
    pub index_buffers: Vec<Vec<u16>>,
    pub vertex_count: u16,
    pub position_buffer: Option<Vec<Vec<i16>>>,
    pub normal_buffer: Option<Vec<Vec<i16>>>,
    pub tangent_buffer: Option<Vec<Vec<i16>>>,
    pub uv_buffer: Option<UvBuffer>,
    pub boneid_buffer: Option<Vec<u16>>,
    pub skin: Option<LegacySkin>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum UvBuffer {
    Shorts(Vec<Vec<u16>>),
    Floats(Vec<Vec<f32>>),
}

#[derive(Clone, Debug, Serialize)]
// RS3 model mesh data flags include face/material booleans.
#[allow(clippy::struct_excessive_bools)]
pub struct MeshData {
    pub group_flags: u8,
    pub unkint: u8,
    pub face_count: u16,
    pub has_vertices: bool,
    pub has_vertex_alpha: bool,
    pub has_face_bones: bool,
    pub has_bone_ids: bool,
    pub is_hidden: bool,
    pub has_skin: bool,
    pub vertex_count: u32,
    pub position_buffer: Option<Vec<Vec<i16>>>,
    pub normal_buffer: Option<Vec<Vec<i8>>>,
    pub tangent_buffer: Option<Vec<Vec<i16>>>,
    pub uv_buffer: Option<Vec<Vec<u16>>>,
    pub boneid_buffer: Option<Vec<u16>>,
    pub skin: Option<Vec<SkinWeight>>,
    pub vertex_colours: Option<Vec<u16>>,
    pub vertex_alpha: Option<Vec<u8>>,
    pub vertex_facebones: Option<Vec<u16>>,
    pub renders: Vec<Render>,
}

#[derive(Clone, Debug, Serialize)]
// RS3 model render flags include face/lighting/color booleans.
#[allow(clippy::struct_excessive_bools)]
pub struct Render {
    pub group_flags: u8,
    pub has_vertices: bool,
    pub has_vertex_alpha: bool,
    pub has_face_bones: bool,
    pub has_bone_ids: bool,
    pub is_hidden: bool,
    pub has_skin: bool,
    pub unkint: u32,
    pub material_argument: u16,
    pub unkbyte2: u8,
    pub indices: RenderIndices,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum RenderIndices {
    Small(Vec<u16>),
    Large(Vec<u32>),
}

#[derive(Clone, Debug, Serialize)]
pub struct LegacySkin {
    pub skin_weight_count: u32,
    pub skin_bone_buffer: Vec<u16>,
    pub skin_weight_buffer: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkinWeight {
    pub ids: Vec<u16>,
    pub weights: Vec<u8>,
}

impl Model {
    pub fn decode(data: &[u8], build: u32) -> Result<Self> {
        let mut packet = Packet::new(data);
        let format = packet.g1()?;
        let version = packet.g1()?;
        let always_0f = packet.g1()?;
        let mesh_count = packet.g1()?;
        let unk_count0 = packet.g1()?;
        let unk_count1 = packet.g1()?;
        let unk_count2 = packet.g1()?;
        let unk_count3 = packet.g1()?;
        let unk_count4 = if version >= 5 { packet.g1()? } else { 0 };

        let (meshes, meshdata) = if version <= 3 {
            let mut meshes = Vec::with_capacity(usize::from(mesh_count));
            for _ in 0..usize::from(mesh_count) {
                meshes.push(Mesh::decode(&mut packet, build)?);
            }
            (Some(meshes), None)
        } else {
            (None, Some(MeshData::decode(&mut packet, mesh_count)?))
        };

        let unk1_len = if build >= 923 { 39 } else { 37 };
        let unk2_len = if build >= 923 { 50 } else { 44 };
        let unk3_len = if build >= 923 { 18 } else { 16 };

        let unk1_buffer = read_hex_buffers(&mut packet, unk_count1, unk1_len)?;
        let unk2_buffer = read_hex_buffers(&mut packet, unk_count2, unk2_len)?;
        let unk3_buffer = read_hex_buffers(&mut packet, unk_count3, unk3_len)?;
        let unk4_buffer = read_hex_buffers(&mut packet, unk_count4, 0)?;

        if !packet.is_done() {
            bail!("model decoder did not consume full payload");
        }

        Ok(Self {
            format,
            version,
            always_0f,
            mesh_count,
            unk_count0,
            unk_count1,
            unk_count2,
            unk_count3,
            unk_count4,
            meshes,
            meshdata,
            unk1_buffer,
            unk2_buffer,
            unk3_buffer,
            unk4_buffer,
        })
    }
}

impl Mesh {
    fn decode(packet: &mut Packet<'_>, build: u32) -> Result<Self> {
        let group_flags = packet.g1()?;
        let unkint = read_uint_be(packet)?;
        let material_argument = packet.g2_le()?;
        let face_count = packet.g2_le()?;
        let has_vertices = bit(group_flags, 0);
        let has_vertex_alpha = bit(group_flags, 1);
        let has_face_bones = bit(group_flags, 2);
        let has_bone_ids = bit(group_flags, 3);
        let is_hidden = bit(group_flags, 4);
        let has_skin = bit(group_flags, 5);

        let colour_buffer = if has_vertices {
            Some(read_u16_buffer(packet, usize::from(face_count))?)
        } else {
            None
        };
        let alpha_buffer = if has_vertex_alpha {
            Some(read_u8_buffer(packet, usize::from(face_count))?)
        } else {
            None
        };
        let facebone_id_buffer = if has_face_bones {
            Some(read_u16_buffer(packet, usize::from(face_count))?)
        } else {
            None
        };

        let index_buffer_count = usize::from(packet.g1()?);
        let mut index_buffers = Vec::with_capacity(index_buffer_count);
        for _ in 0..index_buffer_count {
            let len = usize::from(packet.g2_le()?);
            index_buffers.push(read_u16_buffer(packet, len)?);
        }

        let vertex_count = if has_vertices { packet.g2_le()? } else { 0 };
        let position_buffer = if has_vertices {
            Some(read_i16_matrix(packet, usize::from(vertex_count), 3)?)
        } else {
            None
        };
        let normal_buffer = if has_vertices {
            if build >= 887 {
                let values = read_i8_matrix(packet, usize::from(vertex_count), 3)?;
                Some(
                    values
                        .into_iter()
                        .map(|row| row.into_iter().map(i16::from).collect())
                        .collect(),
                )
            } else {
                Some(read_i16_matrix(packet, usize::from(vertex_count), 3)?)
            }
        } else {
            None
        };
        let tangent_buffer = if has_vertices && build >= 906 {
            Some(read_i16_matrix(packet, usize::from(vertex_count), 2)?)
        } else {
            None
        };
        let uv_buffer = if has_vertices {
            if build >= 887 {
                Some(UvBuffer::Shorts(read_u16_matrix(
                    packet,
                    usize::from(vertex_count),
                    2,
                )?))
            } else {
                Some(UvBuffer::Floats(read_f32_matrix(
                    packet,
                    usize::from(vertex_count),
                    2,
                )?))
            }
        } else {
            None
        };
        let boneid_buffer = if has_bone_ids {
            Some(read_u16_buffer(packet, usize::from(vertex_count))?)
        } else {
            None
        };
        let skin = if has_skin {
            Some(LegacySkin::decode(packet)?)
        } else {
            None
        };

        Ok(Self {
            group_flags,
            unkint,
            material_argument,
            face_count,
            has_vertices,
            has_vertex_alpha,
            has_face_bones,
            has_bone_ids,
            is_hidden,
            has_skin,
            colour_buffer,
            alpha_buffer,
            facebone_id_buffer,
            index_buffers,
            vertex_count,
            position_buffer,
            normal_buffer,
            tangent_buffer,
            uv_buffer,
            boneid_buffer,
            skin,
        })
    }
}

impl MeshData {
    fn decode(packet: &mut Packet<'_>, mesh_count: u8) -> Result<Self> {
        let group_flags = packet.g1()?;
        let unkint = packet.g1()?;
        let face_count = packet.g2_le()?;
        let has_vertices = bit(group_flags, 0);
        let has_vertex_alpha = bit(group_flags, 1);
        let has_face_bones = bit(group_flags, 2);
        let has_bone_ids = bit(group_flags, 3);
        let is_hidden = bit(group_flags, 4);
        let has_skin = bit(group_flags, 5);

        let vertex_count = read_uint_le(packet)?;
        let vertex_count_usize = usize::try_from(vertex_count).context("vertex count too large")?;

        let position_buffer = if has_vertices {
            Some(read_i16_matrix(packet, vertex_count_usize, 3)?)
        } else {
            None
        };
        let normal_buffer = if has_vertices {
            Some(read_i8_matrix(packet, vertex_count_usize, 3)?)
        } else {
            None
        };
        let tangent_buffer = if has_vertices {
            Some(read_i16_matrix(packet, vertex_count_usize, 2)?)
        } else {
            None
        };
        let uv_buffer = if has_vertices {
            Some(read_u16_matrix(packet, vertex_count_usize, 2)?)
        } else {
            None
        };
        let boneid_buffer = if has_bone_ids {
            Some(read_u16_buffer(packet, vertex_count_usize)?)
        } else {
            None
        };
        let skin = if has_skin {
            let mut skin = Vec::with_capacity(vertex_count_usize);
            for _ in 0..vertex_count_usize {
                skin.push(SkinWeight::decode(packet)?);
            }
            Some(skin)
        } else {
            None
        };
        let vertex_colours = if has_vertices {
            Some(read_u16_buffer(packet, vertex_count_usize)?)
        } else {
            None
        };
        let vertex_alpha = if has_vertices {
            Some(read_u8_buffer(packet, vertex_count_usize)?)
        } else {
            None
        };
        let vertex_facebones = if has_face_bones {
            Some(read_u16_buffer(packet, vertex_count_usize)?)
        } else {
            None
        };

        let small_vertices = vertex_count <= u32::from(u16::MAX);
        let mut renders = Vec::with_capacity(usize::from(mesh_count));
        for _ in 0..usize::from(mesh_count) {
            renders.push(Render::decode(packet, small_vertices)?);
        }

        Ok(Self {
            group_flags,
            unkint,
            face_count,
            has_vertices,
            has_vertex_alpha,
            has_face_bones,
            has_bone_ids,
            is_hidden,
            has_skin,
            vertex_count,
            position_buffer,
            normal_buffer,
            tangent_buffer,
            uv_buffer,
            boneid_buffer,
            skin,
            vertex_colours,
            vertex_alpha,
            vertex_facebones,
            renders,
        })
    }
}

impl Render {
    fn decode(packet: &mut Packet<'_>, small_vertices: bool) -> Result<Self> {
        let group_flags = packet.g1()?;
        let has_vertices = bit(group_flags, 0);
        let has_vertex_alpha = bit(group_flags, 1);
        let has_face_bones = bit(group_flags, 2);
        let has_bone_ids = bit(group_flags, 3);
        let is_hidden = bit(group_flags, 4);
        let has_skin = bit(group_flags, 5);
        let unkint = read_uint_be(packet)?;
        let material_argument = packet.g2_le()?;
        let unkbyte2 = packet.g1()?;
        let len = usize::from(packet.g2_le()?);
        let indices = if small_vertices {
            RenderIndices::Small(read_u16_buffer(packet, len)?)
        } else {
            RenderIndices::Large(read_u32_buffer_le(packet, len)?)
        };
        Ok(Self {
            group_flags,
            has_vertices,
            has_vertex_alpha,
            has_face_bones,
            has_bone_ids,
            is_hidden,
            has_skin,
            unkint,
            material_argument,
            unkbyte2,
            indices,
        })
    }
}

impl LegacySkin {
    fn decode(packet: &mut Packet<'_>) -> Result<Self> {
        let skin_weight_count = read_uint_le(packet)?;
        let count = usize::try_from(skin_weight_count).context("skin weight count too large")?;
        let skin_bone_buffer = read_u16_buffer(packet, count)?;
        let skin_weight_buffer = read_u8_buffer(packet, count)?;
        Ok(Self {
            skin_weight_count,
            skin_bone_buffer,
            skin_weight_buffer,
        })
    }
}

impl SkinWeight {
    fn decode(packet: &mut Packet<'_>) -> Result<Self> {
        let id_count = usize::from(packet.g2_le()?);
        let ids = read_u16_buffer(packet, id_count)?;
        let weight_count = usize::from(packet.g2_le()?);
        let weights = read_u8_buffer(packet, weight_count)?;
        Ok(Self { ids, weights })
    }
}

fn bit(value: u8, offset: u8) -> bool {
    ((value >> offset) & 1) == 1
}

fn read_u8_buffer(packet: &mut Packet<'_>, count: usize) -> Result<Vec<u8>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(packet.g1()?);
    }
    Ok(values)
}

fn read_u16_buffer(packet: &mut Packet<'_>, count: usize) -> Result<Vec<u16>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(packet.g2_le()?);
    }
    Ok(values)
}

fn read_u32_buffer_le(packet: &mut Packet<'_>, count: usize) -> Result<Vec<u32>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_uint_le(packet)?);
    }
    Ok(values)
}

fn read_i8_matrix(packet: &mut Packet<'_>, count: usize, width: usize) -> Result<Vec<Vec<i8>>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = Vec::with_capacity(width);
        for _ in 0..width {
            row.push(packet.g1s()?);
        }
        values.push(row);
    }
    Ok(values)
}

fn read_i16_matrix(packet: &mut Packet<'_>, count: usize, width: usize) -> Result<Vec<Vec<i16>>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = Vec::with_capacity(width);
        for _ in 0..width {
            row.push(packet.g2s_le()?);
        }
        values.push(row);
    }
    Ok(values)
}

fn read_u16_matrix(packet: &mut Packet<'_>, count: usize, width: usize) -> Result<Vec<Vec<u16>>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = Vec::with_capacity(width);
        for _ in 0..width {
            row.push(packet.g2_le()?);
        }
        values.push(row);
    }
    Ok(values)
}

fn read_f32_matrix(packet: &mut Packet<'_>, count: usize, width: usize) -> Result<Vec<Vec<f32>>> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = Vec::with_capacity(width);
        for _ in 0..width {
            row.push(packet.gfloat_le()?);
        }
        values.push(row);
    }
    Ok(values)
}

fn read_hex_buffers(packet: &mut Packet<'_>, count: u8, length: usize) -> Result<Vec<String>> {
    let mut values = Vec::with_capacity(usize::from(count));
    for _ in 0..usize::from(count) {
        let start = packet.pos();
        let end = start
            .checked_add(length)
            .context("hex buffer range overflow")?;
        let bytes = packet.slice(start, end)?;
        values.push(hex::encode(bytes));
        packet.set_pos(end)?;
    }
    Ok(values)
}

fn read_uint_be(packet: &mut Packet<'_>) -> Result<u32> {
    Ok(packet.g4s()? as u32)
}

fn read_uint_le(packet: &mut Packet<'_>) -> Result<u32> {
    Ok(packet.g4s_le()? as u32)
}
