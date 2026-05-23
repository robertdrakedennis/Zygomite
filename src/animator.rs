use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum AnimatorController {
    #[serde(rename = "AnimationStateMachine")]
    AnimationStateMachine {
        initial: String,
        states: Vec<NamedState>,
        transitions: Vec<Transition>,
    },
    #[serde(rename = "LayeredAnimatorController")]
    LayeredAnimatorController { layers: Vec<LayeredAnimatorLayer> },
}

#[derive(Clone, Debug, Serialize)]
pub struct LayeredAnimatorLayer {
    pub layer: AnimationStateMachine,
    pub unknown1: i32,
    pub unknown2: String,
    pub unknown3: i64,
    pub unknown4: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnimationStateMachine {
    pub initial: String,
    pub states: Vec<NamedState>,
    pub transitions: Vec<Transition>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NamedState {
    pub name: String,
    pub state: Option<StateNode>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Transition {
    pub state1: String,
    pub state2: String,
    pub duration: i64,
    pub unknown3: i32,
    pub unknown4: i32,
    pub unknown5: String,
    pub unknown6: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum StateNode {
    #[serde(rename = "Animation")]
    Animation {
        animation: String,
        unknown1: i32,
        unknown2: i32,
        unknown3: i32,
        unknown4: u8,
        unknown5: u8,
        unknown6: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        unknown7: Option<String>,
        unknown8: i32,
    },
    #[serde(rename = "BlendTreeDirect")]
    BlendTreeDirect {
        unknown0: i32,
        unknown1: i32,
        unknown2: String,
        unknown3: Box<Self>,
        unknown4: Box<Self>,
    },
    #[serde(rename = "BlendTree1D")]
    BlendTree1D {
        unknown0: f32,
        unknown1: f32,
        variable: String,
        animations: Vec<BlendTree1DEntry>,
    },
    #[serde(rename = "BlendTree2D")]
    BlendTree2D {
        variable1: String,
        variable2: String,
        animations: Vec<BlendTree2DEntry>,
        unknown3: Vec<i32>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct BlendTree1DEntry {
    pub value: f32,
    pub animation: StateNode,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlendTree2DEntry {
    pub value1: f32,
    pub value2: f32,
    pub animation: StateNode,
}

pub fn decode(data: &[u8]) -> Result<AnimatorController> {
    let mut packet = Packet::new(data);
    let version = packet.g1()?;
    if version != 1 {
        bail!("animator controller unsupported version {version}");
    }
    let controller_type = packet.g1()?;
    let controller = match controller_type {
        0 => {
            let state_machine = decode_state_machine(&mut packet)?;
            AnimatorController::AnimationStateMachine {
                initial: state_machine.initial,
                states: state_machine.states,
                transitions: state_machine.transitions,
            }
        }
        1 => AnimatorController::LayeredAnimatorController {
            layers: decode_layered_controller(&mut packet)?,
        },
        value => bail!("animator controller unknown type {value}"),
    };
    Ok(controller)
}

fn decode_layered_controller(packet: &mut Packet<'_>) -> Result<Vec<LayeredAnimatorLayer>> {
    let layer_count = i32_count_to_usize(packet.g4s()?, "layer count")?;
    let mut layers = Vec::with_capacity(layer_count);
    for _ in 0..layer_count {
        layers.push(LayeredAnimatorLayer {
            layer: decode_state_machine(packet)?,
            unknown1: packet.g4s()?,
            unknown2: packet.gjstr()?,
            unknown3: packet.g8s()?,
            unknown4: packet.g8s()?,
        });
    }
    Ok(layers)
}

fn decode_state_machine(packet: &mut Packet<'_>) -> Result<AnimationStateMachine> {
    let initial = packet.gjstr()?;
    let state_count = i32_count_to_usize(packet.g4s()?, "state count")?;
    let mut states = Vec::with_capacity(state_count);
    for _ in 0..state_count {
        states.push(decode_named_state(packet)?);
    }

    let transition_count = i32_count_to_usize(packet.g4s()?, "transition count")?;
    let mut transitions = Vec::with_capacity(transition_count);
    for _ in 0..transition_count {
        transitions.push(Transition {
            state1: packet.gjstr()?,
            state2: packet.gjstr()?,
            duration: packet.g8s()?,
            unknown3: packet.g4s()?,
            unknown4: packet.g4s()?,
            unknown5: packet.gjstr()?,
            unknown6: packet.g4s()?,
        });
    }

    Ok(AnimationStateMachine {
        initial,
        states,
        transitions,
    })
}

fn decode_named_state(packet: &mut Packet<'_>) -> Result<NamedState> {
    let name = packet.gjstr()?;
    let state_type = packet.g4s()?;
    let state = match state_type {
        0 => None,
        1 => Some(decode_animation(packet)?),
        2 => Some(decode_blend_tree(packet)?),
        value => bail!("named state invalid type {value}"),
    };
    Ok(NamedState { name, state })
}

fn decode_blend_tree(packet: &mut Packet<'_>) -> Result<StateNode> {
    let node_type = packet.g4s()?;
    match node_type {
        0 => decode_animation(packet),
        1 => Ok(StateNode::BlendTreeDirect {
            unknown0: packet.g4s()?,
            unknown1: packet.g4s()?,
            unknown2: packet.gjstr()?,
            unknown3: Box::new(decode_blend_tree(packet)?),
            unknown4: Box::new(decode_blend_tree(packet)?),
        }),
        2 => {
            let unknown0 = f32::from_bits(packet.g4s()? as u32);
            let unknown1 = f32::from_bits(packet.g4s()? as u32);
            let variable = packet.gjstr()?;
            let entry_count = i32_count_to_usize(packet.g4s()?, "blend tree 1d entry count")?;
            let mut animations = Vec::with_capacity(entry_count);
            for _ in 0..entry_count {
                animations.push(BlendTree1DEntry {
                    value: f32::from_bits(packet.g4s()? as u32),
                    animation: decode_blend_tree(packet)?,
                });
            }
            Ok(StateNode::BlendTree1D {
                unknown0,
                unknown1,
                variable,
                animations,
            })
        }
        3 => {
            let variable1 = packet.gjstr()?;
            let variable2 = packet.gjstr()?;
            let entry_count = i32_count_to_usize(packet.g4s()?, "blend tree 2d entry count")?;
            let mut animations = Vec::with_capacity(entry_count);
            for _ in 0..entry_count {
                animations.push(BlendTree2DEntry {
                    value1: f32::from_bits(packet.g4s()? as u32),
                    value2: f32::from_bits(packet.g4s()? as u32),
                    animation: decode_blend_tree(packet)?,
                });
            }

            let unknown_count = i32_count_to_usize(packet.g4s()?, "blend tree 2d integer count")?;
            let mut unknown3 = Vec::with_capacity(unknown_count);
            for _ in 0..unknown_count {
                unknown3.push(packet.g4s()?);
            }

            Ok(StateNode::BlendTree2D {
                variable1,
                variable2,
                animations,
                unknown3,
            })
        }
        value => bail!("unknown blend tree type {value}"),
    }
}

fn decode_animation(packet: &mut Packet<'_>) -> Result<StateNode> {
    let sequence_id = packet.g4s()?;
    let unknown1 = packet.g4s()?;
    let unknown2 = packet.g4s()?;
    let unknown3 = packet.g4s()?;
    let unknown4 = packet.g1()?;
    let unknown5 = packet.g1()?;

    let (unknown6, unknown7, unknown8) = if unknown5 != 0 {
        (packet.g4s()? as f32, Some(packet.gjstr()?), packet.g4s()?)
    } else {
        (0.0_f32, None, 0_i32)
    };

    Ok(StateNode::Animation {
        animation: format_seq(sequence_id),
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

fn format_seq(value: i32) -> String {
    if value == -1 {
        String::from("null")
    } else {
        format!("seq_{value}")
    }
}

fn i32_count_to_usize(value: i32, label: &str) -> Result<usize> {
    usize::try_from(value).with_context(|| format!("negative {label}: {value}"))
}

#[cfg(test)]
mod tests {
    use super::{AnimatorController, decode};

    #[test]
    fn decodes_animation_state_machine() {
        let mut bytes = Vec::new();
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 0);
        push_jstr(&mut bytes, "idle");

        push_i32_be(&mut bytes, 1);
        push_jstr(&mut bytes, "idle");
        push_i32_be(&mut bytes, 1);
        push_i32_be(&mut bytes, 123);
        push_i32_be(&mut bytes, 0);
        push_i32_be(&mut bytes, 1);
        push_i32_be(&mut bytes, 2);
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 0);

        push_i32_be(&mut bytes, 1);
        push_jstr(&mut bytes, "idle");
        push_jstr(&mut bytes, "run");
        push_i64_be(&mut bytes, 10);
        push_i32_be(&mut bytes, 3);
        push_i32_be(&mut bytes, 4);
        push_jstr(&mut bytes, "");
        push_i32_be(&mut bytes, 5);

        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("animator decode should succeed: {error}"),
        };
        match decoded {
            AnimatorController::AnimationStateMachine {
                initial,
                states,
                transitions,
            } => {
                assert_eq!("idle", initial);
                assert_eq!(1, states.len());
                assert_eq!(1, transitions.len());
            }
            AnimatorController::LayeredAnimatorController { .. } => {
                panic!("expected AnimationStateMachine")
            }
        }
    }

    #[test]
    fn decodes_layered_animator_controller() {
        let mut bytes = Vec::new();
        push_u8(&mut bytes, 1);
        push_u8(&mut bytes, 1);
        push_i32_be(&mut bytes, 0);

        let decoded = match decode(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("layered decode should succeed: {error}"),
        };
        match decoded {
            AnimatorController::LayeredAnimatorController { layers } => {
                assert_eq!(0, layers.len());
            }
            AnimatorController::AnimationStateMachine { .. } => {
                panic!("expected LayeredAnimatorController")
            }
        }
    }

    fn push_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    fn push_i32_be(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i64_be(out: &mut Vec<u8>, value: i64) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_jstr(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }
}
