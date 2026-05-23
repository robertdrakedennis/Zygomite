use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Vfx {
    pub version: u8,
    pub name: String,
    pub unknown2: u8,
    pub emitters: Vec<ModularParticleEmitter>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModularParticleEmitter {
    pub name: String,
    pub unknown1: u8,
    pub unknown2: u8,
    pub unknown3: u8,
    pub unknown4: u8,
    pub material: u16,
    #[serde(rename = "maxParticles")]
    pub max_particles: u16,
    #[serde(rename = "numTiles")]
    pub num_tiles: u8,
    pub unknown6: u8,
    pub unknown7: f32,
    pub unknown8: f32,
    pub unknown9: f32,
    pub unknown10: f32,
    pub unknown11: f32,
    pub unknown12: u8,
    #[serde(rename = "warmupTime")]
    pub warmup_time: i32,
    pub unknown13: f32,
    pub lifetime: f32,
    pub position: Vector3,
    pub rotation: Vector3,
    pub modules: Vec<Module>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
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

#[derive(Clone, Debug, Serialize)]
pub struct FloatCurve {
    pub keyframes: Vec<FloatCurveKeyFrame>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FloatCurveKeyFrame {
    pub time1: f32,
    pub value1: f32,
    pub time2: f32,
    pub value2: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vec2Curve {
    pub keyframes: Vec<Vec2CurveKeyFrame>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vec2CurveKeyFrame {
    pub time1: f32,
    pub value1: Vector2,
    pub time2: f32,
    pub value2: Vector2,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vec3Curve {
    pub keyframes: Vec<Vec3CurveKeyFrame>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vec3CurveKeyFrame {
    pub time1: f32,
    pub value1: Vector3,
    pub time2: f32,
    pub value2: Vector3,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vec4Curve {
    pub keyframes: Vec<Vec4CurveKeyFrame>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vec4CurveKeyFrame {
    pub time1: f32,
    pub value1: Vector4,
    pub time2: f32,
    pub value2: Vector4,
}

#[derive(Clone, Debug, Serialize)]
pub struct FlowShape {
    pub kind: FlowShapeType,
    pub unknown1: bool,
    pub position: Vector3,
    pub rotation: Vector3,
    pub length: f32,
    pub height: f32,
    pub width: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowShapeType {
    None,
    Line,
    Plane,
    Box,
    Disc,
    Cone,
    Sphere,
    Hemisphere,
}

#[derive(Clone, Debug, Serialize)]
pub struct Unknown {
    pub unknown1: u8,
    pub unknown2: f32,
    pub unknown3: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Module {
    Flow {
        flags: u8,
        shape1: FlowShape,
        shape2: FlowShape,
        unknown3: f32,
        #[serde(rename = "spawnRateNum")]
        spawn_rate_num: f32,
        #[serde(rename = "spawnRateDenom")]
        spawn_rate_denom: f32,
        unknown10: f32,
        #[serde(rename = "lifetimeStart")]
        lifetime_start: f32,
        #[serde(rename = "lifetimeEnd")]
        lifetime_end: f32,
        unknown8: f32,
        unknown9: f32,
        unknown11: f32,
        unknown12: f32,
    },
    Colour {
        unknown0: Vec3Curve,
        unknown1: FloatCurve,
        unknown2: Vec3Curve,
        unknown3: FloatCurve,
    },
    Gravity {
        unknown0: FloatCurve,
        unknown1: FloatCurve,
    },
    TriggerPlane {
        unknown0: u8,
        unknown1: FloatCurve,
        unknown2: Vec3Curve,
        unknown3: Vec3Curve,
    },
    Attractor {
        position: Vec3Curve,
        strength: FloatCurve,
    },
    Rotation {
        unknown0: bool,
        unknown1: f32,
        unknown2: f32,
        unknown3: FloatCurve,
        unknown4: FloatCurve,
    },
    Scale {
        unknown0: bool,
        unknown1: f32,
        unknown2: f32,
        unknown3: f32,
        unknown4: f32,
        unknown5: FloatCurve,
        unknown6: FloatCurve,
        unknown7: FloatCurve,
        unknown8: FloatCurve,
    },
    Noise {
        unknown0: FloatCurve,
        unknown1: FloatCurve,
    },
    Flipbook {
        #[serde(rename = "frameRate")]
        frame_rate: u8,
    },
    Acceleration {
        unknown0: f32,
        unknown1: f32,
        unknown2: FloatCurve,
        unknown3: FloatCurve,
    },
    Lighting {
        unknown0: u8,
        unknown1: u8,
        unknown2: u8,
    },
    Unknown11 {
        unknown1: f32,
        unknown2: f32,
        unknown3: f32,
        unknown4: f32,
        unknown5: u8,
    },
    Unknown12 {
        unknown1: u8,
        unknown2: Unknown,
        unknown3: Vec3Curve,
    },
    Unknown13 {
        unknown1: f32,
        unknown2: f32,
        unknown3: f32,
        unknown4: f32,
        unknown5: u8,
        unknown6: Vec4Curve,
        unknown7: FloatCurve,
    },
    Unknown14 {
        unknown1: Unknown,
    },
    Unknown15 {
        unknown1: Unknown,
        unknown2: FloatCurve,
        unknown3: FlowShape,
        unknown4: f32,
        unknown5: f32,
        unknown6: u8,
    },
    Unknown16 {
        unknown1: Unknown,
        unknown2: FloatCurve,
        unknown3: Unknown,
        unknown4: FloatCurve,
        unknown5: Vector3,
    },
    Unknown17 {
        unknown1: u8,
        unknown2: Unknown,
        unknown3: Vec2Curve,
    },
}

pub fn decode(data: &[u8]) -> Result<Vfx> {
    let mut packet = Packet::new(data);
    let version = packet.g1()?;
    let name = packet.gjstr2()?;
    let unknown2 = packet.g1()?;
    let emitter_count = usize::from(packet.g1()?);
    let mut emitters = Vec::with_capacity(emitter_count);
    for _ in 0..emitter_count {
        emitters.push(decode_emitter(&mut packet, version)?);
    }
    Ok(Vfx {
        version,
        name,
        unknown2,
        emitters,
    })
}

fn decode_emitter(packet: &mut Packet<'_>, version: u8) -> Result<ModularParticleEmitter> {
    let name = packet.gjstr2()?;

    let (emitter_mode_a, emitter_mode_b) = if version >= 8 {
        (packet.g1()?, packet.g1()?)
    } else {
        (0, 0)
    };

    let emitter_flag_a = packet.g1()?;
    let emitter_flag_b = if version >= 2 { packet.g1()? } else { 0 };
    let material = packet.g2()?;
    let max_particles = packet.g2()?;
    let (num_tiles, tile_ratio) = if version < 4 {
        let a = packet.g1()?;
        let b = packet.g1()?;
        if b == 0 {
            bail!("vfx emitter {name} num_tiles is zero");
        }
        (b, a / b)
    } else {
        (packet.g1()?, packet.g1()?)
    };

    let (scalar7, scalar8, scalar9, scalar10, scalar11, flag12) = if version >= 8 {
        (
            read_f32_be(packet)?,
            read_f32_be(packet)?,
            read_f32_be(packet)?,
            read_f32_be(packet)?,
            read_f32_be(packet)?,
            packet.g1()?,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0, 0.0, 0)
    };

    let warmup_time = packet.g4s()?;
    let scalar13 = if version >= 8 {
        read_f32_be(packet)?
    } else {
        0.0
    };
    let lifetime = read_f32_be(packet)?;
    let position = read_vec3(packet)?;
    let rotation = read_vec3(packet)?;

    let module_count = usize::from(packet.g1()?);
    let mut modules = Vec::with_capacity(module_count);
    for _ in 0..module_count {
        modules.push(decode_module(packet, version)?);
    }

    Ok(ModularParticleEmitter {
        name,
        unknown1: emitter_flag_a,
        unknown2: emitter_flag_b,
        unknown3: emitter_mode_a,
        unknown4: emitter_mode_b,
        material,
        max_particles,
        num_tiles,
        unknown6: tile_ratio,
        unknown7: scalar7,
        unknown8: scalar8,
        unknown9: scalar9,
        unknown10: scalar10,
        unknown11: scalar11,
        unknown12: flag12,
        warmup_time,
        unknown13: scalar13,
        lifetime,
        position,
        rotation,
        modules,
    })
}

fn decode_module(packet: &mut Packet<'_>, version: u8) -> Result<Module> {
    let module_type = packet.g1()?;
    match module_type {
        0 => {
            let flags = packet.g1()?;
            let shape1 = decode_flow_shape(packet)?;
            let shape2 = decode_flow_shape(packet)?;
            let unknown3 = if version < 8 {
                read_f32_be(packet)?
            } else {
                0.0
            };
            let spawn_rate_num = read_f32_be(packet)?;
            let spawn_rate_denom = read_f32_be(packet)?;
            let unknown10 = if version >= 3 {
                read_f32_be(packet)?
            } else {
                0.0
            };
            let unknown12 = if version >= 7 {
                read_f32_be(packet)?
            } else {
                0.0
            };
            let lifetime_start = read_f32_be(packet)?;
            let lifetime_end = read_f32_be(packet)?;
            let unknown8 = read_f32_be(packet)?;
            let unknown9 = read_f32_be(packet)?;
            let unknown11 = if version >= 3 {
                read_f32_be(packet)?
            } else {
                0.0
            };
            Ok(Module::Flow {
                flags,
                shape1,
                shape2,
                unknown3,
                spawn_rate_num,
                spawn_rate_denom,
                unknown10,
                lifetime_start,
                lifetime_end,
                unknown8,
                unknown9,
                unknown11,
                unknown12,
            })
        }
        1 => Ok(Module::Colour {
            unknown0: decode_vec3_curve(packet)?,
            unknown1: decode_float_curve(packet)?,
            unknown2: decode_vec3_curve(packet)?,
            unknown3: decode_float_curve(packet)?,
        }),
        2 => Ok(Module::Gravity {
            unknown0: decode_float_curve(packet)?,
            unknown1: decode_float_curve(packet)?,
        }),
        3 => Ok(Module::TriggerPlane {
            unknown0: packet.g1()?,
            unknown1: decode_float_curve(packet)?,
            unknown2: decode_vec3_curve(packet)?,
            unknown3: decode_vec3_curve(packet)?,
        }),
        4 => Ok(Module::Attractor {
            position: decode_vec3_curve(packet)?,
            strength: decode_float_curve(packet)?,
        }),
        5 => {
            let unknown0 = if version < 8 {
                packet.g1()? == 1
            } else {
                false
            };
            Ok(Module::Rotation {
                unknown0,
                unknown1: read_f32_be(packet)?,
                unknown2: read_f32_be(packet)?,
                unknown3: decode_float_curve(packet)?,
                unknown4: decode_float_curve(packet)?,
            })
        }
        6 => Ok(Module::Scale {
            unknown0: packet.g1()? == 1,
            unknown1: read_f32_be(packet)?,
            unknown2: read_f32_be(packet)?,
            unknown3: read_f32_be(packet)?,
            unknown4: read_f32_be(packet)?,
            unknown5: decode_float_curve(packet)?,
            unknown6: decode_float_curve(packet)?,
            unknown7: decode_float_curve(packet)?,
            unknown8: decode_float_curve(packet)?,
        }),
        7 => Ok(Module::Noise {
            unknown0: decode_float_curve(packet)?,
            unknown1: decode_float_curve(packet)?,
        }),
        8 => Ok(Module::Flipbook {
            frame_rate: packet.g1()?,
        }),
        9 => Ok(Module::Acceleration {
            unknown0: read_f32_be(packet)?,
            unknown1: read_f32_be(packet)?,
            unknown2: decode_float_curve(packet)?,
            unknown3: decode_float_curve(packet)?,
        }),
        10 => Ok(Module::Lighting {
            unknown0: packet.g1()?,
            unknown1: packet.g1()?,
            unknown2: packet.g1()?,
        }),
        11 => Ok(Module::Unknown11 {
            unknown1: read_f32_be(packet)?,
            unknown2: read_f32_be(packet)?,
            unknown3: read_f32_be(packet)?,
            unknown4: read_f32_be(packet)?,
            unknown5: packet.g1()?,
        }),
        12 => Ok(Module::Unknown12 {
            unknown1: packet.g1()?,
            unknown2: decode_unknown(packet)?,
            unknown3: decode_vec3_curve(packet)?,
        }),
        13 => Ok(Module::Unknown13 {
            unknown1: read_f32_be(packet)?,
            unknown2: read_f32_be(packet)?,
            unknown3: read_f32_be(packet)?,
            unknown4: read_f32_be(packet)?,
            unknown5: packet.g1()?,
            unknown6: decode_vec4_curve(packet)?,
            unknown7: decode_float_curve(packet)?,
        }),
        14 => Ok(Module::Unknown14 {
            unknown1: decode_unknown(packet)?,
        }),
        15 => Ok(Module::Unknown15 {
            unknown1: decode_unknown(packet)?,
            unknown2: decode_float_curve(packet)?,
            unknown3: decode_flow_shape(packet)?,
            unknown4: read_f32_be(packet)?,
            unknown5: read_f32_be(packet)?,
            unknown6: packet.g1()?,
        }),
        16 => Ok(Module::Unknown16 {
            unknown1: decode_unknown(packet)?,
            unknown2: decode_float_curve(packet)?,
            unknown3: decode_unknown(packet)?,
            unknown4: decode_float_curve(packet)?,
            unknown5: read_vec3(packet)?,
        }),
        17 => Ok(Module::Unknown17 {
            unknown1: packet.g1()?,
            unknown2: decode_unknown(packet)?,
            unknown3: decode_vec2_curve(packet)?,
        }),
        value => bail!("vfx unknown module type {value}"),
    }
}

fn decode_unknown(packet: &mut Packet<'_>) -> Result<Unknown> {
    Ok(Unknown {
        unknown1: packet.g1()?,
        unknown2: read_f32_be(packet)?,
        unknown3: read_f32_be(packet)?,
    })
}

fn decode_flow_shape(packet: &mut Packet<'_>) -> Result<FlowShape> {
    let kind = match packet.g1()? {
        0 => FlowShapeType::None,
        1 => FlowShapeType::Line,
        2 => FlowShapeType::Plane,
        3 => FlowShapeType::Box,
        4 => FlowShapeType::Disc,
        5 => FlowShapeType::Cone,
        6 => FlowShapeType::Sphere,
        7 => FlowShapeType::Hemisphere,
        value => bail!("vfx unknown flow shape kind {value}"),
    };

    let unknown1 = packet.g1()? == 1;
    let position = read_vec3(packet)?;
    let rotation = read_vec3(packet)?;
    let mut length = 0.0_f32;
    let mut height = 0.0_f32;
    let mut width = 0.0_f32;

    match kind {
        FlowShapeType::None => {}
        FlowShapeType::Line => {
            length = read_f32_be(packet)?;
        }
        FlowShapeType::Plane => {
            width = read_f32_be(packet)?;
            length = read_f32_be(packet)?;
        }
        FlowShapeType::Box => {
            width = read_f32_be(packet)?;
            height = read_f32_be(packet)?;
            length = read_f32_be(packet)?;
        }
        FlowShapeType::Disc | FlowShapeType::Sphere | FlowShapeType::Hemisphere => {
            length = read_f32_be(packet)?;
        }
        FlowShapeType::Cone => {
            height = read_f32_be(packet)?;
            length = read_f32_be(packet)?;
            width = read_f32_be(packet)?;
        }
    }

    Ok(FlowShape {
        kind,
        unknown1,
        position,
        rotation,
        length,
        height,
        width,
    })
}

fn decode_float_curve(packet: &mut Packet<'_>) -> Result<FloatCurve> {
    let frame_count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(frame_count);
    if frame_count == 1 {
        keyframes.push(FloatCurveKeyFrame {
            time1: read_f32_be(packet)?,
            value1: read_f32_be(packet)?,
            time2: 0.0,
            value2: 0.0,
        });
    } else {
        for _ in 0..frame_count {
            keyframes.push(FloatCurveKeyFrame {
                time1: read_f32_be(packet)?,
                value1: read_f32_be(packet)?,
                time2: read_f32_be(packet)?,
                value2: read_f32_be(packet)?,
            });
        }
    }
    Ok(FloatCurve { keyframes })
}

fn decode_vec2_curve(packet: &mut Packet<'_>) -> Result<Vec2Curve> {
    let frame_count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(frame_count);
    if frame_count == 1 {
        keyframes.push(Vec2CurveKeyFrame {
            time1: read_f32_be(packet)?,
            value1: read_vec2(packet)?,
            time2: 0.0,
            value2: Vector2 { x: 0.0, y: 0.0 },
        });
    } else {
        for _ in 0..frame_count {
            keyframes.push(Vec2CurveKeyFrame {
                time1: read_f32_be(packet)?,
                value1: read_vec2(packet)?,
                time2: read_f32_be(packet)?,
                value2: read_vec2(packet)?,
            });
        }
    }
    Ok(Vec2Curve { keyframes })
}

fn decode_vec3_curve(packet: &mut Packet<'_>) -> Result<Vec3Curve> {
    let frame_count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(frame_count);
    if frame_count == 1 {
        keyframes.push(Vec3CurveKeyFrame {
            time1: read_f32_be(packet)?,
            value1: read_vec3(packet)?,
            time2: 0.0,
            value2: Vector3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        });
    } else {
        for _ in 0..frame_count {
            keyframes.push(Vec3CurveKeyFrame {
                time1: read_f32_be(packet)?,
                value1: read_vec3(packet)?,
                time2: read_f32_be(packet)?,
                value2: read_vec3(packet)?,
            });
        }
    }
    Ok(Vec3Curve { keyframes })
}

fn decode_vec4_curve(packet: &mut Packet<'_>) -> Result<Vec4Curve> {
    let frame_count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(frame_count);
    if frame_count == 1 {
        keyframes.push(Vec4CurveKeyFrame {
            time1: read_f32_be(packet)?,
            value1: read_vec4(packet)?,
            time2: 0.0,
            value2: Vector4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        });
    } else {
        for _ in 0..frame_count {
            keyframes.push(Vec4CurveKeyFrame {
                time1: read_f32_be(packet)?,
                value1: read_vec4(packet)?,
                time2: read_f32_be(packet)?,
                value2: read_vec4(packet)?,
            });
        }
    }
    Ok(Vec4Curve { keyframes })
}

fn read_vec2(packet: &mut Packet<'_>) -> Result<Vector2> {
    Ok(Vector2 {
        x: read_f32_be(packet)?,
        y: read_f32_be(packet)?,
    })
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
    use super::{Module, decode};

    #[test]
    fn decodes_minimal_vfx_emitter() {
        let mut bytes = Vec::new();
        push_u8(&mut bytes, 8);
        push_jstr2(&mut bytes, "vfx");
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 1);

        push_jstr2(&mut bytes, "emitter");
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 2);
        push_u8(&mut bytes, 3);
        push_u8(&mut bytes, 4);
        push_u16_be(&mut bytes, 5);
        push_u16_be(&mut bytes, 6);
        push_u8(&mut bytes, 7);
        push_u8(&mut bytes, 8);
        push_f32_be(&mut bytes, 1.0);
        push_f32_be(&mut bytes, 2.0);
        push_f32_be(&mut bytes, 3.0);
        push_f32_be(&mut bytes, 4.0);
        push_f32_be(&mut bytes, 5.0);
        push_u8(&mut bytes, 9);
        push_i32_be(&mut bytes, 10);
        push_f32_be(&mut bytes, 6.0);
        push_f32_be(&mut bytes, 7.0);
        push_f32_be(&mut bytes, 8.0);
        push_f32_be(&mut bytes, 9.0);
        push_f32_be(&mut bytes, 10.0);
        push_f32_be(&mut bytes, 11.0);
        push_f32_be(&mut bytes, 12.0);
        push_f32_be(&mut bytes, 13.0);
        push_u8(&mut bytes, 0);

        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("vfx decode should succeed: {error}"),
        };
        assert_eq!(8, decoded.version);
        assert_eq!(1, decoded.emitters.len());
        assert_eq!("emitter", decoded.emitters[0].name);
        assert_eq!(0, decoded.emitters[0].modules.len());
    }

    #[test]
    fn decodes_flipbook_module() {
        let mut bytes = Vec::new();
        push_u8(&mut bytes, 8);
        push_jstr2(&mut bytes, "vfx");
        push_u8(&mut bytes, 0);
        push_u8(&mut bytes, 1);

        push_jstr2(&mut bytes, "e");
        push_u8(&mut bytes, 0);
        push_u8(&mut bytes, 0);
        push_u8(&mut bytes, 0);
        push_u8(&mut bytes, 0);
        push_u16_be(&mut bytes, 0);
        push_u16_be(&mut bytes, 0);
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 1);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_u8(&mut bytes, 0);
        push_i32_be(&mut bytes, 0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_f32_be(&mut bytes, 0.0);
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 8);
        push_u8(&mut bytes, 33);

        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("vfx decode should succeed: {error}"),
        };
        assert_eq!(1, decoded.emitters[0].modules.len());
        match &decoded.emitters[0].modules[0] {
            Module::Flipbook { frame_rate } => assert_eq!(33, *frame_rate),
            other => panic!("expected flipbook module, got {other:?}"),
        }
    }

    #[test]
    fn tolerates_trailing_bytes() {
        let mut bytes = Vec::new();
        push_u8(&mut bytes, 8);
        push_jstr2(&mut bytes, "v");
        push_u8(&mut bytes, 0);
        push_u8(&mut bytes, 0);
        push_u8(&mut bytes, 0xAA);
        push_u8(&mut bytes, 0xBB);

        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("decode should tolerate trailing bytes: {error}"),
        };
        assert_eq!("v", decoded.name);
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
