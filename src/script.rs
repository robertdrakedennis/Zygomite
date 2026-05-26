use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::{ByteWriter, Packet};
use crate::vars::VarDomain;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

// ── Build compatibility ──
//
// Build 947+ (supported): Classic RS3 bytecode format. Opcodes 0-200
// mapped via opcodes-{version}.txt with full type signatures.
// Code length validation enforced; all operators decode cleanly.
//
// Builds < 947 (best-effort): The bytecode may use RuneScriptKt
// opcode numbering (command IDs in the 4000+ range for arithmetic,
// separate namespaces for engine commands). We decode what we can
// from the opcode table and surface unknown commands as `cmd_NNNN`
// with a 1-byte operand. Code length validation is relaxed.
pub const MIN_SCRIPT_BUILD: u32 = 947;

#[derive(Clone, Debug, Serialize)]
pub struct OpcodeBook {
    pub by_id: Vec<Option<String>>,
    by_name: BTreeMap<String, u16>,
    large_by_id: Vec<bool>,
}

impl OpcodeBook {
    pub fn load(data_dir: &Path, version: u32, subversion: u32) -> Result<Self> {
        // Build-specific opcode file (e.g. opcodes-910.txt, opcodes-947.txt).
        // Falls back to opcodes-unscrambled.txt if no version-specific file exists.
        let path = scoped_data_file(data_dir, "opcodes", version, subversion)
            .unwrap_or_else(|| data_dir.join("opcodes-unscrambled.txt"));

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
        for (name, id) in &by_name {
            by_id[usize::from(*id)] = Some(name.clone());
        }

        let mut large_by_id = vec![false; by_id.len()];
        if let Some(large_path) = scoped_data_file(data_dir, "opcodes-large", version, subversion) {
            let large_content = fs::read_to_string(&large_path).with_context(|| {
                format!("failed reading large-operand file {}", large_path.display())
            })?;
            for raw in large_content.lines() {
                let line = raw.trim();
                if line.is_empty() || line.starts_with("//") {
                    continue;
                }
                let mut parts = line.split(',');
                let id = parts
                    .next()
                    .context("large-operand row missing id")?
                    .trim()
                    .parse::<usize>()
                    .with_context(|| format!("invalid large-operand id in row: {line}"))?;
                let is_large = parts
                    .next()
                    .context("large-operand row missing flag")?
                    .trim()
                    .parse::<u8>()
                    .with_context(|| format!("invalid large-operand flag in row: {line}"))?
                    != 0;
                if id >= large_by_id.len() {
                    large_by_id.resize(id + 1, false);
                }
                large_by_id[id] = is_large;
            }
        }

        Ok(Self {
            by_id,
            by_name,
            large_by_id,
        })
    }

    pub fn name(&self, opcode: u16) -> Result<&str> {
        self.by_id
            .get(usize::from(opcode))
            .and_then(Option::as_deref)
            .with_context(|| format!("missing opcode mapping for id {opcode}"))
    }

    pub fn opcode_for(&self, name: &str) -> Result<u16> {
        self.by_name
            .get(name)
            .copied()
            .with_context(|| format!("missing opcode mapping for name '{name}'"))
    }

    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    pub fn has_large_operand(&self, opcode: u16) -> bool {
        self.large_by_id
            .get(usize::from(opcode))
            .copied()
            .unwrap_or(false)
    }
}

fn scoped_data_file(
    data_dir: &Path,
    stem: &str,
    version: u32,
    subversion: u32,
) -> Option<std::path::PathBuf> {
    if version < 685 {
        return None;
    }
    let scoped = data_dir.join(format!("{stem}-{version}-{subversion}.txt"));
    if scoped.is_file() {
        return Some(scoped);
    }
    let fallback = data_dir.join(format!("{stem}-{version}.txt"));
    fallback.is_file().then_some(fallback)
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

fn fixed_var_command_domain(command: &str) -> Option<VarDomain> {
    match command {
        "push_varc_int" | "pop_varc_int" | "push_varc_string" | "pop_varc_string" => {
            Some(VarDomain::Client)
        }
        "push_varclan" | "push_varclan_long" | "push_varclan_string" => Some(VarDomain::Clan),
        "push_varclansetting" | "push_varclansetting_long" | "push_varclansetting_string" => {
            Some(VarDomain::ClanSetting)
        }
        _ => None,
    }
}

fn is_fixed_varbit_command(command: &str) -> bool {
    matches!(command, "push_varclanbit" | "push_varclansettingbit")
}

fn parse_fixed_var_ref_operand(command: &str, operand_text: &str) -> Result<Operand> {
    let Some(expected_domain) = fixed_var_command_domain(command) else {
        bail!("fixed var command expected");
    };
    if let Ok(id) = operand_text.parse::<u16>() {
        return Ok(Operand::VarRef(VarRef {
            domain: expected_domain,
            id,
            transmog: false,
        }));
    }
    let Some((domain_str, id_str)) = operand_text.split_once(':') else {
        bail!("expected integer or domain:id format for {command}, got: {operand_text}");
    };
    let domain = VarDomain::from_label(domain_str).context("unknown var domain")?;
    if domain != expected_domain {
        bail!(
            "expected {} domain for {command}, got {domain_str}",
            expected_domain.as_label()
        );
    }
    let id = id_str.parse::<u16>().context("expected var id")?;
    Ok(Operand::VarRef(VarRef {
        domain,
        id,
        transmog: false,
    }))
}

fn parse_fixed_varbit_operand(command: &str, operand_text: &str) -> Result<Operand> {
    if !is_fixed_varbit_command(command) {
        bail!("fixed varbit command expected");
    }
    let text = operand_text.strip_prefix("varbit:").unwrap_or(operand_text);
    let id = text.parse::<u16>().context("expected varbit id")?;
    Ok(Operand::VarBitRef(VarBitRef {
        id,
        transmog: false,
    }))
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
        let command = opcode_book
            .name(opcode)
            .map(str::to_string)
            // Builds before MIN_SCRIPT_BUILD: unknown opcodes get synthetic
            // names so the script decodes without failing. This surfaces
            // every instruction in the output even when we lack the full
            // opcode table for that build.
            .unwrap_or_else(|_| format!("cmd_{opcode}"));
        let index = i32::try_from(code.len()).context("script too large")?;
        let operand = decode_operand(
            &command,
            opcode_book.has_large_operand(opcode),
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

    // ── Code length validation ──
    // Only enforced when we have a complete opcode table. On older builds
    // unknown commands may decode with different operand sizes than the
    // actual bytecode, so the instruction count can drift.
    if version >= MIN_SCRIPT_BUILD && code.len() != code_len {
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
    is_large_operand: bool,
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
        cmd if fixed_var_command_domain(cmd).is_some() => {
            let id = u16::try_from(packet.g4s()?).context("fixed var id out of range")?;
            Ok(Operand::VarRef(VarRef {
                domain: fixed_var_command_domain(cmd).expect("checked above"),
                id,
                transmog: false,
            }))
        }
        cmd if is_fixed_varbit_command(cmd) => {
            let id = u16::try_from(packet.g4s()?).context("fixed varbit id out of range")?;
            Ok(Operand::VarBitRef(VarBitRef {
                id,
                transmog: false,
            }))
        }
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
        // ── Unknown commands ──
        // Legacy handler tables still vary by build. When explicit
        // per-build width metadata exists, preserve full i32 operands.
        _ if is_large_operand => Ok(Operand::Int(packet.g4s()?)),
        _ => Ok(Operand::Byte(packet.g1()?)),
    }
}

// ── Encoder ──

/// Information about a switch instruction that needs patching.
struct SwitchInfo {
    /// Byte position in the stream where the i32 switch table index is written.
    patch_pos: usize,
    /// Case values (unmodified from `Operand::Switch`).
    values: Vec<i32>,
    /// Absolute target indices for each case.
    targets: Vec<i32>,
    /// Index of this switch instruction in the script (0-based).
    instr_index: i32,
}

/// Write an i32 big-endian at a specific position in the writer's buffer.
fn patch_i32_at(data: &mut [u8], pos: usize, value: i32) -> Result<()> {
    let bytes = value.to_be_bytes();
    let end = pos.checked_add(4).context("patch position overflow")?;
    if end > data.len() {
        bail!("patch position {pos} out of bounds (len {})", data.len());
    }
    data[pos..end].copy_from_slice(&bytes);
    Ok(())
}

pub fn encode_script(
    script: &CompiledScript,
    opcode_book: &OpcodeBook,
    version: u32,
) -> Result<Vec<u8>> {
    let mut writer = ByteWriter::with_capacity(script.code.len() * 8 + 128);

    // ── Write name at position 0 ──
    if version >= 459 {
        writer.pjstrnull(script.name.as_deref());
    }

    // ── Pass 1: emit instructions. Branches and switches get placeholder i32=0. ──
    #[derive(Debug)]
    struct BranchPatch {
        pos: usize,
        target: i32,
        instr_index: i32,
    }
    let mut branch_patches: Vec<BranchPatch> = Vec::new();
    let mut switch_infos: Vec<SwitchInfo> = Vec::new();

    for (instr_index, instr) in script.code.iter().enumerate() {
        let instr_index_i32 = i32::try_from(instr_index).context("instruction index overflow")?;
        let opcode = opcode_book.opcode_for(&instr.command)?;
        writer.p2(opcode);

        let is_branch = matches!(&instr.operand, Operand::Branch(_));
        let is_switch = matches!(&instr.operand, Operand::Switch(_));

        if is_branch {
            if let Operand::Branch(target) = &instr.operand {
                branch_patches.push(BranchPatch {
                    pos: writer.len(),
                    target: *target,
                    instr_index: instr_index_i32,
                });
            }
            writer.p4s(0); // placeholder
        } else if is_switch {
            if let Operand::Switch(cases) = &instr.operand {
                let patch_pos = writer.len();
                writer.p4s(0); // placeholder for switch table index
                switch_infos.push(SwitchInfo {
                    patch_pos,
                    values: cases.iter().map(|c| c.value).collect(),
                    targets: cases.iter().map(|c| c.target).collect(),
                    instr_index: instr_index_i32,
                });
            }
        } else {
            encode_operand(
                &instr.operand,
                &instr.command,
                opcode_book.has_large_operand(opcode),
                version,
                &mut writer,
            )?;
        }
    }

    // ── Pass 2: patch branch relative offsets ──
    for bp in &branch_patches {
        let relative = bp.target - bp.instr_index;
        patch_i32_at(&mut writer.data, bp.pos, relative)?;
    }

    // ── Pass 2: patch switch table indices (ordinal 0, 1, 2, ...) ──
    for (table_idx, sw) in switch_infos.iter().enumerate() {
        let idx = i32::try_from(table_idx).context("switch table index overflow")?;
        patch_i32_at(&mut writer.data, sw.patch_pos, idx)?;
    }

    // ── Write header (code_len first, then counts, then switch tables, then trailer_size) ──
    let code_len = i32::try_from(script.code.len()).context("script too large to encode")?;
    writer.p4s(code_len);
    writer.p2(script.local_count_int);
    writer.p2(script.local_count_object);
    if version >= 642 {
        writer.p2(script.local_count_long);
    }
    writer.p2(script.argument_count_int);
    writer.p2(script.argument_count_object);
    if version >= 642 {
        writer.p2(script.argument_count_long);
    }

    // ── Write switch tables at end of header ──
    let switch_table_start = writer.len();
    let switch_count = u8::try_from(switch_infos.len()).context("too many switch tables")?;
    writer.p1(switch_count);
    for sw in &switch_infos {
        let case_count = u16::try_from(sw.values.len()).context("switch case count overflow")?;
        writer.p2(case_count);
        for (value, target) in sw.values.iter().zip(sw.targets.iter()) {
            let relative = *target - sw.instr_index;
            writer.p4s(*value);
            writer.p4s(relative);
        }
    }
    let trailer_size =
        u16::try_from(writer.len() - switch_table_start).context("trailer too large")?;

    // ── Write trailer size ──
    writer.p2(trailer_size);

    Ok(writer.data)
}

fn encode_operand(
    operand: &Operand,
    command: &str,
    is_large_operand: bool,
    version: u32,
    writer: &mut ByteWriter,
) -> Result<()> {
    match command {
        "push_constant_int" => match operand {
            Operand::Int(v) => writer.p4s(*v),
            _ => bail!("push_constant_int expected Int operand"),
        },
        "push_long_constant" => match operand {
            Operand::Long(v) => writer.p8s(*v),
            _ => bail!("push_long_constant expected Long operand"),
        },
        "push_constant_string" => {
            if version < 800 {
                match operand {
                    Operand::Str(s) => writer.pjstr(s),
                    Operand::Int(v) => writer.p4s(*v),
                    _ => bail!("push_constant_string unexpected operand type for v<800"),
                }
            } else {
                match operand {
                    Operand::Int(v) => {
                        writer.p1(0);
                        writer.p4s(*v);
                    }
                    Operand::Long(v) => {
                        writer.p1(1);
                        writer.p8s(*v);
                    }
                    Operand::Str(s) => {
                        writer.p1(2);
                        writer.pjstr(s);
                    }
                    _ => bail!("push_constant_string unexpected operand type for v>=800"),
                }
            }
        }
        "push_int_local" | "pop_int_local" | "push_string_local" | "pop_string_local"
        | "push_long_local" | "pop_long_local" => match operand {
            Operand::Local(v) | Operand::Int(v) => writer.p4s(*v),
            _ => bail!("local command expected Local/Int operand"),
        },
        "push_var" | "pop_var" => match operand {
            Operand::VarRef(vr) => {
                if version < 800 {
                    writer.p4s(i32::from(vr.id));
                } else {
                    writer.p1(vr.domain as u8);
                    writer.p2(vr.id);
                    writer.p1(u8::from(vr.transmog));
                }
            }
            _ => bail!("push/pop_var expected VarRef operand"),
        },
        "push_varbit" | "pop_varbit" => match operand {
            Operand::VarBitRef(vbr) => {
                if version < 800 {
                    writer.p4s(i32::from(vbr.id));
                } else {
                    writer.p2(vbr.id);
                    writer.p1(u8::from(vbr.transmog));
                }
            }
            _ => bail!("push/pop_varbit expected VarBitRef operand"),
        },
        cmd if fixed_var_command_domain(cmd).is_some() => {
            let expected_domain = fixed_var_command_domain(cmd).expect("checked above");
            match operand {
                Operand::VarRef(vr) => {
                    if vr.domain != expected_domain {
                        bail!(
                            "{cmd} expected {} var domain, got {}",
                            expected_domain.as_label(),
                            vr.domain.as_label()
                        );
                    }
                    writer.p4s(i32::from(vr.id));
                }
                Operand::Int(v) => writer.p4s(*v),
                _ => bail!("{cmd} expected VarRef/Int operand"),
            }
        }
        cmd if is_fixed_varbit_command(cmd) => match operand {
            Operand::VarBitRef(vbr) => writer.p4s(i32::from(vbr.id)),
            Operand::Int(v) => writer.p4s(*v),
            _ => bail!("{cmd} expected VarBitRef/Int operand"),
        },
        "join_string" => match operand {
            Operand::Count(v) | Operand::Int(v) => writer.p4s(*v),
            _ => bail!("join_string expected Count operand"),
        },
        "gosub_with_params" => match operand {
            Operand::Script(v) | Operand::Int(v) => writer.p4s(*v),
            _ => bail!("gosub_with_params expected Script operand"),
        },
        "define_array"
        | "push_array_int"
        | "pop_array_int"
        | "push_array_int_leave_index_on_stack"
        | "push_array_int_and_index"
        | "pop_array_int_leave_value_on_stack" => match operand {
            Operand::Array(v) | Operand::Int(v) => writer.p4s(*v),
            _ => bail!("array command expected Array/Int operand"),
        },
        _ => match operand {
            Operand::Byte(b) if is_large_operand => writer.p4s(i32::from(*b)),
            Operand::Byte(b) => writer.p1(*b),
            Operand::Int(v) if is_large_operand => writer.p4s(*v),
            Operand::Int(v) => writer.p1(*v as u8),
            _ if is_large_operand => writer.p4s(0),
            _ => writer.p1(0),
        },
    }
    Ok(())
}

// ── ASM pragma format (lossless roundtrip representation) ──
//
// Format:
//   // @cs2 name "script_name"
//   // @cs2 locals int=2 obj=1 long=0
//   // @cs2 args int=1 obj=0 long=0
//   // @cs2 <opcode> [operand]
//   //   @cs2 case <value> <target>
//
// Examples:
//   // @cs2 push_constant_int 42
//   // @cs2 push_int_local 0
//   // @cs2 add
//   // @cs2 pop_int_local 0
//   // @cs2 branch 10
//   // @cs2 switch
//   //   @cs2 case 0 5
//   //   @cs2 case 1 8
//   // @cs2 return

/// Serialize a `CompiledScript` to ASM pragma lines (one per instruction).
pub fn script_to_asm(script: &CompiledScript) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    // Header
    if let Some(name) = &script.name {
        let _ = writeln!(out, "// @cs2 name \"{}\"", name.replace('"', "\\\""));
    }
    let _ = writeln!(
        out,
        "// @cs2 locals int={} obj={} long={}",
        script.local_count_int, script.local_count_object, script.local_count_long
    );
    let _ = writeln!(
        out,
        "// @cs2 args int={} obj={} long={}",
        script.argument_count_int, script.argument_count_object, script.argument_count_long
    );

    for instr in &script.code {
        if instr.command == "switch" {
            let _ = writeln!(out, "// @cs2 switch");
            if let Operand::Switch(cases) = &instr.operand {
                for c in cases {
                    let _ = writeln!(out, "//   @cs2 case {} {}", c.value, c.target);
                }
            }
        } else {
            let _ = writeln!(out, "// @cs2 {}", format_instruction_asm(instr));
        }
    }

    out
}

fn format_instruction_asm(instr: &Instruction) -> String {
    if instr.command == "push_constant_string" {
        return format_push_constant_string_asm(&instr.operand);
    }
    if !command_has_explicit_asm_operand_format(&instr.command)
        && let Operand::Int(v) = &instr.operand
    {
        return format!("{} raw32:{v}", instr.command);
    }
    let operand_str = format_operand_asm(&instr.operand);
    if operand_str.is_empty() {
        instr.command.clone()
    } else {
        format!("{} {}", instr.command, operand_str)
    }
}

fn format_push_constant_string_asm(operand: &Operand) -> String {
    match operand {
        Operand::Int(v) => format!("push_constant_string int:{v}"),
        Operand::Long(v) => format!("push_constant_string long:{v}"),
        Operand::Str(s) => format!(
            "push_constant_string str:\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        ),
        _ => format!("push_constant_string {}", format_operand_asm(operand)),
    }
}

fn format_operand_asm(operand: &Operand) -> String {
    match operand {
        Operand::Int(v) => v.to_string(),
        Operand::Long(v) => format!("{v}"),
        Operand::Str(s) => format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        ),
        Operand::Local(v) => v.to_string(),
        Operand::VarRef(vr) => {
            let base = format!("{}:{}", vr.domain.as_label(), vr.id);
            if vr.transmog {
                format!("{base}:transmog")
            } else {
                base
            }
        }
        Operand::VarBitRef(vbr) => {
            if vbr.transmog {
                format!("{}:transmog", vbr.id)
            } else {
                vbr.id.to_string()
            }
        }
        Operand::Branch(target) => target.to_string(),
        Operand::Switch(_) => String::new(),
        Operand::Script(v) => v.to_string(),
        Operand::Array(v) => v.to_string(),
        Operand::Count(v) => v.to_string(),
        Operand::Byte(v) => v.to_string(),
    }
}

fn command_has_explicit_asm_operand_format(command: &str) -> bool {
    matches!(
        command,
        "push_constant_int"
            | "push_long_constant"
            | "push_constant_string"
            | "push_int_local"
            | "pop_int_local"
            | "push_string_local"
            | "pop_string_local"
            | "push_long_local"
            | "pop_long_local"
            | "push_var"
            | "pop_var"
            | "push_varbit"
            | "pop_varbit"
            | "push_varc_int"
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
            | "push_varclansetting_string"
            | "branch"
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
            | "branch_if_false"
            | "switch"
            | "join_string"
            | "gosub_with_params"
            | "define_array"
            | "push_array_int"
            | "pop_array_int"
            | "push_array_int_leave_index_on_stack"
            | "push_array_int_and_index"
            | "pop_array_int_leave_value_on_stack"
    )
}

/// Parse ASM pragma lines back into a `CompiledScript`.
pub fn parse_cs2_asm(source: &str) -> Result<CompiledScript> {
    let mut name: Option<String> = None;
    let mut local_count_int: u16 = 0;
    let mut local_count_object: u16 = 0;
    let mut local_count_long: u16 = 0;
    let mut argument_count_int: u16 = 0;
    let mut argument_count_object: u16 = 0;
    let mut argument_count_long: u16 = 0;

    let mut instructions: Vec<Instruction> = Vec::new();
    // In the ASM format, switch cases are emitted after the switch instruction.
    // We collect them here and process them when we encounter a `switch` line.
    let mut pending_switch_cases: Vec<SwitchCase> = Vec::new();
    let mut pending_switch_open = false;

    for raw_line in source.lines() {
        let line = raw_line.trim();

        // Only process @cs2 pragma lines
        let cs2_content = if let Some(rest) = line.strip_prefix("// @cs2 ") {
            rest.trim()
        } else if let Some(rest) = line.strip_prefix("//   @cs2 ") {
            rest.trim()
        } else {
            continue;
        };

        if cs2_content.is_empty() {
            continue;
        }

        // Handle special header lines
        if let Some(name_val) = cs2_content.strip_prefix("name ") {
            let name_val = name_val.trim();
            if name_val.starts_with('"') && name_val.ends_with('"') {
                name = Some(
                    name_val[1..name_val.len() - 1]
                        .replace("\\\"", "\"")
                        .replace("\\\\", "\\"),
                );
            }
            continue;
        }

        if let Some(counts) = cs2_content.strip_prefix("locals ") {
            parse_counts(
                counts,
                &mut local_count_int,
                &mut local_count_object,
                &mut local_count_long,
            )?;
            continue;
        }

        if let Some(counts) = cs2_content.strip_prefix("args ") {
            parse_counts(
                counts,
                &mut argument_count_int,
                &mut argument_count_object,
                &mut argument_count_long,
            )?;
            continue;
        }

        // Handle switch case line
        if let Some(case_str) = cs2_content.strip_prefix("case ") {
            let parts: Vec<&str> = case_str.split_whitespace().collect();
            if parts.len() >= 2 {
                let value = parts[0].parse::<i32>().context("invalid case value")?;
                let target = parts[1].parse::<i32>().context("invalid case target")?;
                pending_switch_cases.push(SwitchCase { value, target });
            }
            pending_switch_open = true;
            continue;
        }

        // Handle switch instruction marker (starts a switch body)
        if cs2_content == "switch" {
            flush_pending_switch(
                &mut instructions,
                &mut pending_switch_cases,
                &mut pending_switch_open,
            );
            pending_switch_open = true;
            continue;
        }

        // Regular instruction — flush any pending switch first
        flush_pending_switch(
            &mut instructions,
            &mut pending_switch_cases,
            &mut pending_switch_open,
        );

        let (opcode_name, operand_text) = cs2_content
            .split_once(' ')
            .map_or((cs2_content, ""), |(a, b)| (a, b.trim()));

        instructions.push(Instruction {
            opcode: 0,
            command: opcode_name.to_string(),
            operand: parse_operand_asm(opcode_name, operand_text)?,
        });
    }

    flush_pending_switch(
        &mut instructions,
        &mut pending_switch_cases,
        &mut pending_switch_open,
    );

    Ok(CompiledScript {
        name,
        local_count_int,
        local_count_object,
        local_count_long,
        argument_count_int,
        argument_count_object,
        argument_count_long,
        code: instructions,
    })
}

fn parse_counts(input: &str, int: &mut u16, obj: &mut u16, long: &mut u16) -> Result<()> {
    for part in input.split_whitespace() {
        if let Some((key, val)) = part.split_once('=') {
            let v = val.parse::<u16>().context("invalid count value")?;
            match key {
                "int" => *int = v,
                "obj" => *obj = v,
                "long" => *long = v,
                _ => {}
            }
        }
    }
    Ok(())
}

fn flush_pending_switch(
    instructions: &mut Vec<Instruction>,
    pending: &mut Vec<SwitchCase>,
    pending_open: &mut bool,
) {
    if *pending_open {
        instructions.push(Instruction {
            opcode: 0,
            command: "switch".to_string(),
            operand: Operand::Switch(std::mem::take(pending)),
        });
        *pending_open = false;
    }
}

fn parse_operand_asm(opcode_name: &str, operand_text: &str) -> Result<Operand> {
    if operand_text.is_empty() {
        // No-operand commands like add, sub, return, neg
        return Ok(Operand::Int(0)); // sentinel, but won't be written
    }

    match opcode_name {
        "push_constant_int" => Ok(Operand::Int(
            operand_text.parse::<i32>().context("expected integer")?,
        )),
        "push_long_constant" => Ok(Operand::Long(
            operand_text
                .parse::<i64>()
                .context("expected long integer")?,
        )),
        "push_constant_string" => {
            if let Some((type_tag, rest)) = operand_text.split_once(':') {
                match type_tag {
                    "int" => Ok(Operand::Int(
                        rest.parse::<i32>()
                            .context("expected int for string constant")?,
                    )),
                    "long" => Ok(Operand::Long(
                        rest.parse::<i64>()
                            .context("expected long for string constant")?,
                    )),
                    "str" => {
                        if rest.starts_with('"') && rest.ends_with('"') {
                            let s = &rest[1..rest.len() - 1];
                            Ok(Operand::Str(
                                s.replace("\\n", "\n")
                                    .replace("\\\"", "\"")
                                    .replace("\\\\", "\\"),
                            ))
                        } else {
                            Ok(Operand::Str(rest.to_string()))
                        }
                    }
                    _ => bail!("unknown constant string type tag: {type_tag}"),
                }
            } else if operand_text.starts_with('"') && operand_text.ends_with('"') {
                let s = &operand_text[1..operand_text.len() - 1];
                Ok(Operand::Str(
                    s.replace("\\n", "\n")
                        .replace("\\\"", "\"")
                        .replace("\\\\", "\\"),
                ))
            } else if let Ok(v) = operand_text.parse::<i32>() {
                Ok(Operand::Int(v))
            } else {
                Ok(Operand::Str(operand_text.to_string()))
            }
        }
        "push_int_local" | "pop_int_local" | "push_string_local" | "pop_string_local"
        | "push_long_local" | "pop_long_local" => Ok(Operand::Local(
            operand_text
                .parse::<i32>()
                .context("expected local index")?,
        )),
        "push_var" | "pop_var" => {
            if let Some((domain_str, rest)) = operand_text.split_once(':') {
                let (id_str, transmog) = if let Some((id_part, _)) = rest.split_once(":transmog") {
                    (id_part, true)
                } else {
                    (rest, false)
                };
                let domain = VarDomain::from_label(domain_str).context("unknown var domain")?;
                let id = id_str.parse::<u16>().context("expected var id")?;
                Ok(Operand::VarRef(VarRef {
                    domain,
                    id,
                    transmog,
                }))
            } else {
                bail!("expected domain:id format for var ref, got: {operand_text}")
            }
        }
        "push_varbit" | "pop_varbit" => {
            if let Some((id_str, _)) = operand_text.split_once(":transmog") {
                let id = id_str.parse::<u16>().context("expected varbit id")?;
                Ok(Operand::VarBitRef(VarBitRef { id, transmog: true }))
            } else {
                let id = operand_text.parse::<u16>().context("expected varbit id")?;
                Ok(Operand::VarBitRef(VarBitRef {
                    id,
                    transmog: false,
                }))
            }
        }
        cmd if fixed_var_command_domain(cmd).is_some() => {
            parse_fixed_var_ref_operand(cmd, operand_text)
        }
        cmd if is_fixed_varbit_command(cmd) => parse_fixed_varbit_operand(cmd, operand_text),
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
        | "branch_if_false" => Ok(Operand::Branch(
            operand_text
                .parse::<i32>()
                .context("expected branch target")?,
        )),
        "switch" => {
            Ok(Operand::Int(0)) // dummy, switch handling is special
        }
        "join_string" => Ok(Operand::Count(
            operand_text.parse::<i32>().context("expected count")?,
        )),
        "gosub_with_params" => Ok(Operand::Script(
            operand_text.parse::<i32>().context("expected script id")?,
        )),
        "define_array"
        | "push_array_int"
        | "pop_array_int"
        | "push_array_int_leave_index_on_stack"
        | "push_array_int_and_index"
        | "pop_array_int_leave_value_on_stack" => Ok(Operand::Array(
            operand_text.parse::<i32>().context("expected array id")?,
        )),
        _ => {
            if let Some(raw32) = operand_text.strip_prefix("raw32:") {
                return Ok(Operand::Int(
                    raw32.parse::<i32>().context("expected raw32 operand")?,
                ));
            }
            // Unknown command: 1-byte operand
            if let Ok(v) = operand_text.parse::<i32>() {
                Ok(Operand::Byte(v as u8))
            } else {
                Ok(Operand::Byte(0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn assert_operand_eq(a: &Operand, b: &Operand) {
        match (a, b) {
            (Operand::Int(av), Operand::Int(bv)) => assert_eq!(av, bv),
            (Operand::Long(av), Operand::Long(bv)) => assert_eq!(av, bv),
            (Operand::Str(av), Operand::Str(bv)) => assert_eq!(av, bv),
            (Operand::Local(av), Operand::Local(bv)) => assert_eq!(av, bv),
            (Operand::VarRef(av), Operand::VarRef(bv)) => {
                assert_eq!(av.domain, bv.domain);
                assert_eq!(av.id, bv.id);
                assert_eq!(av.transmog, bv.transmog);
            }
            (Operand::VarBitRef(av), Operand::VarBitRef(bv)) => {
                assert_eq!(av.id, bv.id);
                assert_eq!(av.transmog, bv.transmog);
            }
            (Operand::Branch(av), Operand::Branch(bv)) => assert_eq!(av, bv),
            (Operand::Switch(av), Operand::Switch(bv)) => {
                assert_eq!(av.len(), bv.len());
                for (ac, bc) in av.iter().zip(bv.iter()) {
                    assert_eq!(ac.value, bc.value);
                    assert_eq!(ac.target, bc.target);
                }
            }
            (Operand::Script(av), Operand::Script(bv)) => assert_eq!(av, bv),
            (Operand::Array(av), Operand::Array(bv)) => assert_eq!(av, bv),
            (Operand::Count(av), Operand::Count(bv)) => assert_eq!(av, bv),
            (Operand::Byte(av), Operand::Byte(bv)) => assert_eq!(av, bv),
            _ => panic!("operand type mismatch: {a:?} vs {b:?}"),
        }
    }

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
    }

    #[test]
    fn opcode_book_loads_910_client_opcode_metadata() -> Result<()> {
        let opcode_book = OpcodeBook::load(&test_data_dir(), 910, 0)?;
        assert_eq!(opcode_book.opcode_for("push_constant_string")?, 1376);
        assert_eq!(opcode_book.opcode_for("if_sendtofront")?, 12);
        assert_eq!(opcode_book.opcode_for("db_find_with_count")?, 593);
        assert_eq!(opcode_book.name(1144)?, "field5245");
        assert!(opcode_book.has_large_operand(1376));
        assert!(!opcode_book.has_large_operand(842));
        Ok(())
    }

    #[test]
    fn asm_roundtrip_preserves_unknown_raw32_operands() -> Result<()> {
        let script = CompiledScript {
            name: Some("raw32_test".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![Instruction {
                opcode: 0,
                command: "field6317".to_string(),
                operand: Operand::Int(0x1234_5678),
            }],
        };
        let asm = script_to_asm(&script);
        assert!(asm.contains("field6317 raw32:305419896"));
        let reparsed = parse_cs2_asm(&asm)?;
        assert_eq!(reparsed.code.len(), 1);
        assert_operand_eq(&script.code[0].operand, &reparsed.code[0].operand);
        Ok(())
    }

    #[test]
    fn fixed_domain_var_operands_parse_domain_and_legacy_integer_forms() -> Result<()> {
        assert_operand_eq(
            &parse_operand_asm("push_varc_string", "client:42")?,
            &Operand::VarRef(VarRef {
                domain: VarDomain::Client,
                id: 42,
                transmog: false,
            }),
        );
        assert_operand_eq(
            &parse_operand_asm("push_varclan", "77")?,
            &Operand::VarRef(VarRef {
                domain: VarDomain::Clan,
                id: 77,
                transmog: false,
            }),
        );
        assert_operand_eq(
            &parse_operand_asm("push_varclansetting_string", "clan_setting:99")?,
            &Operand::VarRef(VarRef {
                domain: VarDomain::ClanSetting,
                id: 99,
                transmog: false,
            }),
        );
        assert_operand_eq(
            &parse_operand_asm("push_varclanbit", "123")?,
            &Operand::VarBitRef(VarBitRef {
                id: 123,
                transmog: false,
            }),
        );
        Ok(())
    }

    #[test]
    fn asm_roundtrip_preserves_all_operand_types() {
        let script = CompiledScript {
            name: Some("test_script".to_string()),
            local_count_int: 3,
            local_count_object: 1,
            local_count_long: 0,
            argument_count_int: 1,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".into(),
                    operand: Operand::Int(42),
                },
                Instruction {
                    opcode: 0,
                    command: "push_int_local".into(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "add".into(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".into(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_long_constant".into(),
                    operand: Operand::Long(999),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".into(),
                    operand: Operand::Str("hello\nworld".into()),
                },
                Instruction {
                    opcode: 0,
                    command: "push_long_local".into(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_var".into(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::Player,
                        id: 1234,
                        transmog: true,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_var".into(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::Client,
                        id: 42,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varbit".into(),
                    operand: Operand::VarBitRef(VarBitRef {
                        id: 5678,
                        transmog: true,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varc_string".into(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::Client,
                        id: 9,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_varc_string".into(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::Client,
                        id: 10,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varclan".into(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::Clan,
                        id: 11,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varclansetting_long".into(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::ClanSetting,
                        id: 12,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varclanbit".into(),
                    operand: Operand::VarBitRef(VarBitRef {
                        id: 13,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "branch_if_true".into(),
                    operand: Operand::Branch(15),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".into(),
                    operand: Operand::Script(9999),
                },
                Instruction {
                    opcode: 0,
                    command: "define_array".into(),
                    operand: Operand::Array(0),
                },
                Instruction {
                    opcode: 0,
                    command: "join_string".into(),
                    operand: Operand::Count(3),
                },
                Instruction {
                    opcode: 0,
                    command: "branch_equals".into(),
                    operand: Operand::Branch(7),
                },
                Instruction {
                    opcode: 0,
                    command: "switch".into(),
                    operand: Operand::Switch(vec![
                        SwitchCase {
                            value: 0,
                            target: 10,
                        },
                        SwitchCase {
                            value: 1,
                            target: 20,
                        },
                    ]),
                },
                Instruction {
                    opcode: 0,
                    command: "return".into(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let asm = script_to_asm(&script);
        let parsed = parse_cs2_asm(&asm).expect("parse_cs2_asm should succeed");

        assert_eq!(script.name, parsed.name);
        assert_eq!(script.local_count_int, parsed.local_count_int);
        assert_eq!(script.local_count_object, parsed.local_count_object);
        assert_eq!(script.local_count_long, parsed.local_count_long);
        assert_eq!(script.argument_count_int, parsed.argument_count_int);
        assert_eq!(script.argument_count_object, parsed.argument_count_object);
        assert_eq!(script.argument_count_long, parsed.argument_count_long);
        assert_eq!(
            script.code.len(),
            parsed.code.len(),
            "instruction count mismatch"
        );

        for (i, (orig, round)) in script.code.iter().zip(parsed.code.iter()).enumerate() {
            assert_eq!(orig.command, round.command, "command mismatch at index {i}");
            assert_operand_eq(&orig.operand, &round.operand);
        }
    }

    #[test]
    fn asm_roundtrip_preserves_empty_switch() -> Result<()> {
        let script = CompiledScript {
            name: None,
            local_count_int: 1,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_int_local".into(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "switch".into(),
                    operand: Operand::Switch(vec![]),
                },
                Instruction {
                    opcode: 0,
                    command: "return".into(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let asm = script_to_asm(&script);
        let parsed = parse_cs2_asm(&asm)?;

        assert_eq!(parsed.code.len(), 3);
        assert_eq!(parsed.code[1].command, "switch");
        assert_operand_eq(&script.code[1].operand, &parsed.code[1].operand);
        Ok(())
    }
}
