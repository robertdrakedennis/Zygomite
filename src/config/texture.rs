use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct TextureEntry {
    pub id: u32,
    pub averagecolour: u16,
    pub opaque: bool,
    pub sprite: i32,
    pub unknown1: i32,
    pub animation: (u8, u8),
}

pub fn parse_texture(id: u32, data: &[u8]) -> Result<TextureEntry> {
    let mut packet = Packet::new(data);
    let averagecolour = packet.g2()?;
    let opaque = packet.g1()? == 1;
    if packet.g1()? != 1 {
        bail!("texture {id} unsupported format");
    }
    let sprite = packet.g2null()?;
    let unknown1 = packet.g4s()?;
    let animation = (packet.g1()?, packet.g1()?);
    if !packet.is_done() {
        bail!("texture {id} did not consume full payload");
    }
    Ok(TextureEntry {
        id,
        averagecolour,
        opaque,
        sprite,
        unknown1,
        animation,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct StylesheetPropertyEntry {
    pub unknown: u8,
    pub key_hash: i32,
    pub value: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct StylesheetEntry {
    pub id: u32,
    pub parent: i32,
    pub entries: Vec<StylesheetPropertyEntry>,
}

pub fn parse_stylesheet(id: u32, data: &[u8]) -> Result<StylesheetEntry> {
    let mut packet = Packet::new(data);
    let parent = packet.g2null()?;
    let count = usize::from(packet.g2()?);
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(StylesheetPropertyEntry {
            unknown: packet.g1()?,
            key_hash: packet.g4s()?,
            value: packet.g4s()?,
        });
    }
    if !packet.is_done() {
        bail!("stylesheet {id} did not consume full payload");
    }
    Ok(StylesheetEntry {
        id,
        parent,
        entries,
    })
}
