use crate::packet::Packet;
use crate::vars::VarDomain;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

// Minimum build for standard RS3 opcode numbering (IDs 0-200 range).
// Builds before 947 use a different encoding (`RuneScriptKt` opcode IDs).
// Config type exports work on all builds; script transpilation requires >= 947.
pub const MIN_SCRIPT_BUILD: u32 = 947;

#[derive(Clone, Debug, Serialize)]
pub struct OpcodeBook {
    pub by_id: Vec<Option<String>>,
}

impl OpcodeBook {
    pub fn load(data_dir: &Path, version: u32, subversion: u32) -> Result<Self> {
        let mut path = data_dir.join("opcodes-unscrambled.txt");
        if version >= 685 {
            let scoped = data_dir.join(format!("opcodes-{version}-{subversion}.txt"));
            if scoped.is_file() {
                path = scoped;
            } else {
                let fallback = data_dir.join(format!("opcodes-{version}.txt"));
                if fallback.is_file() {
                    path = fallback;
                }
            }
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed reading opcode file {}", path.display()))?;
        let mut by_name = BTreeMap::<String, u16>::new();
        for raw in content.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }

            let mut parts = line.split(',');
            let name = parts.next().context("opcode row missing name")?.trim();
            let id_text = parts.next().context("opcode row missing id")?.trim();
            let row_version = parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::parse::<u32>)
                .transpose()
                .context("invalid opcode version gate")?;
            if row_version.is_some_and(|gate| gate > version) {
                continue;
            }

            let id = id_text
                .parse::<u16>()
                .with_context(|| format!("invalid opcode id in row: {line}"))?;
            by_name.insert(name.to_string(), id);
        }

        let max_id = by_name
            .values()
            .copied()
            .max()
            .map(usize::from)
            .unwrap_or(0);
        let mut by_id = vec![None; max_id.saturating_add(1)];
        for (name, id) in by_name {
            by_id[usize::from(id)] = Some(name);
        }
        Ok(Self { by_id })
    }

    pub fn name(&self, opcode: u16) -> Result<&str> {
        self.by_id
            .get(usize::from(opcode))
            .and_then(Option::as_deref)
            .with_context(|| format!("missing opcode mapping for id {opcode}"))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CompiledScript {
    pub name: Option<String>,
    pub local_count_int: u16,
    pub local_count_object: u16,
    pub local_count_long: u16,
    pub argument_count_int: u16,
    pub argument_count_object: u16,
    pub argument_count_long: u16,
    pub code: Vec<Instruction>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Instruction {
    pub opcode: u16,
    pub command: String,
    pub operand: Operand,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum Operand {
    Int(i32),
    Long(i64),
    Str(String),
    Local(i32),
    VarRef(VarRef),
    VarBitRef(VarBitRef),
    Branch(i32),
    Switch(Vec<SwitchCase>),
    Script(i32),
    Array(i32),
    Count(i32),
    Byte(u8),
}

#[derive(Clone, Debug, Serialize)]
pub struct VarRef {
    pub domain: VarDomain,
    pub id: u16,
    pub transmog: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct VarBitRef {
    pub id: u16,
    pub transmog: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SwitchCase {
    pub value: i32,
    pub target: i32,
}

pub fn decode_script(
    data: &[u8],
    opcode_book: &OpcodeBook,
    version: u32,
) -> Result<CompiledScript> {
    let mut packet = Packet::new(data);
    let mut header_size = 12_usize;
    if version >= 642 {
        header_size = header_size.checked_add(4).context("header size overflow")?;
    }
    if version >= 488 {
        packet.set_pos(data.len().saturating_sub(2))?;
        let trailer_size = usize::from(packet.g2()?);
        header_size = header_size
            .checked_add(2)
            .and_then(|v| v.checked_add(trailer_size))
            .context("header size overflow")?;
    }
    let header_pos = data
        .len()
        .checked_sub(header_size)
        .context("invalid script header size")?;

    packet.set_pos(header_pos)?;
    let code_len = usize::try_from(packet.g4s()?).context("negative script code length")?;
    let local_count_int = packet.g2()?;
    let local_count_object = packet.g2()?;
    let local_count_long = if version >= 642 { packet.g2()? } else { 0 };
    let argument_count_int = packet.g2()?;
    let argument_count_object = packet.g2()?;
    let argument_count_long = if version >= 642 { packet.g2()? } else { 0 };

    let mut switch_values: Vec<Vec<i32>> = Vec::new();
    let mut switch_offsets: Vec<Vec<i32>> = Vec::new();
    if version >= 488 {
        let switch_count = usize::from(packet.g1()?);
        switch_values = vec![Vec::new(); switch_count];
        switch_offsets = vec![Vec::new(); switch_count];
        for index in 0..switch_count {
            let case_count = usize::from(packet.g2()?);
            let mut values = Vec::with_capacity(case_count);
            let mut offsets = Vec::with_capacity(case_count);
            for _ in 0..case_count {
                values.push(packet.g4s()?);
                offsets.push(packet.g4s()?);
            }
            switch_values[index] = values;
            switch_offsets[index] = offsets;
        }
    }

    packet.set_pos(0)?;
    let name = if version >= 459 {
        packet.gjstrnull()?
    } else {
        None
    };

    let mut code = Vec::with_capacity(code_len);
    while packet.pos() < header_pos {
        let opcode = packet.g2()?;
        let command = opcode_book.name(opcode)?.to_string();
        let index = i32::try_from(code.len()).context("script too large")?;
        let operand = decode_operand(
            &command,
            &mut packet,
            version,
            index,
            &switch_values,
            &switch_offsets,
        )?;
        code.push(Instruction {
            opcode,
            command,
            operand,
        });
    }

    if code.len() != code_len {
        bail!(
            "script code length mismatch: decoded {} expected {}",
            code.len(),
            code_len
        );
    }

    Ok(CompiledScript {
        name,
        local_count_int,
        local_count_object,
        local_count_long,
        argument_count_int,
        argument_count_object,
        argument_count_long,
        code,
    })
}

fn decode_operand(
    command: &str,
    packet: &mut Packet<'_>,
    version: u32,
    index: i32,
    switch_values: &[Vec<i32>],
    switch_offsets: &[Vec<i32>],
) -> Result<Operand> {
    match command {
        "push_constant_int" => Ok(Operand::Int(packet.g4s()?)),
        "push_long_constant" => Ok(Operand::Long(packet.g8s()?)),
        "push_constant_string" => {
            if version < 800 {
                Ok(Operand::Str(packet.gjstr()?))
            } else {
                match packet.g1()? {
                    0 => Ok(Operand::Int(packet.g4s()?)),
                    1 => Ok(Operand::Long(packet.g8s()?)),
                    2 => Ok(Operand::Str(packet.gjstr()?)),
                    tag => bail!("unsupported typed constant tag: {tag}"),
                }
            }
        }
        "push_int_local" | "pop_int_local" | "push_string_local" | "pop_string_local"
        | "push_long_local" | "pop_long_local" => Ok(Operand::Local(packet.g4s()?)),
        "push_var" | "pop_var" => {
            if version < 800 {
                let id = u16::try_from(packet.g4s()?).context("legacy var id out of range")?;
                Ok(Operand::VarRef(VarRef {
                    domain: VarDomain::Player,
                    id,
                    transmog: false,
                }))
            } else {
                let domain = VarDomain::from_id(packet.g1()?)?;
                let id = packet.g2()?;
                let transmog = packet.g1()? == 1;
                Ok(Operand::VarRef(VarRef {
                    domain,
                    id,
                    transmog,
                }))
            }
        }
        "push_varbit" | "pop_varbit" => {
            if version < 800 {
                let id = u16::try_from(packet.g4s()?).context("legacy varbit id out of range")?;
                Ok(Operand::VarBitRef(VarBitRef {
                    id,
                    transmog: false,
                }))
            } else {
                let id = packet.g2()?;
                let transmog = packet.g1()? == 1;
                Ok(Operand::VarBitRef(VarBitRef { id, transmog }))
            }
        }
        "push_varc_int"
        | "pop_varc_int"
        | "push_varc_string"
        | "pop_varc_string"
        | "push_varclan"
        | "push_varclanbit"
        | "push_varclan_long"
        | "push_varclan_string"
        | "push_varclansetting"
        | "push_varclansettingbit"
        | "push_varclansetting_long"
        | "push_varclansetting_string" => Ok(Operand::Int(packet.g4s()?)),
        "branch"
        | "branch_not"
        | "branch_equals"
        | "branch_less_than"
        | "branch_greater_than"
        | "branch_less_than_or_equals"
        | "branch_greater_than_or_equals"
        | "long_branch_not"
        | "long_branch_equals"
        | "long_branch_less_than"
        | "long_branch_greater_than"
        | "long_branch_less_than_or_equals"
        | "long_branch_greater_than_or_equals"
        | "branch_if_true"
        | "branch_if_false" => Ok(Operand::Branch(index + packet.g4s()?)),
        "switch" => {
            let switch_index =
                usize::try_from(packet.g4s()?).context("negative switch table index")?;
            let values = switch_values
                .get(switch_index)
                .with_context(|| format!("switch table out of range: {switch_index}"))?;
            let offsets = switch_offsets
                .get(switch_index)
                .with_context(|| format!("switch offset table out of range: {switch_index}"))?;
            if values.len() != offsets.len() {
                bail!("switch table length mismatch");
            }
            let mut cases = Vec::with_capacity(values.len());
            for (value, offset) in values.iter().zip(offsets) {
                cases.push(SwitchCase {
                    value: *value,
                    target: index + *offset,
                });
            }
            Ok(Operand::Switch(cases))
        }
        "join_string" => Ok(Operand::Count(packet.g4s()?)),
        "gosub_with_params" => Ok(Operand::Script(packet.g4s()?)),
        "define_array"
        | "push_array_int"
        | "pop_array_int"
        | "push_array_int_leave_index_on_stack"
        | "push_array_int_and_index"
        | "pop_array_int_leave_value_on_stack" => Ok(Operand::Array(packet.g4s()?)),
        _ => Ok(Operand::Byte(packet.g1()?)),
    }
}
