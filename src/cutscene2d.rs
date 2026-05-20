use crate::packet::Packet;
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2D {
    pub version: u8,
    pub width: u16,
    pub height: u16,
    pub text: u16,
    pub scenes: Vec<Cutscene2DScene>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DScene {
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub images: Vec<Cutscene2DImageElement>,
    pub audio: Vec<Cutscene2DAudioElement>,
    pub subtitles: Vec<Cutscene2DSubtitleElement>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DImageElement {
    pub name: String,
    pub height: u16,
    pub width: u16,
    pub id: i32,
    pub animation1: Cutscene2DAnimationFloat,
    pub animation2: Cutscene2DAnimationFloat,
    pub animation3: Cutscene2DAnimationVector3,
    pub animation4: Cutscene2DAnimationVector3,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DAudioElement {
    pub name: String,
    pub id: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DSubtitleElement {
    pub name: String,
    pub id: i32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DAnimationFloat {
    pub keys: Vec<Cutscene2DAnimationFloatKey>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DAnimationFloatKey {
    pub unknown0: f32,
    pub unknown1: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DAnimationVector3 {
    pub keys: Vec<Cutscene2DAnimationVector3Key>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cutscene2DAnimationVector3Key {
    pub unknown0: f32,
    pub unknown1: f32,
    pub unknown2: f32,
}

pub fn decode(data: &[u8]) -> Result<Cutscene2D> {
    let mut packet = Packet::new(data);
    let version = packet.g1()?;
    if version != 1 {
        bail!("cutscene2d unsupported version {version}");
    }

    let width = packet.g2()?;
    let height = packet.g2()?;
    let text = packet.g2()?;

    let scene_count = usize::from(packet.g1()?);
    let mut scenes = Vec::with_capacity(scene_count);
    for _ in 0..scene_count {
        scenes.push(decode_scene(&mut packet)?);
    }

    Ok(Cutscene2D {
        version,
        width,
        height,
        text,
        scenes,
    })
}

fn decode_scene(packet: &mut Packet<'_>) -> Result<Cutscene2DScene> {
    let name = packet.gjstr2()?;
    let start = f32::from_bits(packet.g4s()? as u32);
    let end = f32::from_bits(packet.g4s()? as u32);

    let image_count = usize::from(packet.g1()?);
    let mut images = Vec::with_capacity(image_count);
    for _ in 0..image_count {
        images.push(decode_image(packet)?);
    }

    let audio_count = usize::from(packet.g1()?);
    let mut audio = Vec::with_capacity(audio_count);
    for _ in 0..audio_count {
        audio.push(Cutscene2DAudioElement {
            name: packet.gjstr2()?,
            id: packet.g4s()?,
        });
    }

    let subtitle_count = usize::from(packet.g1()?);
    let mut subtitles = Vec::with_capacity(subtitle_count);
    for _ in 0..subtitle_count {
        subtitles.push(Cutscene2DSubtitleElement {
            name: packet.gjstr2()?,
            id: packet.g4s()?,
            x: f32::from_bits(packet.g4s()? as u32),
            y: f32::from_bits(packet.g4s()? as u32),
        });
    }

    Ok(Cutscene2DScene {
        name,
        start,
        end,
        images,
        audio,
        subtitles,
    })
}

fn decode_image(packet: &mut Packet<'_>) -> Result<Cutscene2DImageElement> {
    let name = packet.gjstr2()?;
    let height = packet.g2()?;
    let width = packet.g2()?;
    let id = packet.g4s()?;

    Ok(Cutscene2DImageElement {
        name,
        height,
        width,
        id,
        animation1: decode_animation_float(packet)?,
        animation2: decode_animation_float(packet)?,
        animation3: decode_animation_vector3(packet)?,
        animation4: decode_animation_vector3(packet)?,
    })
}

fn decode_animation_float(packet: &mut Packet<'_>) -> Result<Cutscene2DAnimationFloat> {
    let key_count = usize::from(packet.g1()?);
    let mut keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        keys.push(Cutscene2DAnimationFloatKey {
            unknown0: f32::from_bits(packet.g4s()? as u32),
            unknown1: f32::from_bits(packet.g4s()? as u32),
        });
    }
    Ok(Cutscene2DAnimationFloat { keys })
}

fn decode_animation_vector3(packet: &mut Packet<'_>) -> Result<Cutscene2DAnimationVector3> {
    let key_count = usize::from(packet.g1()?);
    let mut keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        keys.push(Cutscene2DAnimationVector3Key {
            unknown0: f32::from_bits(packet.g4s()? as u32),
            unknown1: f32::from_bits(packet.g4s()? as u32),
            unknown2: f32::from_bits(packet.g4s()? as u32),
        });
    }
    Ok(Cutscene2DAnimationVector3 { keys })
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_cutscene2d_payload() {
        let mut bytes = Vec::new();
        push_u8(&mut bytes, 1);
        push_u16_be(&mut bytes, 320);
        push_u16_be(&mut bytes, 200);
        push_u16_be(&mut bytes, 5);
        push_u8(&mut bytes, 1);

        push_jstr2(&mut bytes, "intro");
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 1.5);

        push_u8(&mut bytes, 1);
        push_jstr2(&mut bytes, "img");
        push_u16_be(&mut bytes, 64);
        push_u16_be(&mut bytes, 128);
        push_i32_be(&mut bytes, 42);

        push_u8(&mut bytes, 1);
        push_f32_be(&mut bytes, 0.1);
        push_f32_be(&mut bytes, 0.2);

        push_u8(&mut bytes, 0);

        push_u8(&mut bytes, 1);
        push_f32_be(&mut bytes, 1.0);
        push_f32_be(&mut bytes, 2.0);
        push_f32_be(&mut bytes, 3.0);

        push_u8(&mut bytes, 0);

        push_u8(&mut bytes, 1);
        push_jstr2(&mut bytes, "snd");
        push_i32_be(&mut bytes, 9);

        push_u8(&mut bytes, 1);
        push_jstr2(&mut bytes, "sub");
        push_i32_be(&mut bytes, 7);
        push_f32_be(&mut bytes, 0.5);
        push_f32_be(&mut bytes, 0.75);

        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("cutscene decode should succeed: {error}"),
        };
        assert_eq!(1, decoded.version);
        assert_eq!(1, decoded.scenes.len());
        assert_eq!("intro", decoded.scenes[0].name);
        assert_eq!(1, decoded.scenes[0].images.len());
        assert_eq!("img", decoded.scenes[0].images[0].name);
        assert_eq!(1, decoded.scenes[0].images[0].animation1.keys.len());
        assert_eq!(1, decoded.scenes[0].audio.len());
        assert_eq!(1, decoded.scenes[0].subtitles.len());
    }

    #[test]
    fn rejects_unknown_version() {
        let Err(err) = decode(&[2_u8]) else {
            panic!("version 2 should fail");
        };
        let text = err.to_string();
        assert!(text.contains("unsupported version"));
    }

    fn push_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    fn push_u16_be(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i32_be(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_f32_be(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_bits().to_be_bytes());
    }

    fn push_jstr2(out: &mut Vec<u8>, value: &str) {
        out.push(0);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }
}
