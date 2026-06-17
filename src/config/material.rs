use super::{gfloat_be, yes_no, OpListEntry};
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;

pub fn parse_material(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let version = packet.g1()?;
    let mut ops = vec![format!("version={version}")];

    match version {
        0 => parse_material_v0(&mut packet, &mut ops)?,
        1 | 2 => parse_material_v1(&mut packet, &mut ops)?,
        _ => bail!("unsupported material version {version} in {id}"),
    }

    if !packet.is_done() {
        bail!("material {id} did not consume full payload (pos {} of {})", packet.pos(), data.len());
    }
    Ok(OpListEntry { id, ops })
}

// Material format uses short names for texture/animation IDs.
#[allow(clippy::similar_names)]
fn parse_material_v0(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    ops.push(format!("unknown1={}", packet.g1()?));
    ops.push(format!("size={}", packet.g1()?));
    let flags_a = packet.g4s()?;

    let flaga0 = (flags_a & 1) != 0;
    let flaga1 = (flags_a & 2) != 0;
    let flaga2 = (flags_a & 4) != 0;
    let flaga3 = (flags_a & 8) != 0;
    let flaga4 = (flags_a & 16) != 0;

    if flaga0 {
        ops.push(String::from("flaga0=yes"));
    }
    if flaga1 {
        ops.push(String::from("flaga1=yes"));
    }
    if flaga2 {
        ops.push(String::from("flaga2=yes"));
    }
    if flaga3 {
        ops.push(String::from("flaga3=yes"));
    }
    if flaga4 {
        ops.push(String::from("flaga4=yes"));
    }

    if flaga0 || flaga4 {
        ops.push(format!("texture={}", packet.g4s()?));
    }
    if flaga3 || flaga1 {
        ops.push(format!("bloomtexture={}", packet.g4s()?));
    }

    let repeat = packet.g1()?;
    ops.push(format!("repeat={},{}", repeat & 7, (repeat >> 3) & 7));

    let flags_b = packet.g4s()?;
    let flagb4 = (flags_b & 0x10) != 0;
    let flagb5 = (flags_b & 0x20) != 0;
    let flagb6 = (flags_b & 0x40) != 0;
    let flagb11 = (flags_b & 0x800) != 0;
    // Build 948+: new flag bit 23 gates one BE float read after the flags_c
    // speed block (byte-mapped against material 3224 in 947.1 vs 948.1).
    let flagb23 = (flags_b & 0x0080_0000) != 0;
    let flagb18 = (flags_b & 0x40000) != 0;
    let flagb19 = (flags_b & 0x80000) != 0;
    let flagb20 = (flags_b & 0x0010_0000) != 0;
    let flagb21 = (flags_b & 0x0020_0000) != 0;

    ops.push(format!("flagb0={}", yes_no((flags_b & 1) != 0)));
    ops.push(format!("flagb1={}", yes_no((flags_b & 2) != 0)));
    ops.push(format!("flagb2={}", yes_no((flags_b & 4) != 0)));
    ops.push(format!("flagb4={}", yes_no(flagb4)));
    ops.push(format!("flagb21={}", yes_no(flagb21)));
    ops.push(format!("flagb20={}", yes_no(flagb20)));

    if flagb5 {
        ops.push(format!("unknown19={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb6 {
        ops.push(format!("unknown20={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb18 {
        ops.push(format!("unknown4={}", packet.g4s()?));
    }
    if flagb19 {
        ops.push(format!(
            "unknown5={},{},{},{},{}",
            packet.g4s()?,
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            packet.g4s()?,
            packet.g4s()?
        ));
    }
    if flagb4 {
        ops.push(format!(
            "unknown6={},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?
        ));
    }
    if flaga1 {
        ops.push(format!("unknown7={}", gfloat_be(packet)?));
    }

    ops.push(format!("bloom={}", yes_no(packet.g1()? == 1)));
    ops.push(format!("facetmode={}", packet.g1()?));

    match packet.g1()? {
        0 => ops.push(String::from("alphamode=none")),
        1 => ops.push(format!("alphamode=test,{}", packet.g1()?)),
        2 => ops.push(String::from("alphamode=multiply")),
        value => bail!("unknown material alphamode {value}"),
    }

    if flagb11 {
        ops.push(format!(
            "unknown9={},{},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            gfloat_be(packet)?
        ));
    }

    let flags_c = packet.g1()?;
    if (flags_c & 1) != 0 {
        ops.push(format!("speedu={}", packet.g2s()?));
    }
    if (flags_c & 2) != 0 {
        ops.push(format!("speedv={}", packet.g2s()?));
    }

    if flagb23 {
        ops.push(format!("unknown27={}", gfloat_be(packet)?));
    }

    if packet.g1()? == 1 {
        ops.push(format!("effect={}", packet.g1()?));
        ops.push(format!("effectarg1={}", packet.g1()?));
        ops.push(format!("effectarg2={}", packet.g4s()?));
        ops.push(format!("effectcombiner={}", packet.g1()?));
        ops.push(format!("unknown15={}", yes_no(packet.g1()? == 1)));
        ops.push(format!("mipmapping={}", packet.g1()?));
        ops.push(format!("lowdetail={}", yes_no(packet.g1()? == 1)));
        ops.push(format!("highdetail={}", yes_no(packet.g1()? == 1)));
        ops.push(format!("lightness={}", packet.g1()?));
        ops.push(format!("saturation={}", packet.g1()?));
        ops.push(format!("averagecolour={}", packet.g2()?));
    }
    Ok(())
}

// Same pattern as v0; texture/animation variable naming follows game conventions.
#[allow(clippy::similar_names)]
fn parse_material_v1(packet: &mut Packet<'_>, ops: &mut Vec<String>) -> Result<()> {
    let flags_b = packet.g4s()?;
    if (flags_b >> 22) != 0 {
        bail!("invalid material flags {flags_b}");
    }

    let flagb5 = (flags_b & 0x20) != 0;
    let flagb6 = (flags_b & 0x40) != 0;
    let flagb7 = (flags_b & 0x80) != 0;
    let flagb8 = (flags_b & 0x100) != 0;
    let flagb9 = (flags_b & 0x200) != 0;
    let flagb11 = (flags_b & 0x800) != 0;
    let flagb12 = (flags_b & 0x1000) != 0;
    let flagb13 = (flags_b & 0x2000) != 0;
    let flagb14 = (flags_b & 0x4000) != 0;
    let flagb15 = (flags_b & 0x8000) != 0;
    let flagb16 = (flags_b & 0x10000) != 0;
    let flagb17 = (flags_b & 0x20000) != 0;
    let flagb18 = (flags_b & 0x40000) != 0;
    let flagb19 = (flags_b & 0x80000) != 0;
    let flagb20 = (flags_b & 0x0010_0000) != 0;
    let flagb21 = (flags_b & 0x0020_0000) != 0;

    for (label, bit) in [
        ("flagsb0", 1),
        ("flagsb1", 2),
        ("flagsb2", 4),
        ("flagsb3", 8),
        ("flagsb4", 0x10),
        ("flagsb10", 0x400),
    ] {
        if (flags_b & bit) != 0 {
            ops.push(format!("{label}=yes"));
        }
    }
    if flagb21 {
        ops.push(String::from("flagsb21=yes"));
    }
    if flagb20 {
        ops.push(String::from("flagsb20=yes"));
    }

    if flagb5 {
        ops.push(format!("unknown19={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb6 {
        ops.push(format!("unknown20={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb7 {
        ops.push(format!("unknown21={},{}", packet.g1()?, packet.g4s()?));
    }
    if flagb18 {
        ops.push(format!("unknown4={}", packet.g4s()?));
    }
    if flagb19 {
        ops.push(format!(
            "unknown5={},{},{},{},{}",
            packet.g4s()?,
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            packet.g4s()?,
            packet.g4s()?
        ));
    }
    if flagb12 {
        ops.push(format!("unknown6={}", gfloat_be(packet)?));
    }
    if flagb13 {
        ops.push(format!("unknown7={}", packet.g4s()?));
    }
    if flagb14 {
        ops.push(format!("unknown22={}", gfloat_be(packet)?));
    }
    if flagb15 {
        ops.push(format!("unknown23={}", packet.g4s()?));
    }
    if flagb6 {
        ops.push(format!("unknown24={}", gfloat_be(packet)?));
    }
    if flagb11 {
        ops.push(format!(
            "unknown9={},{},{}",
            gfloat_be(packet)?,
            gfloat_be(packet)?,
            gfloat_be(packet)?
        ));
    }
    if flagb16 {
        ops.push(format!("unknown25={}", gfloat_be(packet)?));
    }
    if flagb17 {
        ops.push(format!("unknown26={}", gfloat_be(packet)?));
    }
    if flagb8 {
        ops.push(format!("speedu={}", packet.g2s()?));
    }
    if flagb9 {
        ops.push(format!("speedv={}", packet.g2s()?));
    }

    let repeat = packet.g1()?;
    ops.push(format!("repeat={},{}", repeat & 7, (repeat >> 3) & 7));
    ops.push(format!("facetmode={}", packet.g1()?));
    ops.push(format!("qualitymode={}", packet.g1()?));
    match packet.g1()? {
        0 => ops.push(String::from("alphamode=none")),
        1 => ops.push(format!("alphamode=test,{}", packet.g1()?)),
        2 => ops.push(String::from("alphamode=multiply")),
        value => bail!("unknown material alphamode {value}"),
    }
    ops.push(format!("averagecolour={}", packet.g2()?));
    ops.push(format!("size={}", packet.g1()?));
    Ok(())
}
