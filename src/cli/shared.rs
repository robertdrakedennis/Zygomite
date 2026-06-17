//! Helpers shared by 2+ command handlers.
//!
//! These were private free functions in the old `cli.rs` god-module, used
//! across several `run_*` handlers. They are hoisted here so each command module
//! in [`crate::commands`] can use them without duplication. The module is
//! `pub(crate)`, so these `pub` items are reachable crate-wide but not outside.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::io::{BufWriter, Write as IoWrite};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use std::collections::HashSet;

use crate::cache::FlatCache;
use crate::constants::ARCHIVE_CLIENTSCRIPTS;
use crate::dep_tree::ResolverContext;
use crate::script::{CompiledScript, Instruction, Operand, VarBitRef, VarRef};
use crate::transpile::{
    ReverseCompileContext, ScriptCatalog, ScriptCatalogBuilder, enum_pair_property_name,
};

// ---------------------------------------------------------------------------
// Filesystem writers
// ---------------------------------------------------------------------------

pub fn write_binary(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    fs::write(path, data).with_context(|| format!("failed writing {}", path.display()))
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("failed writing {}", path.display()))
}

pub fn write_lines(path: &Path, lines: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }

    let file =
        fs::File::create(path).with_context(|| format!("failed writing {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            writer.write_all(b"\n")?;
        }
        writer.write_all(line.as_bytes())?;
    }
    writer
        .flush()
        .with_context(|| format!("failed writing {}", path.display()))
}

pub struct TextFileWriter {
    path: PathBuf,
    writer: BufWriter<fs::File>,
    wrote_line: bool,
}

impl TextFileWriter {
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating {}", parent.display()))?;
        }

        let file =
            fs::File::create(path).with_context(|| format!("failed writing {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
            wrote_line: false,
        })
    }

    pub fn line(&mut self, line: impl AsRef<str>) -> Result<()> {
        if self.wrote_line {
            self.writer.write_all(b"\n")?;
        }
        self.writer.write_all(line.as_ref().as_bytes())?;
        self.wrote_line = true;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.writer
            .flush()
            .with_context(|| format!("failed writing {}", self.path.display()))
    }
}

pub fn write_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(data).context("failed to encode json")?;
    fs::write(path, json).with_context(|| format!("failed writing {}", path.display()))
}

pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to encode summary json")?
    );
    Ok(())
}

pub fn write_jsonl_file<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    let file = fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for row in rows {
        serde_json::to_writer(&mut writer, row)
            .with_context(|| format!("writing {}", path.display()))?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// TypeScript string / identifier helpers (shared by ts-export + transpile)
// ---------------------------------------------------------------------------

pub fn escape_ts_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Convert a RS3 interface component name (`snake_case` or kebab-case)
/// to a valid TypeScript object property name (also `snake_case`, but
/// with hyphens and spaces replaced by underscores).
pub fn sanitize_ts_prop(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if c == '-' || c == ' ' || c == '/' {
            out.push('_');
        }
        // drop other chars
    }
    // Property can't start with a digit
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    // Can't be empty
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

// ---------------------------------------------------------------------------
// CS2 listing formatting (shared by `cs2` dump + transpile + ts-export)
// ---------------------------------------------------------------------------

pub fn format_script_source(group: u32, file: u32, script: &CompiledScript) -> String {
    let mut out = String::new();
    let script_name = script.name.as_deref().unwrap_or("null");
    let _ = writeln!(out, "// group={group} file={file}");
    let _ = writeln!(out, "// name={script_name}");
    let _ = writeln!(
        out,
        "// locals int={} object={} long={}",
        script.local_count_int, script.local_count_object, script.local_count_long
    );
    let _ = writeln!(
        out,
        "// args int={} object={} long={}",
        script.argument_count_int, script.argument_count_object, script.argument_count_long
    );
    for (index, instruction) in script.code.iter().enumerate() {
        let _ = writeln!(out, "{index:05}: {}", format_instruction(instruction));
    }
    out
}

pub fn format_instruction(instruction: &Instruction) -> String {
    format!(
        "{} {}",
        instruction.command,
        format_operand(&instruction.operand)
    )
    .trim_end()
    .to_string()
}

pub fn format_operand(operand: &Operand) -> String {
    match operand {
        Operand::Int(value) => value.to_string(),
        Operand::Long(value) => value.to_string(),
        Operand::Str(value) => format!("\"{}\"", escape_string(value)),
        Operand::Local(value) => format!("local_{value}"),
        Operand::VarRef(value) => {
            let mut tag = format!("{}:{}", value.domain.as_label(), value.id);
            if value.transmog {
                tag.push_str(":transmog");
            }
            tag
        }
        Operand::VarBitRef(value) => {
            let mut tag = format!("varbit:{}", value.id);
            if value.transmog {
                tag.push_str(":transmog");
            }
            tag
        }
        Operand::Branch(value) => format!("->{value}"),
        Operand::Switch(cases) => {
            let mut text = String::new();
            text.push('{');
            for (index, case) in cases.iter().enumerate() {
                if index != 0 {
                    text.push_str(", ");
                }
                let _ = write!(text, "{}->{}", case.value, case.target);
            }
            text.push('}');
            text
        }
        Operand::Script(value) => format!("script_{value}"),
        Operand::Array(value) => format!("array_{value}"),
        Operand::Count(value) => format!("count_{value}"),
        Operand::Byte(value) => value.to_string(),
    }
}

pub fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('"', "\\\"")
}

pub fn sanitize_file_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "script".to_string()
    } else {
        out
    }
}

// ---------------------------------------------------------------------------
// Script-group / hash-name maps (shared by `cs2`, ts-export, transpile)
// ---------------------------------------------------------------------------

pub fn load_script_group_names(
    index: &crate::js5::ArchiveIndex,
    data_dir: &Path,
) -> Result<HashMap<u32, String>> {
    let Some(group_hashes) = &index.group_name_hash else {
        return Ok(HashMap::new());
    };

    let names_path = data_dir.join("names/scripts.txt");
    if !names_path.is_file() {
        return Ok(HashMap::new());
    }

    let hash_names = load_hash_name_map(&names_path)?;
    let mut by_group = HashMap::new();
    for group in &index.group_id {
        let idx = usize::try_from(*group).context("script group index overflow")?;
        let hash = *group_hashes
            .get(idx)
            .with_context(|| format!("missing group hash slot for {group}"))?;
        if hash == -1 {
            continue;
        }
        if let Some(name) = hash_names.get(&hash) {
            by_group.insert(*group, extract_name_suffix(name));
        }
    }
    Ok(by_group)
}

pub fn load_script_group_names_from_cache(
    cache: &FlatCache,
    data_dir: &Path,
) -> Result<HashMap<u32, String>> {
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    load_script_group_names(&index, data_dir)
}

pub fn load_hash_name_map(path: &Path) -> Result<HashMap<i32, String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed reading {}", path.display()))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        expand_name_pattern(name, &mut map);
    }
    Ok(map)
}

pub fn expand_name_pattern(name: &str, out: &mut HashMap<i32, String>) {
    if let Some(index) = name.find('#') {
        let prefix = &name[..index];
        let suffix = &name[index + 1..];
        for value in 0..500 {
            let expanded = format!("{prefix}{value}{suffix}");
            expand_name_pattern(&expanded, out);
        }
    } else {
        out.insert(java_string_hash(name), name.to_string());
    }
}

pub fn java_string_hash(value: &str) -> i32 {
    let mut hash = 0_i32;
    for c in value.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as i32);
    }
    hash
}

pub fn extract_name_suffix(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if let Some((_, suffix)) = inner.split_once(',') {
            return suffix.to_string();
        }
    }
    trimmed.to_string()
}

// ---------------------------------------------------------------------------
// Worldmap value formatting (shared by `unpack` worldmap dump + ts-export enums)
// ---------------------------------------------------------------------------

pub fn format_coordgrid(value: i32) -> String {
    if value == -1 {
        return String::from("null");
    }
    let as_u32 = value as u32;
    let level = as_u32 >> 28;
    let x = (as_u32 >> 14) & 0x3fff;
    let z = as_u32 & 0x3fff;
    format!("{level}_{}_{}_{}_{}", x / 64, z / 64, x % 64, z % 64)
}

pub fn format_colour(value: i32) -> String {
    let as_u32 = value as u32;
    if as_u32 > 0x00ff_ffff {
        format!("0x{as_u32:08x}")
    } else {
        format!("0x{as_u32:06x}")
    }
}

pub fn format_map_element(value: u16) -> String {
    format!("mapelement_{value}")
}

/// Classify why a generated `high-ts` source still carries low-level control
/// flow (used by both the transpile fallback path and the ts-export enum dump).
pub fn source_control_flow_fallback_reason(source: &str) -> Option<String> {
    if source
        .lines()
        .any(|line| line.contains("stackpush_then(") && line.contains("goto("))
    {
        Some("stack_goto".to_string())
    } else if source.contains("goto(") || source.contains("label(") {
        Some("residual_goto".to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Reverse-compile context (shared by `assemble-script` + transpile)
// ---------------------------------------------------------------------------

pub fn build_reverse_compile_context(
    ctx: &ResolverContext,
    cache: &FlatCache,
    data_dir: &Path,
) -> Result<ReverseCompileContext> {
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    let mut builder = ScriptCatalogBuilder::new(&script_group_names, &ctx.opcode_book, ctx.build);
    for (&packed_id_raw, data) in &ctx.scripts {
        builder.add_script(packed_id_raw, data);
    }
    Ok(build_reverse_compile_context_from_catalog(
        ctx,
        builder.build(),
    ))
}

pub fn build_reverse_compile_context_from_catalog(
    ctx: &ResolverContext,
    script_catalog: ScriptCatalog,
) -> ReverseCompileContext {
    let mut var_refs_by_name = HashMap::new();
    for (domain, vars) in &ctx.varps_by_domain {
        for (&id, entry) in vars {
            var_refs_by_name.insert(
                entry.var_name.clone(),
                VarRef {
                    domain: *domain,
                    id: id as u16,
                    transmog: false,
                },
            );
            var_refs_by_name.insert(
                format!("{}_transmog", entry.var_name),
                VarRef {
                    domain: *domain,
                    id: id as u16,
                    transmog: true,
                },
            );
        }
    }

    let mut varbit_refs_by_name = HashMap::new();
    for (&id, entry) in &ctx.varbits {
        let id = id as u16;
        varbit_refs_by_name.insert(
            entry.varbit_name.clone(),
            VarBitRef {
                id,
                transmog: false,
            },
        );
        varbit_refs_by_name.insert(
            format!("{}_transmog", entry.varbit_name),
            VarBitRef { id, transmog: true },
        );
        varbit_refs_by_name.insert(
            format!("varbit_{id}"),
            VarBitRef {
                id,
                transmog: false,
            },
        );
        varbit_refs_by_name.insert(
            format!("varbit_{id}_transmog"),
            VarBitRef { id, transmog: true },
        );
    }

    let string_param_ids = ctx
        .params
        .values()
        .filter(|entry| {
            matches!(entry.type_char, Some(b's' | b'S'))
                || matches!(entry.default, Some(crate::config::ScalarValue::Str(_)))
        })
        .map(|entry| entry.id as i32)
        .collect::<HashSet<_>>();

    let mut enum_values_by_name = HashMap::new();
    for entry in ctx.enums.values() {
        let object_name = format!("Enum_{}", entry.id);
        let mut used_properties = HashSet::new();
        for pair in &entry.values {
            let property_name =
                enum_pair_property_name(&pair.value, pair.key, &mut used_properties);
            enum_values_by_name.insert(format!("{object_name}.{property_name}"), pair.key);
        }
    }

    let mut component_ids_by_name = HashMap::new();
    for (&interface_id, components) in &ctx.parsed_components {
        for (&component_id, deps) in components {
            let property_name = deps
                .name
                .as_deref()
                .map(sanitize_ts_prop)
                .filter(|prop| !prop.is_empty() && prop != "unnamed")
                .unwrap_or_else(|| {
                    sanitize_ts_prop(&crate::interface::component_fallback_name(
                        interface_id,
                        component_id,
                    ))
                });
            component_ids_by_name.insert(
                format!("ComponentId.{property_name}"),
                crate::interface::component_uid(interface_id, component_id) as i32,
            );
        }
    }

    ReverseCompileContext {
        build: ctx.build,
        script_signatures: script_catalog.signature_map(),
        script_catalog,
        var_refs_by_name,
        varbit_refs_by_name,
        string_param_ids,
        enum_values_by_name,
        component_ids_by_name,
        opcode_commands: ctx.opcode_book.commands().map(str::to_string).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::Operand;

    #[test]
    fn sanitize_file_component_rewrites_unsupported_chars() {
        assert_eq!("hello_world", sanitize_file_component("hello/world"));
        assert_eq!("script", sanitize_file_component(""));
    }

    #[test]
    fn extract_name_suffix_parses_tag_syntax() {
        assert_eq!(
            "100guide_flour_drawitems",
            extract_name_suffix("[clientscript,100guide_flour_drawitems]")
        );
        assert_eq!("plain_name", extract_name_suffix("plain_name"));
    }

    #[test]
    fn java_string_hash_matches_known_value() {
        assert_eq!(2_111_159_123, java_string_hash("[clientscript,script0]"));
    }

    #[test]
    fn worldmap_format_helpers_match_expected_shape() {
        assert_eq!("null", format_coordgrid(-1));
        assert_eq!("0_0_0_0_0", format_coordgrid(0));
        assert_eq!("0_50_248_42_54", format_coordgrid(53_132_854));
        assert_eq!("0x00ab12", format_colour(43_794));
        assert_eq!("0xff00ab12", format_colour(-16_733_422));
        assert_eq!("mapelement_42", format_map_element(42));
    }

    #[test]
    fn source_control_flow_fallback_reason_classifies_stack_and_residual_gotos() {
        assert_eq!(
            Some("stack_goto".to_string()),
            source_control_flow_fallback_reason("stackpush_then(1, goto(2));")
        );
        assert_eq!(
            Some("residual_goto".to_string()),
            source_control_flow_fallback_reason("stackpush_then(1, push(x));\ngoto(2);")
        );
        assert_eq!(None, source_control_flow_fallback_reason("return;"));
    }

    #[test]
    fn format_script_source_renders_headers_and_code() {
        let script = CompiledScript {
            name: Some("my/script".to_string()),
            local_count_int: 1,
            local_count_object: 2,
            local_count_long: 3,
            argument_count_int: 4,
            argument_count_object: 5,
            argument_count_long: 6,
            code: vec![Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(42),
            }],
        };

        let source = format_script_source(10, 0, &script);
        assert!(source.contains("// group=10 file=0"));
        assert!(source.contains("// name=my/script"));
        assert!(source.contains("00000: push_constant_int 42"));
    }
}
