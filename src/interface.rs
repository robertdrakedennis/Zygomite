use crate::cache_bail as bail;
use crate::error::{CacheError, Result};
use crate::packet::Packet;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashSet;

/// Full component UID used by CS2 opcodes: `(interface_id << 16) | component_id`.
pub fn component_uid(interface_id: u32, component_id: u32) -> u32 {
    (interface_id << 16) | (component_id & 0xFFFF)
}

/// Fallback TypeScript property name when the interface binary has no explicit name.
pub fn component_fallback_name(interface_id: u32, component_id: u32) -> String {
    format!("Interface_{interface_id}_Com_{component_id}")
}

pub fn render_interface_group(
    group: u32,
    files: &BTreeMap<u32, Vec<u8>>,
    build: u32,
) -> Vec<String> {
    let mut out = Vec::new();
    for (component, data) in files {
        match parse_component(*component, data, build) {
            Ok(lines) => {
                out.extend(lines);
                out.push(String::new());
            }
            Err(error) => {
                out.push(format!("[com{component}]"));
                out.push("type=unsupported".to_string());
                out.push(format!(
                    "parse_error={}",
                    error.to_string().replace('\n', " ")
                ));
                out.push(String::new());
            }
        }
    }
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.insert(0, format!("; interface_{group}"));
    out
}

pub fn parse_component(component_id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = Packet::new(data);
    let mut lines = Vec::new();
    lines.push(format!("[com{}]", component_id & 0xFFFF));

    let mut version = i16::from(packet.g1()?);
    if build < 566 && version != i16::from(u8::MAX) {
        return Ok(lines);
    }
    if version == i16::from(u8::MAX) {
        version = -1;
    }

    let mut if_type = packet.g1()?;
    lines.push(format!(
        "type={}",
        format_if_type(i32::from(if_type & 0x7F))
    ));

    if (if_type & 128) != 0 {
        if_type &= 127;
        let name = packet.gjstr()?;
        if !name.is_empty() {
            lines.push(format!("name={name}"));
        }
    }

    let contenttype = packet.g2()?;
    if contenttype != 0 {
        lines.push(format!("contenttype={contenttype}"));
    }

    let x = packet.g2s()?;
    let y = packet.g2s()?;
    let width = packet.g2()?;
    let height = packet.g2()?;
    if x != 0 {
        lines.push(format!("x={x}"));
    }
    if y != 0 {
        lines.push(format!("y={y}"));
    }
    if width != 0 {
        lines.push(format!("width={width}"));
    }
    if height != 0 {
        lines.push(format!("height={height}"));
    }

    let mut width_mode: i8 = 0;
    let mut height_mode: i8 = 0;
    if build >= 493 {
        width_mode = packet.g1s()?;
        height_mode = packet.g1s()?;
        let x_mode = packet.g1s()?;
        let y_mode = packet.g1s()?;
        if width_mode != 0 {
            lines.push(format!("widthmode={}", decode_size_mode(width_mode)));
        }
        if height_mode != 0 {
            lines.push(format!("heightmode={}", decode_size_mode(height_mode)));
        }
        if x_mode != 0 {
            lines.push(format!("xmode={}", decode_x_mode(x_mode)));
        }
        if y_mode != 0 {
            lines.push(format!("ymode={}", decode_y_mode(y_mode)));
        }
    }

    if width_mode == 4 || height_mode == 4 {
        let aspect_width = packet.g2()?;
        let aspect_height = packet.g2()?;
        if aspect_width != 1 {
            lines.push(format!("aspectwidth={aspect_width}"));
        }
        if aspect_height != 1 {
            lines.push(format!("aspectheight={aspect_height}"));
        }
    }

    let layer = packet.g2null()?;
    if layer != -1 {
        lines.push(format!("layer=com{layer}"));
    }

    let flags = packet.g1()?;
    if (flags & 1) != 0 {
        lines.push("hide=yes".to_string());
    }
    if version >= 0 && (flags & 2) != 0 {
        lines.push("noclickthrough=yes".to_string());
    }

    let parsed_body = match if_type {
        0 => {
            decode_layer(&mut packet, &mut lines, version, build)?;
            true
        }
        3 => {
            decode_rectangle(&mut packet, &mut lines)?;
            true
        }
        4 => {
            decode_text(&mut packet, &mut lines, version, build)?;
            true
        }
        5 => {
            decode_graphic(&mut packet, &mut lines, version, build)?;
            true
        }
        6 => {
            decode_model(
                &mut packet,
                &mut lines,
                build,
                width_mode != 0,
                height_mode != 0,
            )?;
            true
        }
        9 => {
            decode_line(&mut packet, &mut lines, build)?;
            true
        }
        10 => {
            decode_button(&mut packet, &mut lines, version, build)?;
            true
        }
        11 => {
            decode_panel(&mut packet, &mut lines)?;
            true
        }
        12 => {
            decode_check(&mut packet, &mut lines, version, build)?;
            true
        }
        13 => {
            decode_input(&mut packet, &mut lines, version, build)?;
            true
        }
        15 => {
            decode_grid(&mut packet, &mut lines)?;
            true
        }
        16 => {
            decode_list(&mut packet, &mut lines, version, build)?;
            true
        }
        26 => {
            decode_crm_view(&mut packet, &mut lines)?;
            true
        }
        _ => false,
    };

    if parsed_body {
        decode_common_tail(&mut packet, &mut lines, version, build)?;
        let remaining = packet.len().saturating_sub(packet.pos());
        if remaining != 0 {
            bail!("end of file not reached: {remaining} bytes remaining");
        }
    }

    Ok(lines)
}

fn decode_layer(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    let scroll_width = packet.g2()?;
    let scroll_height = packet.g2()?;
    if scroll_width != 0 {
        lines.push(format!("scrollwidth={scroll_width}"));
    }
    if scroll_height != 0 {
        lines.push(format!("scrollheight={scroll_height}"));
    }

    if version == -1 && build >= 495 {
        if packet.g1()? == 1 {
            lines.push("noclickthrough=yes".to_string());
        }
    } else if version >= 9 {
        let margin = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
        if margin != [0, 0, 0, 0] {
            lines.push(format!(
                "margin={},{},{},{}",
                margin[0], margin[1], margin[2], margin[3]
            ));
        }
    } else if version >= 6 {
        let margin = [packet.g2()?, packet.g2()?, packet.g2()?, packet.g2()?];
        if margin != [0, 0, 0, 0] {
            lines.push(format!(
                "margin={},{},{},{}",
                margin[0], margin[1], margin[2], margin[3]
            ));
        }
    }
    Ok(())
}

fn decode_rectangle(packet: &mut Packet<'_>, lines: &mut Vec<String>) -> Result<()> {
    lines.push(format!("colour={}", format_colour(packet.g4s()?)));
    if packet.g1()? == 1 {
        lines.push("fill=yes".to_string());
    }
    let trans = packet.g1()?;
    if trans != 0 {
        lines.push(format!("trans={trans}"));
    }
    Ok(())
}

fn decode_graphic(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    decode_sprite_part(
        "",
        lines,
        packet,
        version,
        build,
        0xFFFF_FFFF_u32 as i32,
        "yes",
    )?;
    Ok(())
}

fn decode_model(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    build: u32,
    has_width_mode: bool,
    has_height_mode: bool,
) -> Result<()> {
    let model = if build < 681 {
        packet.g2null()?
    } else {
        packet.gsmart2or4null()?
    };
    lines.push(format!("model={}", format_model(model)));

    if build < 619 {
        lines.push(format!("modelorigin_x={}", packet.g2s()?));
        lines.push(format!("modelorigin_y={}", packet.g2s()?));
        lines.push(format!("modelangle_x={}", packet.g2()?));
        lines.push(format!("modelangle_y={}", packet.g2()?));
        lines.push(format!("modelangle_z={}", packet.g2()?));
        lines.push(format!("modelzoom={}", packet.g2()?));
        let modelanim = if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        };
        if modelanim != -1 {
            lines.push(format!("modelanim={}", format_seq(modelanim)));
        }
        if packet.g1()? == 1 {
            lines.push("modelorthog=yes".to_string());
        }
        if build >= 493 {
            lines.push(format!("unknown100={}", packet.g2()?));
        }
        if build >= 501 {
            lines.push(format!("unknown101={}", packet.g2()?));
            if packet.g1()? == 1 {
                lines.push("unknown103=yes".to_string());
            }
        }
    } else {
        let model_flags = packet.g1()?;
        let has_transform = (model_flags & 1) != 0;
        let has_precise_zoom = (model_flags & 2) != 0;
        let has_orthographic = (model_flags & 4) != 0;
        let has_no_depth = (model_flags & 8) != 0;
        if has_precise_zoom {
            lines.push("modelprecisezoom=yes".to_string());
        }
        if has_orthographic {
            lines.push("modelorthog=yes".to_string());
        }
        if has_no_depth {
            lines.push("modelnodepth=yes".to_string());
        }

        if has_transform {
            lines.push(format!("modelorigin_x={}", packet.g2s()?));
            lines.push(format!("modelorigin_y={}", packet.g2s()?));
            lines.push(format!("modelangle_x={}", packet.g2()?));
            lines.push(format!("modelangle_y={}", packet.g2()?));
            lines.push(format!("modelangle_z={}", packet.g2()?));
            lines.push(format!("modelzoom={}", packet.g2()?));
        } else if has_precise_zoom {
            lines.push(format!("modelorigin_x={}", packet.g2s()?));
            lines.push(format!("modelorigin_y={}", packet.g2s()?));
            lines.push(format!("modelorigin_z={}", packet.g2s()?));
            lines.push(format!("modelangle_x={}", packet.g2()?));
            lines.push(format!("modelangle_y={}", packet.g2()?));
            lines.push(format!("modelangle_z={}", packet.g2()?));
            lines.push(format!("modelzoom={}", packet.g2()?));
        }

        let modelanim = if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        };
        if modelanim != -1 {
            lines.push(format!("modelanim={}", format_seq(modelanim)));
        }
    }

    if has_width_mode {
        lines.push(format!("modelobjwidth={}", packet.g2()?));
    }
    if has_height_mode {
        lines.push(format!("modelobjheight={}", packet.g2()?));
    }
    Ok(())
}

fn decode_text(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    decode_text_part(
        "",
        lines,
        packet,
        version,
        build,
        TextPartDefaults {
            align_h: 0,
            align_v: 0,
            text_shadow: "no",
        },
    )?;
    Ok(())
}

fn decode_line(packet: &mut Packet<'_>, lines: &mut Vec<String>, build: u32) -> Result<()> {
    let linewid = packet.g1()?;
    if linewid != 1 {
        lines.push(format!("linewid={linewid}"));
    }
    lines.push(format!("colour={}", format_colour(packet.g4s()?)));
    if build >= 493 && packet.g1()? == 1 {
        lines.push("linedirection=yes".to_string());
    }
    Ok(())
}

fn decode_button(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    if packet.g1()? == 0 {
        lines.push("enabled=no".to_string());
    }
    if packet.g1()? == 1 {
        lines.push("cantoggle=yes".to_string());
    }
    let unknown1 = packet.g1()?;
    if unknown1 != 1 {
        lines.push(format!("unknown1={unknown1}"));
    }
    if packet.g1()? == 0 {
        lines.push("setlinkobjoptions1=no".to_string());
    }
    if packet.g1()? == 0 {
        lines.push("setlinkobjoptions2=no".to_string());
    }
    let text_area = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
    if text_area != [0, 0, 0, 0] {
        lines.push(format!(
            "textareasizeoffsets={},{},{},{}",
            text_area[0], text_area[1], text_area[2], text_area[3]
        ));
    }

    let trans = packet.g1()?;
    if trans != 0 {
        lines.push(format!("trans={trans}"));
    }
    let colour = packet.g4s()?;
    if colour != 0x00FF_FFFF {
        lines.push(format!("colour={}", format_colour(colour)));
    }
    decode_sprite_part("sprite.", lines, packet, version, build, 0x00FF_FFFF, "no")?;
    decode_text_part(
        "text.",
        lines,
        packet,
        version,
        build,
        TextPartDefaults {
            align_h: 1,
            align_v: 1,
            text_shadow: "yes",
        },
    )?;
    Ok(())
}

fn decode_panel(packet: &mut Packet<'_>, lines: &mut Vec<String>) -> Result<()> {
    let scroll_width = packet.g2()?;
    let scroll_height = packet.g2()?;
    if scroll_width != 0 {
        lines.push(format!("scrollwidth={scroll_width}"));
    }
    if scroll_height != 0 {
        lines.push(format!("scrollheight={scroll_height}"));
    }
    if packet.g1()? == 1 {
        lines.push("isvertical=yes".to_string());
    }
    lines.push(format!("childspacing={}", packet.g1()?));
    Ok(())
}

fn decode_check(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    if packet.g1()? == 0 {
        lines.push("enabled=no".to_string());
    }
    if packet.g1()? == 1 {
        lines.push("checked=yes".to_string());
    }
    let alignment = packet.g1()?;
    if alignment != 0 {
        lines.push(format!("alignment={alignment}"));
    }
    let buttonsize = packet.g1()?;
    if buttonsize != 0 {
        lines.push(format!("buttonsize={buttonsize}"));
    }
    let trans = packet.g1()?;
    if trans != 0 {
        lines.push(format!("trans={trans}"));
    }
    let colour = packet.g4s()?;
    if colour != 0xFFFF_FFFF_u32 as i32 {
        lines.push(format!("colour={}", format_colour(colour)));
    }
    decode_sprite_part("sprite.", lines, packet, version, build, 0x00FF_FFFF, "no")?;
    decode_text_part(
        "text.",
        lines,
        packet,
        version,
        build,
        TextPartDefaults {
            align_h: 0,
            align_v: 0,
            text_shadow: "yes",
        },
    )?;
    Ok(())
}

fn decode_input(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    if packet.g1()? == 0 {
        lines.push("enabled=no".to_string());
    }
    lines.push(format!("filtermode={}", packet.g1()?));
    lines.push(format!("visibilitymode={}", packet.g1()?));
    lines.push(format!("unknown8={}", packet.g2()?));

    if version >= 9 {
        let margin = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
        if margin != [0, 0, 0, 0] {
            lines.push(format!(
                "margin={},{},{},{}",
                margin[0], margin[1], margin[2], margin[3]
            ));
        }
    }
    if version >= 7 {
        let keyhandlingmode = packet.g1()?;
        if keyhandlingmode != 0 {
            lines.push(format!("keyhandlingmode={keyhandlingmode}"));
        }
    }

    let trans = packet.g1()?;
    if trans != 0 {
        lines.push(format!("trans={trans}"));
    }
    lines.push(format!("colour={}", format_colour(packet.g4s()?)));
    decode_sprite_part("sprite.", lines, packet, version, build, 0x00FF_FFFF, "no")?;
    decode_text_part(
        "text.",
        lines,
        packet,
        version,
        build,
        TextPartDefaults {
            align_h: 0,
            align_v: 0,
            text_shadow: "no",
        },
    )?;
    decode_scrollbar_part("scrollbar.", lines, packet, version, build)?;
    Ok(())
}

fn decode_grid(packet: &mut Packet<'_>, lines: &mut Vec<String>) -> Result<()> {
    let scroll_width = packet.g2()?;
    let scroll_height = packet.g2()?;
    if scroll_width != 0 {
        lines.push(format!("scrollwidth={scroll_width}"));
    }
    if scroll_height != 0 {
        lines.push(format!("scrollheight={scroll_height}"));
    }
    lines.push(format!("childspacing={}", packet.g1()?));
    lines.push(format!("layoutparams_x={}", packet.g2()?));
    lines.push(format!("layoutparams_y={}", packet.g2()?));
    if packet.g1()? == 1 {
        lines.push("layoutparams_mode=yes".to_string());
    }
    Ok(())
}

fn decode_list(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    if packet.g1()? == 0 {
        lines.push("enabled=no".to_string());
    }
    lines.push(format!("dropdownnumentries={}", packet.g1()?));
    lines.push(format!("selectionlimit={}", packet.g1()?));
    lines.push(format!("entryheight={}", packet.g1()?));

    if version >= 9 {
        lines.push(format!("entryiconscale={}", packet.g1()?));
    }

    lines.push(format!("dropdownbuttonparams_size={}", packet.g1()?));
    lines.push(format!("dropdownbuttonparams_offset={}", packet.g1()?));

    for _ in 0..packet.g2()? {
        lines.push(format!("unknown14={}", packet.gjstr()?));
    }
    for _ in 0..packet.g2()? {
        lines.push(format!("unknown15={}", packet.g4s()?));
    }
    for _ in 0..packet.g2()? {
        lines.push(format!("unknown16={}", packet.g2()?));
    }

    if version >= 9 {
        let margin1 = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
        if margin1 != [0, 0, 0, 0] {
            lines.push(format!(
                "margin1={},{},{},{}",
                margin1[0], margin1[1], margin1[2], margin1[3]
            ));
        }

        let margin2 = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
        if margin2 != [0, 0, 0, 0] {
            lines.push(format!(
                "margin2={},{},{},{}",
                margin2[0], margin2[1], margin2[2], margin2[3]
            ));
        }
    }

    let trans = packet.g1()?;
    if trans != 0 {
        lines.push(format!("trans={trans}"));
    }
    let colour = packet.g4s()?;
    if colour != 0xFFFF_FFFF_u32 as i32 {
        lines.push(format!("colour={}", format_colour(colour)));
    }

    decode_sprite_part("button.", lines, packet, version, build, 0x00FF_FFFF, "no")?;
    decode_sprite_part("header.", lines, packet, version, build, 0x00FF_FFFF, "no")?;
    decode_sprite_part("body.", lines, packet, version, build, 0x00FF_FFFF, "no")?;
    decode_text_part(
        "text.",
        lines,
        packet,
        version,
        build,
        TextPartDefaults {
            align_h: 0,
            align_v: 1,
            text_shadow: "yes",
        },
    )?;
    decode_scrollbar_part("scrollbar.", lines, packet, version, build)?;
    Ok(())
}

fn decode_crm_view(packet: &mut Packet<'_>, lines: &mut Vec<String>) -> Result<()> {
    if packet.g1()? == 1 {
        lines.push("unknown21=yes".to_string());
    }
    for _ in 0..packet.g2()? {
        lines.push(format!("unknown22={}", packet.g2()?));
    }
    lines.push(format!("unknown23={}", packet.gjstr()?));
    lines.push(format!("unknown24={}", packet.g1()?));
    for _ in 0..packet.g2()? {
        lines.push(format!("unknown25={}", packet.gjstr()?));
    }
    for _ in 0..packet.g2()? {
        lines.push(format!("unknown26={}", packet.g4s()?));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct TextPartDefaults {
    align_h: u8,
    align_v: u8,
    text_shadow: &'static str,
}

fn decode_text_part(
    prefix: &str,
    lines: &mut Vec<String>,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
    defaults: TextPartDefaults,
) -> Result<()> {
    let textfont = if build < 800 {
        if build < 681 {
            format_graphic(packet.g2null()?)
        } else {
            format_graphic(packet.gsmart2or4null()?)
        }
    } else {
        format_fontmetrics(packet.gsmart2or4null()?)
    };
    if textfont != "null" {
        lines.push(format!("{prefix}textfont={textfont}"));
    }

    if version >= 2 && packet.g1()? == 0 {
        lines.push(format!("{prefix}fontmono=no"));
    }

    let text = packet.gjstr()?;
    if !text.is_empty() {
        lines.push(format!("{prefix}text={text}"));
    }

    let line_height = packet.g1()?;
    if line_height != 0 {
        lines.push(format!("{prefix}textlineheight={line_height}"));
    }

    let align_h = packet.g1()?;
    if align_h != defaults.align_h {
        lines.push(format!("{prefix}textalignh={align_h}"));
    }

    let align_v = packet.g1()?;
    if align_v != defaults.align_v {
        lines.push(format!("{prefix}textalignv={align_v}"));
    }

    let text_shadow = if packet.g1()? == 1 { "yes" } else { "no" };
    if text_shadow != defaults.text_shadow {
        lines.push(format!("{prefix}textshadow={text_shadow}"));
    }

    let colour = packet.g4s()?;
    if colour != 0x00FF_FFFF {
        lines.push(format!("{prefix}colour={}", format_colour(colour)));
    }

    if build >= 582 {
        let trans = packet.g1()?;
        if trans != 0 {
            lines.push(format!("{prefix}trans={trans}"));
        }
    }

    if version >= 0 {
        let maxlines = packet.g1()?;
        if maxlines != 0 {
            lines.push(format!("{prefix}maxlines={maxlines}"));
        }
    }

    Ok(())
}

fn decode_sprite_part(
    prefix: &str,
    lines: &mut Vec<String>,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
    default_colour: i32,
    default_clickmask: &str,
) -> Result<()> {
    let graphic = format_graphic(packet.g4s()?);
    if graphic != "null" {
        lines.push(format!("{prefix}graphic={graphic}"));
    }

    let angle = packet.g2()?;
    if angle != 0 {
        lines.push(format!("{prefix}2dangle={angle}"));
    }

    let flags = packet.g1()?;
    if (flags & 1) != 0 {
        lines.push(format!("{prefix}tiling=yes"));
    }
    if (flags & 2) != 0 {
        lines.push(format!("{prefix}alpha=yes"));
    }

    let trans = packet.g1()?;
    if trans != 0 {
        lines.push(format!("{prefix}trans={trans}"));
    }

    let outline = packet.g1()?;
    if outline != 0 {
        lines.push(format!("{prefix}outline={outline}"));
    }

    let graphic_shadow = packet.g4s()?;
    if graphic_shadow != 0 {
        lines.push(format!("{prefix}graphicshadow={graphic_shadow}"));
    }

    if packet.g1()? == 1 {
        lines.push(format!("{prefix}vflip=yes"));
    }
    if packet.g1()? == 1 {
        lines.push(format!("{prefix}hflip=yes"));
    }

    if build >= 537 {
        let colour = packet.g4s()?;
        if colour != default_colour {
            lines.push(format!("{prefix}colour={}", format_colour(colour)));
        }
    }

    if version >= 3 {
        let clickmask = if packet.g1()? == 1 { "yes" } else { "no" };
        if clickmask != default_clickmask {
            lines.push(format!("{prefix}clickmask={clickmask}"));
        }
    }

    if version >= 6 {
        let edge = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
        if edge != [0, 0, 0, 0] {
            lines.push(format!(
                "{prefix}edge={},{},{},{}",
                edge[0], edge[1], edge[2], edge[3]
            ));
        }
    }
    Ok(())
}

fn decode_scrollbar_part(
    prefix: &str,
    lines: &mut Vec<String>,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    if packet.g1()? == 1 {
        lines.push(format!("{prefix}unknown17=yes"));
    }
    if packet.g1()? == 1 {
        lines.push(format!("{prefix}unknown18=yes"));
    }
    lines.push(format!("{prefix}unknown19={}", packet.g1()?));
    lines.push(format!("{prefix}unknown20={}", packet.g1()?));

    decode_sprite_part(
        &format!("{prefix}background."),
        lines,
        packet,
        version,
        build,
        0x00FF_FFFF,
        "no",
    )?;
    decode_sprite_part(
        &format!("{prefix}button."),
        lines,
        packet,
        version,
        build,
        0x00FF_FFFF,
        "no",
    )?;
    decode_sprite_part(
        &format!("{prefix}handle."),
        lines,
        packet,
        version,
        build,
        0x00FF_FFFF,
        "no",
    )?;
    Ok(())
}

fn decode_common_tail(
    packet: &mut Packet<'_>,
    lines: &mut Vec<String>,
    version: i16,
    build: u32,
) -> Result<()> {
    if version >= 6 {
        let stylesheet = packet.g4s()?;
        if stylesheet != -1 {
            lines.push(format!("stylesheet={}", format_stylesheet(stylesheet)));
        }
    }

    if version >= 9 {
        let unknown10 = packet.g1()?;
        if unknown10 != 0 {
            lines.push(format!("unknown10={unknown10}"));
        }
    }

    let events = if version < 6 {
        packet.g3()?
    } else {
        packet.g4s()? as u32
    };

    if (events & 1) != 0 {
        lines.push("pausebutton=yes".to_string());
    }
    for bit in 1_u32..=10 {
        if ((events >> bit) & 1) != 0 {
            lines.push(format!("transmitop{bit}=yes"));
        }
    }

    let targetmask = (events >> 11) & 0x7F;
    let mut target_parts = Vec::new();
    if (targetmask & (1 << 0)) != 0 {
        target_parts.push("obj");
    }
    if (targetmask & (1 << 1)) != 0 {
        target_parts.push("npc");
    }
    if (targetmask & (1 << 2)) != 0 {
        target_parts.push("loc");
    }
    if (targetmask & (1 << 3)) != 0 {
        target_parts.push("player");
    }
    if (targetmask & (1 << 4)) != 0 {
        target_parts.push("inv");
    }
    if (targetmask & (1 << 5)) != 0 {
        target_parts.push("com");
    }
    if (targetmask & (1 << 6)) != 0 {
        target_parts.push("coord");
    }
    if !target_parts.is_empty() {
        lines.push(format!("targetmask={}", target_parts.join(",")));
    }

    let dragdepth = (events >> 18) & 0b111;
    if dragdepth != 0 {
        lines.push(format!("dragdepth={dragdepth}"));
    }
    if ((events >> 21) & 1) != 0 {
        lines.push("candrop=yes".to_string());
    }
    if ((events >> 22) & 1) != 0 {
        lines.push("cantarget=yes".to_string());
    }
    for bit in 23_u32..=31 {
        if ((events >> bit) & 1) != 0 {
            lines.push(format!("event{bit}=yes"));
        }
    }

    if build < 499 {
        // nothing
    } else if build < 530 {
        let keycount = packet.g1()?;
        for index in 0..keycount {
            lines.push(format!("opkey{}={}", index + 1, packet.g1s()?));
        }
        if keycount > 0 && build >= 509 {
            let modcount = packet.g1()?;
            for index in 0..modcount {
                lines.push(format!("opkeymod{}={}", index + 1, packet.g1s()?));
            }
        }
    } else {
        let mut value = packet.g1()?;
        while value != 0 {
            let index = (value >> 4).checked_sub(1).ok_or_else(|| {
                CacheError::message(format!("invalid opkey index nibble {}", value >> 4))
            })?;
            let mut rate = (u16::from(value) << 8 | u16::from(packet.g1()?)) & 4095;
            if rate == 4095 {
                rate = u16::MAX;
            }
            let rate_print = if rate == u16::MAX {
                "-1".to_string()
            } else {
                rate.to_string()
            };
            lines.push(format!(
                "opkey{}={},{},{}",
                index + 1,
                rate_print,
                packet.g1s()?,
                packet.g1s()?
            ));
            value = packet.g1()?;
        }
    }

    let opbase = packet.gjstr()?;
    if !opbase.is_empty() {
        lines.push(format!("opbase={opbase}"));
    }

    let opinfo = packet.g1()?;
    let opcount = opinfo & 15;
    let opcursorcount = opinfo >> 4;

    for i in 0..opcount {
        let op = packet.gjstr()?;
        if !op.is_empty() {
            lines.push(format!("op{}={op}", i + 1));
        }
    }

    if opcursorcount > 0 {
        lines.push(format!(
            "opcursor{}={}",
            packet.g1()?,
            format_cursor(i32::from(packet.g2()?))
        ));
    }
    if opcursorcount > 1 {
        lines.push(format!(
            "opcursor{}={}",
            packet.g1()?,
            format_cursor(i32::from(packet.g2()?))
        ));
    }

    if build >= 537 {
        let pausetext = packet.gjstr()?;
        if !pausetext.is_empty() {
            lines.push(format!("pausetext={pausetext}"));
        }
    }

    let dragdeadzone = packet.g1()?;
    if dragdeadzone != 0 {
        lines.push(format!("dragdeadzone={dragdeadzone}"));
    }
    let dragdeadtime = packet.g1()?;
    if dragdeadtime != 0 {
        lines.push(format!("dragdeadtime={dragdeadtime}"));
    }
    let dragrenderbehaviour = packet.g1()?;
    if dragrenderbehaviour != 0 {
        lines.push(format!("dragrenderbehaviour={dragrenderbehaviour}"));
    }

    let targetverb = packet.gjstr()?;
    if !targetverb.is_empty() {
        lines.push(format!("targetverb={targetverb}"));
    }

    if build >= 530 && targetmask != 0 {
        let targetcursor1 = packet.g2null()?;
        if targetcursor1 != -1 {
            lines.push(format!("targetcursor1={}", format_cursor(targetcursor1)));
        }
        let targetcursor2 = packet.g2null()?;
        if targetcursor2 != -1 {
            lines.push(format!("targetcursor2={}", format_cursor(targetcursor2)));
        }
        let targetcursor3 = packet.g2null()?;
        if targetcursor3 != -1 {
            lines.push(format!("targetcursor3={}", format_cursor(targetcursor3)));
        }
    }

    if version >= 0 {
        let mouseover = packet.g2null()?;
        if mouseover != -1 {
            lines.push(format!("mouseovercursor={}", format_cursor(mouseover)));
        }
    }

    if version >= 0 {
        let intparamcount = packet.g1()?;
        for _ in 0..intparamcount {
            lines.push(format!(
                "param={},{}",
                format_param(packet.g3()?),
                packet.g4s()?
            ));
        }

        let stringparamcount = packet.g1()?;
        for _ in 0..stringparamcount {
            lines.push(format!(
                "param={},{}",
                format_param(packet.g3()?),
                packet.gjstr2()?
            ));
        }
    }

    push_hook(lines, "onload=", decode_hook(packet)?);
    push_hook(lines, "onmouseover=", decode_hook(packet)?);
    push_hook(lines, "onmouseleave=", decode_hook(packet)?);
    push_hook(lines, "ontargetleave=", decode_hook(packet)?);
    push_hook(lines, "ontargetenter=", decode_hook(packet)?);
    push_hook(lines, "onvartransmit=", decode_hook(packet)?);
    push_hook(lines, "oninvtransmit=", decode_hook(packet)?);
    push_hook(lines, "onstattransmit=", decode_hook(packet)?);
    push_hook(lines, "ontimer=", decode_hook(packet)?);
    push_hook(lines, "onop=", decode_hook(packet)?);

    if version >= 0 {
        push_hook(lines, "onopt=", decode_hook(packet)?);
    }

    push_hook(lines, "onmouserepeat=", decode_hook(packet)?);
    push_hook(lines, "onclick=", decode_hook(packet)?);
    push_hook(lines, "onclickrepeat=", decode_hook(packet)?);
    push_hook(lines, "onrelease=", decode_hook(packet)?);
    push_hook(lines, "onhold=", decode_hook(packet)?);
    push_hook(lines, "ondrag=", decode_hook(packet)?);
    push_hook(lines, "ondragcomplete=", decode_hook(packet)?);

    if build < 459 {
        push_hook(lines, "ondragcancel=", decode_hook(packet)?);
    }

    push_hook(lines, "onscrollwheel=", decode_hook(packet)?);

    if build >= 506 {
        push_hook(lines, "onvarctransmit=", decode_hook(packet)?);
        push_hook(lines, "onvarcstrtransmit=", decode_hook(packet)?);
    }

    if version >= 6 {
        push_hook(lines, "onbuttonclick=", decode_hook(packet)?);
        push_hook(lines, "onhook51=", decode_hook(packet)?);
        push_hook(lines, "onlistselect=", decode_hook(packet)?);
    }

    if version >= 8 {
        push_hook(lines, "onupdated=", decode_hook(packet)?);
    }

    if build >= 459 {
        push_transmit_list(
            lines,
            "onvartransmitlist=",
            decode_hook_transmit_list(packet, TransmitListType::VarPlayer)?,
        );
        push_transmit_list(
            lines,
            "oninvtransmitlist=",
            decode_hook_transmit_list(packet, TransmitListType::Inv)?,
        );
        push_transmit_list(
            lines,
            "onstattransmitlist=",
            decode_hook_transmit_list(packet, TransmitListType::Stat)?,
        );
    }

    if build >= 506 {
        push_transmit_list(
            lines,
            "onvarctransmitlist=",
            decode_hook_transmit_list(packet, TransmitListType::VarClient)?,
        );
        push_transmit_list(
            lines,
            "onvarcstrtransmitlist=",
            decode_hook_transmit_list(packet, TransmitListType::VarClientString)?,
        );
    }

    Ok(())
}

fn decode_hook(packet: &mut Packet<'_>) -> Result<Option<String>> {
    let count = usize::from(packet.g1()?);
    if count == 0 {
        return Ok(None);
    }

    let _unknown = packet.g1()?;
    let script = packet.g4s()?;
    let mut args = Vec::new();
    for _ in 0..(count - 1) {
        match packet.g1()? {
            0 => args.push(HookArgument::Int(packet.g4s()?)),
            1 => args.push(HookArgument::Str(packet.gjstr()?)),
            value => bail!("unexpected hook argument type {value}"),
        }
    }

    if args.is_empty() {
        Ok(Some(format_clientscript(script)))
    } else {
        let arg_text = args
            .iter()
            .map(format_hook_argument)
            .collect::<Vec<_>>()
            .join(", ");
        Ok(Some(format!("{}({arg_text})", format_clientscript(script))))
    }
}

#[derive(Clone, Debug)]
enum HookArgument {
    Int(i32),
    Str(String),
}

fn format_hook_argument(argument: &HookArgument) -> String {
    match argument {
        HookArgument::Str(value) => {
            if value == "event_opbase" || value == "event_text" {
                value.clone()
            } else {
                format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
            }
        }
        HookArgument::Int(value) => match *value {
            x if x == i32::MIN + 1 => "event_mousex".to_string(),
            x if x == i32::MIN + 2 => "event_mousey".to_string(),
            x if x == i32::MIN + 3 => "event_com".to_string(),
            x if x == i32::MIN + 4 => "event_op".to_string(),
            x if x == i32::MIN + 5 => "event_comsubid".to_string(),
            x if x == i32::MIN + 6 => "event_com2".to_string(),
            x if x == i32::MIN + 7 => "event_comsubid2".to_string(),
            x if x == i32::MIN + 8 => "event_keycode".to_string(),
            x if x == i32::MIN + 9 => "event_keychar".to_string(),
            x if x == i32::MIN + 10 => "event_gamepadvalue".to_string(),
            x if x == i32::MIN + 11 => "event_gamepadbutton".to_string(),
            value => value.to_string(),
        },
    }
}

#[derive(Clone, Copy, Debug)]
enum TransmitListType {
    VarPlayer,
    Inv,
    Stat,
    VarClient,
    VarClientString,
}

fn decode_hook_transmit_list(
    packet: &mut Packet<'_>,
    kind: TransmitListType,
) -> Result<Option<String>> {
    let count = usize::from(packet.g1()?);
    if count == 0 {
        return Ok(None);
    }

    let mut parts = Vec::with_capacity(count);
    for _ in 0..count {
        parts.push(format_transmit(kind, packet.g4s()?));
    }
    Ok(Some(parts.join(",")))
}

fn push_hook(lines: &mut Vec<String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        lines.push(format!("{key}{value}"));
    }
}

fn push_transmit_list(lines: &mut Vec<String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        lines.push(format!("{key}{value}"));
    }
}

fn format_transmit(kind: TransmitListType, id: i32) -> String {
    match kind {
        TransmitListType::VarPlayer => format!("varplayerint_{id}"),
        TransmitListType::Inv => format!("inv_{id}"),
        TransmitListType::Stat => format!("stat_{id}"),
        TransmitListType::VarClient => format!("varclientint_{id}"),
        TransmitListType::VarClientString => format!("varclientstring_{id}"),
    }
}

fn format_graphic(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("graphic_{id}")
    }
}

fn format_fontmetrics(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("fontmetrics_{id}")
    }
}

fn format_clientscript(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("script{id}")
    }
}

fn format_cursor(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("cursor_{id}")
    }
}

fn format_param(id: u32) -> String {
    format!("param_{id}")
}

fn format_stylesheet(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("stylesheet_{id}")
    }
}

fn format_model(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("model_{id}")
    }
}

fn format_seq(id: i32) -> String {
    if id == -1 {
        "null".to_string()
    } else {
        format!("seq_{id}")
    }
}

fn format_colour(colour: i32) -> String {
    let hex = format!("{:x}", colour as u32);
    if hex.len() > 6 {
        format!("0x{hex:0>8}")
    } else {
        format!("0x{hex:0>6}")
    }
}

fn format_if_type(if_type: i32) -> &'static str {
    match if_type {
        0 => "layer",
        3 => "rectangle",
        4 => "text",
        5 => "graphic",
        6 => "model",
        9 => "line",
        10 => "button",
        11 => "panel",
        12 => "check",
        13 => "input",
        14 => "slider",
        15 => "grid",
        16 => "list",
        17 => "combo",
        18 => "pagedlayer",
        19 => "pagedlayerheader",
        20 => "carousel",
        21 => "pagedcarousel",
        22 => "radiogroup",
        23 => "groupbox",
        24 => "radialprogressoverlay",
        26 => "crmview",
        27 => "cutscenelayer",
        28 => "modelgroup",
        _ => "unknown",
    }
}

fn decode_size_mode(value: i8) -> &'static str {
    match value {
        0 => "abs",
        1 => "minus",
        2 => "proportion",
        3 => "mode_3",
        4 => "aspect",
        _ => "mode_unknown",
    }
}

fn decode_x_mode(value: i8) -> &'static str {
    match value {
        0 => "abs_left",
        1 => "abs_centre",
        2 => "abs_right",
        3 => "proportion_left",
        4 => "proportion_centre",
        5 => "proportion_right",
        _ => "mode_unknown",
    }
}

fn decode_y_mode(value: i8) -> &'static str {
    match value {
        0 => "abs_top",
        1 => "abs_centre",
        2 => "abs_bottom",
        3 => "proportion_top",
        4 => "proportion_centre",
        5 => "proportion_bottom",
        _ => "mode_unknown",
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(tag = "domain", content = "id")]
pub enum VarTransmitRef {
    Player(u32),
    Npc(u32),
    Client(u32),
    World(u32),
    Region(u32),
    Object(u32),
    Clan(u32),
    ClanSetting(u32),
    Controller(u32),
    Global(u32),
    PlayerGroup(u32),
    VarClientString(u32),
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentDeps {
    pub component_type: String,
    pub name: Option<String>,
    pub children: Vec<u32>,
    pub scripts: HashSet<u32>,
    /// Script ids from the component `onload` hook only.
    pub onload_scripts: HashSet<u32>,
    pub varps: HashSet<VarTransmitRef>,
    pub varbits: HashSet<u32>,
    pub invs: HashSet<u32>,
    pub stats: HashSet<u32>,
    pub graphics: HashSet<u32>,
    pub models: HashSet<u32>,
    pub cursors: HashSet<u32>,
    pub stylesheets: HashSet<u32>,
    pub params: HashSet<u32>,
    pub seqs: HashSet<u32>,
    pub fontmetrics: HashSet<u32>,
    pub textures: HashSet<u32>,
    pub enums: HashSet<u32>,
}

pub fn parse_component_deps(component_id: u32, data: &[u8], build: u32) -> Result<ComponentDeps> {
    let mut packet = Packet::new(data);
    let mut deps = ComponentDeps {
        component_type: "unknown".to_string(),
        name: None,
        children: Vec::new(),
        scripts: HashSet::new(),
        onload_scripts: HashSet::new(),
        varps: HashSet::new(),
        varbits: HashSet::new(),
        invs: HashSet::new(),
        stats: HashSet::new(),
        graphics: HashSet::new(),
        models: HashSet::new(),
        cursors: HashSet::new(),
        stylesheets: HashSet::new(),
        params: HashSet::new(),
        seqs: HashSet::new(),
        fontmetrics: HashSet::new(),
        textures: HashSet::new(),
        enums: HashSet::new(),
    };

    let mut version = i16::from(packet.g1()?);
    if build < 566 && version != i16::from(u8::MAX) {
        return Ok(deps);
    }
    if version == i16::from(u8::MAX) {
        version = -1;
    }

    let mut if_type = packet.g1()?;
    deps.component_type = format_if_type(i32::from(if_type & 0x7F)).to_string();

    if (if_type & 128) != 0 {
        if_type &= 127;
        let name = packet.gjstr()?;
        if !name.is_empty() {
            deps.name = Some(name);
        }
    }

    let _contenttype = packet.g2()?;
    let _x = packet.g2s()?;
    let _y = packet.g2s()?;
    let _width = packet.g2()?;
    let _height = packet.g2()?;

    let mut width_mode: i8 = 0;
    let mut height_mode: i8 = 0;
    if build >= 493 {
        width_mode = packet.g1s()?;
        height_mode = packet.g1s()?;
        let _x_mode = packet.g1s()?;
        let _y_mode = packet.g1s()?;
    }

    if width_mode == 4 || height_mode == 4 {
        let _aspect_width = packet.g2()?;
        let _aspect_height = packet.g2()?;
    }

    let layer = packet.g2null()?;
    if layer != -1 {
        deps.children.push(layer as u32);
    }

    let _flags = packet.g1()?;

    // Use a closure to catch errors and return partial results
    let parse_result = (|| -> Result<()> {
        let parsed_body = match if_type {
            6 => {
                collect_model_deps(
                    &mut deps,
                    &mut packet,
                    build,
                    width_mode != 0,
                    height_mode != 0,
                )?;
                true
            }
            0 => {
                let _scroll_width = packet.g2()?;
                let _scroll_height = packet.g2()?;
                if version == -1 && build >= 495 {
                    let _ = packet.g1()?;
                } else if version >= 9 || version >= 6 {
                    if version >= 9 {
                        let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                    } else {
                        let _ = [packet.g2()?, packet.g2()?, packet.g2()?, packet.g2()?];
                    }
                }
                true
            }
            3 => {
                let _ = packet.g4s()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                true
            }
            4 => {
                collect_text_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            5 => {
                collect_graphic_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            9 => {
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                if build >= 493 {
                    let _ = packet.g1()?;
                }
                true
            }
            10 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            11 => {
                let _ = packet.g2()?;
                let _ = packet.g2()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                true
            }
            12 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            13 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g2()?;
                if version >= 9 {
                    let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                }
                if version >= 7 {
                    let _ = packet.g1()?;
                }
                let _ = packet.g1()?;
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                collect_scrollbar_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            15 => {
                let _ = packet.g2()?;
                let _ = packet.g2()?;
                let _ = packet.g1()?;
                let _ = packet.g2()?;
                let _ = packet.g2()?;
                let _ = packet.g1()?;
                true
            }
            16 => {
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                if version >= 9 {
                    let _ = packet.g1()?;
                }
                let _ = packet.g1()?;
                let _ = packet.g1()?;
                for _ in 0..packet.g2()? {
                    let _ = packet.gjstr()?;
                }
                for _ in 0..packet.g2()? {
                    let _ = packet.g4s()?;
                }
                for _ in 0..packet.g2()? {
                    let _ = packet.g2()?;
                }
                if version >= 9 {
                    let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                    let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
                }
                let _ = packet.g4s()?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_sprite_part_deps(&mut deps, &mut packet, version, build)?;
                collect_text_part_deps(&mut deps, &mut packet, version, build)?;
                collect_scrollbar_deps(&mut deps, &mut packet, version, build)?;
                true
            }
            26 => {
                let _ = packet.g1()?;
                for _ in 0..packet.g2()? {
                    let _ = packet.g2()?;
                }
                let _ = packet.gjstr()?;
                let _ = packet.g1()?;
                for _ in 0..packet.g2()? {
                    let _ = packet.gjstr()?;
                }
                for _ in 0..packet.g2()? {
                    let _ = packet.g4s()?;
                }
                true
            }
            _ => false,
        };

        if parsed_body {
            collect_common_tail_deps(&mut deps, &mut packet, version, build)?;
        }

        Ok(())
    })();

    // Return partial results even if parsing failed
    if let Err(e) = parse_result {
        eprintln!(
            "parse_component_deps partial failure for comp {component_id} (type={}): {e}",
            deps.component_type
        );
    }

    Ok(deps)
}

fn collect_text_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    collect_text_part_deps(deps, packet, version, build)
}

fn collect_graphic_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    collect_sprite_part_deps(deps, packet, version, build)
}

fn collect_model_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    build: u32,
    has_width_mode: bool,
    has_height_mode: bool,
) -> Result<()> {
    let model = if build < 681 {
        packet.g2null()?
    } else {
        packet.gsmart2or4null()?
    };
    if model != -1 {
        deps.models.insert(model as u32);
    }

    if build < 619 {
        let _ = packet.g2s()?;
        let _ = packet.g2s()?;
        let _ = packet.g2()?;
        let _ = packet.g2()?;
        let _ = packet.g2()?;
        let _ = packet.g2()?;
        let modelanim = if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        };
        if modelanim != -1 {
            deps.seqs.insert(modelanim as u32);
        }
        let _ = packet.g1()?;
        if build >= 493 {
            let _ = packet.g2()?;
        }
        if build >= 501 {
            let _ = packet.g2()?;
            let _ = packet.g1()?;
        }
    } else {
        let model_flags = packet.g1()?;
        let has_transform = (model_flags & 1) != 0;
        let has_precise_zoom = (model_flags & 2) != 0;
        if has_transform {
            let _ = packet.g2s()?;
            let _ = packet.g2s()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
        } else if has_precise_zoom {
            let _ = packet.g2s()?;
            let _ = packet.g2s()?;
            let _ = packet.g2s()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
            let _ = packet.g2()?;
        }
        let modelanim = if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        };
        if modelanim != -1 {
            deps.seqs.insert(modelanim as u32);
        }
    }

    if has_width_mode {
        let _ = packet.g2()?;
    }
    if has_height_mode {
        let _ = packet.g2()?;
    }

    Ok(())
}

fn collect_text_part_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    let textfont = if build < 800 {
        if build < 681 {
            packet.g2null()?
        } else {
            packet.gsmart2or4null()?
        }
    } else {
        packet.gsmart2or4null()?
    };
    if textfont != -1 {
        deps.fontmetrics.insert(textfont as u32);
    }

    if version >= 2 {
        let _ = packet.g1()?;
    }
    let _ = packet.gjstr()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g4s()?;
    if build >= 582 {
        let _ = packet.g1()?;
    }
    if version >= 0 {
        let _ = packet.g1()?;
    }
    Ok(())
}

fn collect_sprite_part_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    let graphic = packet.g4s()?;
    if graphic != -1 {
        deps.graphics.insert(graphic as u32);
    }
    let _ = packet.g2()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g4s()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    if build >= 537 {
        let _ = packet.g4s()?;
    }
    if version >= 3 {
        let _ = packet.g1()?;
    }
    if version >= 6 {
        let _ = [packet.g1()?, packet.g1()?, packet.g1()?, packet.g1()?];
    }
    Ok(())
}

fn collect_scrollbar_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;
    collect_sprite_part_deps(deps, packet, version, build)?;
    collect_sprite_part_deps(deps, packet, version, build)?;
    collect_sprite_part_deps(deps, packet, version, build)?;
    Ok(())
}

fn collect_common_tail_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    version: i16,
    build: u32,
) -> Result<()> {
    if version >= 6 {
        let stylesheet = packet.g4s()?;
        if stylesheet != -1 {
            deps.stylesheets.insert(stylesheet as u32);
        }
    }

    if version >= 9 {
        let _ = packet.g1()?;
    }

    let events = if version < 6 {
        packet.g3()?
    } else {
        packet.g4s()? as u32
    };

    let targetmask = (events >> 11) & 0x7F;

    if build < 499 {
    } else if build < 530 {
        let keycount = packet.g1()?;
        for _ in 0..keycount {
            let _ = packet.g1s()?;
        }
        if keycount > 0 && build >= 509 {
            let modcount = packet.g1()?;
            for _ in 0..modcount {
                let _ = packet.g1s()?;
            }
        }
    } else {
        let mut value = packet.g1()?;
        while value != 0 {
            let _index = (value >> 4).checked_sub(1);
            let _ = packet.g1()?;
            let _ = packet.g1s()?;
            let _ = packet.g1s()?;
            value = packet.g1()?;
        }
    }

    let _opbase = packet.gjstr()?;

    let opinfo = packet.g1()?;
    let opcount = opinfo & 15;
    let opcursorcount = opinfo >> 4;

    for _ in 0..opcount {
        let _ = packet.gjstr()?;
    }

    if opcursorcount > 0 {
        let _ = packet.g1()?;
        let cursor = packet.g2()?;
        if cursor != 0xFFFF {
            deps.cursors.insert(u32::from(cursor));
        }
    }
    if opcursorcount > 1 {
        let _ = packet.g1()?;
        let cursor = packet.g2()?;
        if cursor != 0xFFFF {
            deps.cursors.insert(u32::from(cursor));
        }
    }

    if build >= 537 {
        let _ = packet.gjstr()?;
    }

    let _ = packet.g1()?;
    let _ = packet.g1()?;
    let _ = packet.g1()?;

    let _targetverb = packet.gjstr()?;

    if build >= 530 && targetmask != 0 {
        let _ = packet.g2null()?;
        let _ = packet.g2null()?;
        let _ = packet.g2null()?;
    }

    if version >= 0 {
        let _ = packet.g2null()?;
    }

    if version >= 0 {
        let intparamcount = packet.g1()?;
        for _ in 0..intparamcount {
            let param_id = packet.g3()?;
            deps.params.insert(param_id);
            let _ = packet.g4s()?;
        }

        let stringparamcount = packet.g1()?;
        for _ in 0..stringparamcount {
            let param_id = packet.g3()?;
            deps.params.insert(param_id);
            let _ = packet.gjstr2()?;
        }
    }

    collect_onload_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;

    if version >= 0 {
        collect_hook_deps(deps, packet)?;
    }

    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;
    collect_hook_deps(deps, packet)?;

    if build < 459 {
        collect_hook_deps(deps, packet)?;
    }

    collect_hook_deps(deps, packet)?;

    if build >= 506 {
        collect_hook_deps(deps, packet)?;
        collect_hook_deps(deps, packet)?;
    }

    if version >= 6 {
        collect_hook_deps(deps, packet)?;
        collect_hook_deps(deps, packet)?;
        collect_hook_deps(deps, packet)?;
    }

    if version >= 8 {
        collect_hook_deps(deps, packet)?;
    }

    if build >= 459 {
        collect_transmit_list_deps(deps, packet, TransmitListType::VarPlayer)?;
        collect_transmit_list_deps(deps, packet, TransmitListType::Inv)?;
        collect_transmit_list_deps(deps, packet, TransmitListType::Stat)?;
    }

    if build >= 506 {
        collect_transmit_list_deps(deps, packet, TransmitListType::VarClient)?;
        collect_transmit_list_deps(deps, packet, TransmitListType::VarClientString)?;
    }

    Ok(())
}

fn collect_onload_hook_deps(deps: &mut ComponentDeps, packet: &mut Packet<'_>) -> Result<()> {
    collect_hook_deps_inner(deps, packet, true)
}

fn collect_hook_deps(deps: &mut ComponentDeps, packet: &mut Packet<'_>) -> Result<()> {
    collect_hook_deps_inner(deps, packet, false)
}

fn collect_hook_deps_inner(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    onload: bool,
) -> Result<()> {
    let count = usize::from(packet.g1()?);
    if count == 0 {
        return Ok(());
    }

    let _unknown = packet.g1()?;
    let script = packet.g4s()?;
    if script != -1 {
        let script_id = script as u32;
        deps.scripts.insert(script_id);
        if onload {
            deps.onload_scripts.insert(script_id);
        }
    }
    for _ in 0..(count - 1) {
        match packet.g1()? {
            0 => {
                let _ = packet.g4s()?;
            }
            1 => {
                let _ = packet.gjstr()?;
            }
            value => bail!("unexpected hook argument type {value}"),
        }
    }

    Ok(())
}

fn collect_transmit_list_deps(
    deps: &mut ComponentDeps,
    packet: &mut Packet<'_>,
    kind: TransmitListType,
) -> Result<()> {
    let count = usize::from(packet.g1()?);
    if count == 0 {
        return Ok(());
    }

    for _ in 0..count {
        let id = packet.g4s()?;
        match kind {
            TransmitListType::VarPlayer => {
                deps.varps.insert(VarTransmitRef::Player(id as u32));
            }
            TransmitListType::Inv => {
                deps.invs.insert(id as u32);
            }
            TransmitListType::Stat => {
                deps.stats.insert(id as u32);
            }
            TransmitListType::VarClient => {
                deps.varps.insert(VarTransmitRef::Client(id as u32));
            }
            TransmitListType::VarClientString => {
                deps.varps
                    .insert(VarTransmitRef::VarClientString(id as u32));
            }
        }
    }
    Ok(())
}
