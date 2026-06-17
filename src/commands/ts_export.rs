//! `ts-export` — generate the full set of TypeScript id/type modules (vars,
//! varbits, enums, structs, params, interfaces, configs, db tables, script
//! signatures) from the cache.
//!
//! The `export_*_types` helpers double as the data source for the transpile
//! command, so the three it consumes are `pub`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::{
    TextFileWriter, escape_ts_string, load_script_group_names_from_cache, sanitize_ts_prop,
    write_lines, write_text,
};
use crate::constants::ARCHIVE_CLIENTSCRIPTS;
use crate::dep_tree::ResolverContext;
use crate::script::{OpcodeBook, decode_script};
use crate::transpile::enum_pair_property_name;

fn export_script_signatures_from_cache(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    opcode_book: &OpcodeBook,
    build: u32,
    group_names: &HashMap<u32, String>,
) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated CS2 script signatures".to_string(),
        "// Source: RS3 cache clientscript archive".to_string(),
        String::new(),
    ];
    let mut entries: Vec<(String, String)> = Vec::new();

    if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CLIENTSCRIPTS)
        .is_err()
    {
        return write_lines(&out_dir.join("scripts.d.ts"), &lines);
    }

    let cache2 = FlatCache::open(cache.root())?;
    let index = cache2.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    for group in &index.group_id {
        let files = cache2.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
        for (_file, data) in files {
            let Ok(script) = decode_script(&data, opcode_book, build) else {
                continue;
            };
            let display_name = script
                .name
                .as_deref()
                .map(crate::transpile::extract_script_name_suffix)
                .filter(|name| !name.is_empty())
                .or_else(|| group_names.get(group).cloned());
            // Name by group (`script<group>`), matching the catalog and the
            // transpiled output files — not the packed id.
            let function_name = crate::transpile::script_function_name(
                crate::transpile::ScriptId(*group as i32),
                display_name.as_deref(),
            );
            let mut arg_types: Vec<&str> = Vec::new();
            arg_types.extend(std::iter::repeat_n(
                "number",
                script.argument_count_int as usize,
            ));
            arg_types.extend(std::iter::repeat_n(
                "string",
                script.argument_count_object as usize,
            ));
            arg_types.extend(std::iter::repeat_n(
                "bigint",
                script.argument_count_long as usize,
            ));
            let args = (0..arg_types.len())
                .map(|index| format!("arg{index}: {}", arg_types[index]))
                .collect::<Vec<_>>()
                .join(", ");
            entries.push((
                function_name.clone(),
                format!("export function {function_name}({args}): unknown;"),
            ));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    lines.extend(entries.into_iter().map(|(_, line)| line));
    write_lines(&out_dir.join("scripts.d.ts"), &lines)
}

/// Options for `ts-export`.
#[derive(Clone, Debug)]
pub struct TsExportOpts {
    pub out_dir: std::path::PathBuf,
}

/// `ts-export` — write the full set of generated TypeScript id/type modules.
pub fn run(ctx: &CommandContext, opts: TsExportOpts) -> Result<()> {
    let TsExportOpts { out_dir } = opts;
    let out_dir = out_dir.as_path();
    let cache = ctx.cache();
    let resolver = ResolverContext::load_ts_export(
        cache,
        ctx.tar_path(),
        ctx.data_dir(),
        ctx.build(),
        ctx.subbuild(),
    )?;
    let opcode_book = resolver.opcode_book.clone();
    let script_group_names = load_script_group_names_from_cache(cache, ctx.data_dir())?;
    fs::create_dir_all(out_dir)?;

    export_var_types(&resolver, out_dir)?;
    export_varbit_types(&resolver, out_dir)?;
    export_enum_types(&resolver, out_dir)?;
    export_struct_types(&resolver, out_dir)?;
    export_param_types(&resolver, out_dir)?;
    export_interface_ids(&resolver, out_dir)?;
    export_inv_types(&resolver, out_dir)?;
    export_obj_types(&resolver, out_dir)?;
    export_npc_types(&resolver, out_dir)?;
    export_loc_types(&resolver, out_dir)?;
    export_seq_types(&resolver, out_dir)?;
    export_spot_types(&resolver, out_dir)?;
    export_named_config_ids(&resolver, out_dir)?;
    export_db_types(&resolver, out_dir)?;
    export_script_signatures_from_cache(
        cache,
        ctx.tar_path(),
        out_dir,
        &opcode_book,
        ctx.build(),
        &script_group_names,
    )?;
    export_index(out_dir)?;

    eprintln!("typescript definitions exported to {}", out_dir.display());
    Ok(())
}

pub fn export_var_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx
        .varps_by_domain
        .iter()
        .flat_map(|(domain, vars)| {
            vars.values().map(|entry| VarTypeEntry {
                id: entry.id,
                domain: *domain,
                var_name: entry.var_name.clone(),
                type_id: entry.type_id,
                lifetime: entry.lifetime,
                transmit_level: entry.transmit_level,
                client_code: entry.client_code,
                domain_default: entry.domain_default,
                wiki_sync: entry.wiki_sync,
            })
        })
        .collect();

    entries.sort_by_key(|e| (e.domain as u8, e.id));

    let mut lines = vec![
        "// Auto-generated Var definitions".to_string(),
        "// Source: RS3 cache var config".to_string(),
        String::new(),
        "export type VarDomain = 'player' | 'npc' | 'client' | 'world' | 'region' | 'object' | 'clan' | 'clan_setting' | 'controller' | 'player_group' | 'global';".to_string(),
        "export type VarType = 'int' | 'long' | 'string' | 'unknown';".to_string(),
        "export type VarLifetime = 'temp' | 'perm' | 'serverperm' | 'unknown';".to_string(),
        "export type VarTransmitLevel = 'never' | 'on_set_different' | 'on_set_always' | 'unknown';".to_string(),
        String::new(),
        "export interface VarEntry {".to_string(),
        "    id: number;".to_string(),
        "    domain: VarDomain;".to_string(),
        "    name: string;".to_string(),
        "    type: VarType;".to_string(),
        "    lifetime: VarLifetime;".to_string(),
        "    transmitLevel: VarTransmitLevel;".to_string(),
        "    clientCode: number | null;".to_string(),
        "    domainDefault: boolean;".to_string(),
        "    wikiSync: boolean;".to_string(),
        "}".to_string(),
        String::new(),
        // Use composite key: domain_id * 1000000 + var_id
        "export const VARS: ReadonlyMap<number, VarEntry> = new Map([".to_string(),
    ];
    for entry in &entries {
        let type_str = match entry.type_id {
            Some(0) => "'int'",
            Some(1) => "'long'",
            Some(2) => "'string'",
            _ => "'unknown'",
        };
        let lifetime = entry.lifetime.unwrap_or("unknown");
        let transmit = entry.transmit_level.unwrap_or("unknown");
        let client_code = match entry.client_code {
            Some(c) => c.to_string(),
            None => "null".to_string(),
        };
        let domain_label = entry.domain.as_label();
        let composite_key = (u64::from(entry.domain) * 1_000_000) + u64::from(entry.id);
        lines.push(format!(
            "    [{}, {{ id: {}, domain: '{}', name: '{}', type: {}, lifetime: '{}', transmitLevel: '{}', clientCode: {}, domainDefault: {}, wikiSync: {} }}],",
            composite_key,
            entry.id,
            domain_label,
            escape_ts_string(&entry.var_name),
            type_str,
            lifetime,
            transmit,
            client_code,
            entry.domain_default,
            entry.wiki_sync
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const VAR_COUNT = {};", entries.len()));

    write_lines(&out_dir.join("vars.ts"), &lines)
}

struct VarTypeEntry {
    id: u32,
    domain: crate::vars::VarDomain,
    var_name: String,
    type_id: Option<u8>,
    lifetime: Option<&'static str>,
    transmit_level: Option<&'static str>,
    client_code: Option<u16>,
    domain_default: bool,
    wiki_sync: bool,
}

pub fn export_varbit_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated VarBit definitions".to_string(),
        "// Source: RS3 cache varbit config".to_string(),
        String::new(),
        "export interface VarBitEntry {".to_string(),
        "    id: number;".to_string(),
        "    name: string;".to_string(),
        "    domain: string | null;".to_string(),
        "    baseVar: number | null;".to_string(),
        "    startBit: number | null;".to_string(),
        "    endBit: number | null;".to_string(),
        "    wikiSync: boolean;".to_string(),
        "}".to_string(),
        String::new(),
    ];

    lines.push("export const VARBITS: ReadonlyMap<number, VarBitEntry> = new Map([".to_string());
    for entry in ctx.varbits.values() {
        let domain_str = match entry.domain {
            Some(d) => format!("'{}'", d.as_label()),
            None => "null".to_string(),
        };
        let base_var = entry
            .base_var
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let start_bit = entry
            .start_bit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let end_bit = entry
            .end_bit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        lines.push(format!(
            "    [{}, {{ id: {}, name: '{}', domain: {}, baseVar: {}, startBit: {}, endBit: {}, wikiSync: {} }}],",
            entry.id,
            entry.id,
            escape_ts_string(&entry.varbit_name),
            domain_str,
            base_var,
            start_bit,
            end_bit,
            entry.wiki_sync
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!(
        "export const VARBIT_COUNT = {};",
        ctx.varbits.len()
    ));

    write_lines(&out_dir.join("varbits.ts"), &lines)
}

pub fn export_enum_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("enums.ts"))?;
    writer.line("// Auto-generated Enum definitions")?;
    writer.line("// Source: RS3 cache enum config")?;
    writer.line("")?;

    // ── Per-enum const objects with named constants ──
    let mut reverse_lookup: Vec<(i32, String)> = Vec::new();

    for entry in ctx.enums.values() {
        if entry.values.is_empty() {
            continue;
        }
        let obj_name = format!("Enum_{id}", id = entry.id);
        let mut props: Vec<String> = Vec::new();
        let mut used_properties = HashSet::new();

        for pair in &entry.values {
            let unique_prop = enum_pair_property_name(&pair.value, pair.key, &mut used_properties);
            props.push(format!("    {unique_prop}: {key},", key = pair.key));
            reverse_lookup.push((pair.key, format!("{obj_name}.{unique_prop}")));
        }

        writer.line(format!("export const {obj_name} = {{"))?;
        for prop in props {
            writer.line(prop)?;
        }
        writer.line("} as const;")?;
        writer.line("")?;
    }

    // ── Reverse lookup: enum value → qualified name ──
    reverse_lookup.sort_by_key(|(k, _)| *k);
    reverse_lookup.dedup_by_key(|(k, _)| *k);
    if !reverse_lookup.is_empty() {
        writer.line("// Reverse lookup: maps enum key values to qualified names.")?;
        writer.line("export const ENUM_VALUE_TO_NAME: ReadonlyMap<number, string> = new Map([")?;
        for (key, name) in &reverse_lookup {
            writer.line(format!("    [{key}, '{name}'],"))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }

    // ── Existing types and runtime map ──
    writer.line("export interface EnumPair {")?;
    writer.line("    key: number;")?;
    writer.line("    value: number | string;")?;
    writer.line("    dense: boolean;")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface EnumEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    inputType: string;")?;
    writer.line("    outputType: string;")?;
    writer.line("    default: number | string | null;")?;
    writer.line("    values: EnumPair[];")?;
    writer.line("}")?;
    writer.line("")?;

    writer.line("export const ENUMS: ReadonlyMap<number, EnumEntry> = new Map([")?;
    for entry in ctx.enums.values() {
        let input_type = match entry.input_type_char {
            Some(b'i') => "'int'",
            Some(b's') => "'string'",
            _ => "'unknown'",
        };
        let output_type = match entry.output_type_char {
            Some(b'i') => "'int'",
            Some(b's') => "'string'",
            _ => "'unknown'",
        };
        let default = match &entry.default {
            Some(crate::config::ScalarValue::Int(i)) => i.to_string(),
            Some(crate::config::ScalarValue::Long(l)) => l.to_string(),
            Some(crate::config::ScalarValue::Str(s)) => format!("'{}'", escape_ts_string(s)),
            None => "null".to_string(),
        };
        let values_json: String = entry
            .values
            .iter()
            .map(|pair| {
                let val_str = match &pair.value {
                    crate::config::ScalarValue::Int(i) => i.to_string(),
                    crate::config::ScalarValue::Long(l) => l.to_string(),
                    crate::config::ScalarValue::Str(s) => format!("'{}'", escape_ts_string(s)),
                };
                format!(
                    "{{ key: {}, value: {}, dense: {} }}",
                    pair.key, val_str, pair.dense
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        writer.line(format!(
            "    [{}, {{ id: {}, inputType: {}, outputType: {}, default: {}, values: [{}] }}],",
            entry.id, entry.id, input_type, output_type, default, values_json
        ))?;
    }
    writer.line("]);")?;
    writer.line("")?;
    writer.line(format!("export const ENUM_COUNT = {};", ctx.enums.len()))?;
    writer.finish()
}

/// Convert a lowercase or mixed-case string value (e.g. "`skill_type`",
/// "my value") to `SCREAMING_SNAKE_CASE` for use as a
/// TypeScript const property name.
fn str_to_screaming_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if c == ' ' || c == '-' || c == '/' || c == '.' {
            out.push('_');
        }
    }
    // Trim leading/trailing underscores
    let trimmed = out.trim_matches('_');
    // Can't start with a digit
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{trimmed}")
    } else if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn export_struct_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Struct definitions".to_string(),
        "// Source: RS3 cache struct config".to_string(),
        String::new(),
        "export interface StructParamEntry {".to_string(),
        "    id: number;".to_string(),
        "    value: number | string;".to_string(),
        "}".to_string(),
        String::new(),
        "export interface StructEntry {".to_string(),
        "    id: number;".to_string(),
        "    params: StructParamEntry[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    lines.push("export const STRUCTS: ReadonlyMap<number, StructEntry> = new Map([".to_string());
    for entry in ctx.structs.values() {
        let params_json = entry
            .params
            .iter()
            .map(|p| {
                format!(
                    "{{ id: {}, value: {} }}",
                    p.param_id,
                    format_scalar_value(&p.value)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "    [{}, {{ id: {}, params: [{}] }}],",
            entry.id, entry.id, params_json
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!(
        "export const STRUCT_COUNT = {};",
        ctx.structs.len()
    ));

    write_lines(&out_dir.join("structs.ts"), &lines)
}

fn format_scalar_value(value: &crate::config::ScalarValue) -> String {
    match value {
        crate::config::ScalarValue::Int(i) => i.to_string(),
        crate::config::ScalarValue::Long(l) => l.to_string(),
        crate::config::ScalarValue::Str(s) => format!("'{}'", escape_ts_string(s)),
    }
}

pub fn export_param_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Param definitions".to_string(),
        "// Source: RS3 cache param config".to_string(),
        String::new(),
        "export interface ParamEntry {".to_string(),
        "    id: number;".to_string(),
        "    typeChar: string | null;".to_string(),
        "    typeId: number | null;".to_string(),
        "    defaultInt: number | null;".to_string(),
        "    defaultString: string | null;".to_string(),
        "    autoDisable: boolean;".to_string(),
        "}".to_string(),
        String::new(),
        "export type ParamValue = number | string;".to_string(),
        String::new(),
    ];

    lines.push("export const PARAMS: ReadonlyMap<number, ParamEntry> = new Map([".to_string());
    for entry in ctx.params.values() {
        let type_char = entry
            .type_char
            .map(|c| format!("'{}'", c as char))
            .unwrap_or_else(|| "null".to_string());
        let type_id = entry
            .type_id
            .map(|t| t.to_string())
            .unwrap_or_else(|| "null".to_string());
        let (default_int, default_string) = match &entry.default {
            Some(crate::config::ScalarValue::Int(i)) => (i.to_string(), "null".to_string()),
            Some(crate::config::ScalarValue::Long(l)) => (l.to_string(), "null".to_string()),
            Some(crate::config::ScalarValue::Str(s)) => {
                ("null".to_string(), format!("'{}'", escape_ts_string(s)))
            }
            None => ("null".to_string(), "null".to_string()),
        };
        lines.push(format!(
            "    [{}, {{ id: {}, typeChar: {}, typeId: {}, defaultInt: {}, defaultString: {}, autoDisable: {} }}],",
            entry.id, entry.id, type_char, type_id, default_int, default_string, entry.autodisable
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const PARAM_COUNT = {};", ctx.params.len()));

    write_lines(&out_dir.join("params.ts"), &lines)
}

pub fn export_inv_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Inventory definitions".to_string(),
        "// Source: RS3 cache inv config".to_string(),
        String::new(),
    ];

    lines.push("export interface InvStockEntry {".to_string());
    lines.push("    objId: number;".to_string());
    lines.push("    count: number;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("export interface InvEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    size: number | null;".to_string());
    lines.push("    stocks: InvStockEntry[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    if !ctx.invs.is_empty() {
        lines.push("export const INVS: ReadonlyMap<number, InvEntry> = new Map([".to_string());
        for entry in ctx.invs.values() {
            let size = entry
                .size
                .map(|s| s.to_string())
                .unwrap_or_else(|| "null".to_string());
            let stocks_json: String = entry
                .stocks
                .iter()
                .map(|s| format!("{{ objId: {}, count: {} }}", s.obj_id, s.count))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, size: {size}, stocks: [{stocks_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const INV_COUNT = {};", ctx.invs.len()));

    write_lines(&out_dir.join("invs.ts"), &lines)
}

pub fn export_obj_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("objs.ts"))?;
    writer.line("// Auto-generated Item (Obj) definitions")?;
    writer.line("// Source: RS3 cache obj config")?;
    writer.line("")?;
    writer.line("export interface ObjEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    ops: string[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.objs.is_empty() {
        writer.line("export const OBJS: ReadonlyMap<number, ObjEntry> = new Map([")?;
        for entry in ctx.objs.values() {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const OBJ_COUNT = {};", ctx.objs.len()))?;
    writer.finish()
}

pub fn export_npc_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("npcs.ts"))?;
    writer.line("// Auto-generated NPC definitions")?;
    writer.line("// Source: RS3 cache npc config")?;
    writer.line("")?;
    writer.line("export interface NpcEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    ops: string[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.npcs.is_empty() {
        writer.line("export const NPCS: ReadonlyMap<number, NpcEntry> = new Map([")?;
        for entry in ctx.npcs.values() {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const NPC_COUNT = {};", ctx.npcs.len()))?;
    writer.finish()
}

pub fn export_loc_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("locs.ts"))?;
    writer.line("// Auto-generated Loc (Object) definitions")?;
    writer.line("// Source: RS3 cache loc config")?;
    writer.line("")?;
    writer.line("export interface LocEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    ops: string[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.locs.is_empty() {
        writer.line("export const LOCS: ReadonlyMap<number, LocEntry> = new Map([")?;
        for entry in ctx.locs.values() {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const LOC_COUNT = {};", ctx.locs.len()))?;
    writer.finish()
}

pub fn export_seq_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("seqs.ts"))?;
    writer.line("// Auto-generated Sequence (Animation) definitions")?;
    writer.line("// Source: RS3 cache seq config")?;
    writer.line("")?;
    writer.line("export interface SeqFrame {")?;
    writer.line("    animId: number;")?;
    writer.line("    frameId: number;")?;
    writer.line("    delay: number;")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface SeqEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    frames: SeqFrame[];")?;
    writer.line("    stretches: boolean;")?;
    writer.line("    priority: number | null;")?;
    writer.line("    leftHand: number | null;")?;
    writer.line("    rightHand: number | null;")?;
    writer.line("    loopCount: number | null;")?;
    writer.line("    params: StructParamEntry[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.seqs.is_empty() {
        writer.line("export const SEQS: ReadonlyMap<number, SeqEntry> = new Map([")?;
        for entry in ctx.seqs.values() {
            let frames_json: String = entry
                .frames
                .iter()
                .map(|f| {
                    format!(
                        "{{ animId: {}, frameId: {}, delay: {} }}",
                        f.anim_id, f.frame_id, f.delay
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let params_json: String = entry
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{{ id: {}, value: {} }}",
                        p.param_id,
                        format_scalar_value(&p.value)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, frames: [{frames_json}], stretches: {stretches}, priority: {priority}, leftHand: {lefthand}, rightHand: {righthand}, loopCount: {loopcount}, params: [{params_json}] }}],",
                id = entry.id,
                stretches = entry.stretches,
                priority = entry.priority.map(|p| p.to_string()).unwrap_or_else(|| "null".to_string()),
                lefthand = entry.lefthand_raw.map(|l| l.to_string()).unwrap_or_else(|| "null".to_string()),
                righthand = entry.righthand_raw.map(|r| r.to_string()).unwrap_or_else(|| "null".to_string()),
                loopcount = entry.loopcount.map(|l| l.to_string()).unwrap_or_else(|| "null".to_string()),
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const SEQ_COUNT = {};", ctx.seqs.len()))?;
    writer.finish()
}

pub fn export_spot_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.spots.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Spotanim (Graphic) definitions".to_string(),
        "// Source: RS3 cache spot config".to_string(),
        String::new(),
        "export interface SpotEntry {".to_string(),
        "    id: number;".to_string(),
        "    ops: string[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const SPOTS: ReadonlyMap<number, SpotEntry> = new Map([".to_string());
        for entry in &entries {
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(&format!("{o:?}"))))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, ops: [{ops_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const SPOT_COUNT = {};", entries.len()));

    write_lines(&out_dir.join("spots.ts"), &lines)
}

/// Extract a name from op list entries like "name=Attack" or "name=Man".
fn extract_oplist_name(ops: &[String]) -> String {
    for op in ops {
        if let Some(name) = op.strip_prefix("name=") {
            return format!("'{}'", escape_ts_string(name));
        }
    }
    "null".to_string()
}

fn extract_oplist_name_raw(ops: &[String]) -> Option<String> {
    for op in ops {
        if let Some(name) = op.strip_prefix("name=") {
            return Some(name.to_string());
        }
    }
    None
}

pub fn export_named_config_ids(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    export_named_oplist_ids("Obj", "objs", &ctx.objs, out_dir)?;
    export_named_oplist_ids("Npc", "npcs", &ctx.npcs, out_dir)?;
    export_named_oplist_ids("Loc", "locs", &ctx.locs, out_dir)?;
    Ok(())
}

fn export_named_oplist_ids(
    prefix: &str,
    source_file: &str,
    entries: &BTreeMap<u32, crate::config::OpListEntry>,
    out_dir: &Path,
) -> Result<()> {
    let const_name = format!("Named{prefix}Ids");
    let mut lines = vec![
        format!("// Auto-generated named {prefix} ID constants"),
        format!("// Source: RS3 cache {source_file} config (named entries only)"),
        String::new(),
    ];

    let mut named: Vec<(String, u32)> = Vec::new();
    for entry in entries.values() {
        if let Some(raw_name) = extract_oplist_name_raw(&entry.ops) {
            let prop = str_to_screaming_snake(&raw_name);
            if !prop.is_empty() {
                named.push((prop, entry.id));
            }
        }
    }
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named.dedup_by(|a, b| a.0 == b.0);

    if named.is_empty() {
        lines.push(format!("export const {const_name} = {{}} as const;"));
    } else {
        lines.push(format!("export const {const_name} = {{"));
        for (prop, id) in &named {
            lines.push(format!("    {prop}: {id},"));
        }
        lines.push("} as const;".to_string());
    }
    lines.push(String::new());
    lines.push(format!(
        "export const NAMED_{}_COUNT = {};",
        prefix.to_uppercase(),
        named.len()
    ));

    write_lines(&out_dir.join(format!("named_{source_file}.ts")), &lines)
}

pub fn export_script_signatures(
    out_dir: &Path,
    script_catalog: &crate::transpile::ScriptCatalog,
) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated CS2 script signatures".to_string(),
        "// Source: RS3 cache clientscript archive".to_string(),
        String::new(),
    ];

    let mut entries: Vec<(String, String)> = Vec::new();
    for script in script_catalog.iter() {
        let function_name = script.export_name.clone();
        let mut arg_types: Vec<&str> = Vec::new();
        arg_types.extend(std::iter::repeat_n(
            "number",
            script.signature.arg_count_int as usize,
        ));
        arg_types.extend(std::iter::repeat_n(
            "string",
            script.signature.arg_count_obj as usize,
        ));
        arg_types.extend(std::iter::repeat_n(
            "bigint",
            script.signature.arg_count_long as usize,
        ));
        let args = (0..arg_types.len())
            .map(|i| format!("arg{i}: {}", arg_types[i]))
            .collect::<Vec<_>>()
            .join(", ");

        entries.push((
            function_name.clone(),
            format!(
                "export function {function_name}({args}): {};",
                script.signature.return_type
            ),
        ));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    lines.extend(entries.into_iter().map(|(_, line)| line));

    write_lines(&out_dir.join("scripts.d.ts"), &lines)
}

pub fn export_db_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("dbtables.ts"))?;
    writer.line("// Auto-generated Database definitions")?;
    writer.line("// Source: RS3 cache DB tables and rows (archive 2, groups 40/41)")?;
    writer.line("")?;
    writer.line("export interface DbTableColumn {")?;
    writer.line("    column: number;")?;
    writer.line("    tupleTypes: number[];")?;
    writer.line("    defaults: (number | string)[][];")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface DbTableEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    columns: DbTableColumn[];")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface DbRowColumn {")?;
    writer.line("    column: number;")?;
    writer.line("    tupleTypes: number[];")?;
    writer.line("    rows: (number | string)[][];")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface DbRowEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    table: number | null;")?;
    writer.line("    columns: DbRowColumn[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.dbtables.is_empty() {
        writer.line("export const DB_TABLES: ReadonlyMap<number, DbTableEntry> = new Map([")?;
        for entry in ctx.dbtables.values() {
            writer.line(format!("    [{id}, {{ id: {id}, columns: [", id = entry.id))?;
            for column in &entry.columns {
                let types = column
                    .tuple_types
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let defaults = column
                    .defaults
                    .iter()
                    .map(|row| {
                        let values = row
                            .iter()
                            .map(format_scalar_value)
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("[{values}]")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                writer.line(format!(
                    "        {{ column: {}, tupleTypes: [{}], defaults: [{}] }},",
                    column.column, types, defaults
                ))?;
            }
            writer.line("    ] }],")?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const DB_TABLE_COUNT = {};",
        ctx.dbtables.len()
    ))?;
    writer.line("")?;
    writer.line("// Reverse-engineered table column meanings:")?;
    writer.line("// Table 163 (5,237 rows, 32 cols) — Items")?;
    writer.line("//   col  0: itemId (int)")?;
    writer.line("//   col  1: parentId (int) — parent item or category")?;
    writer.line("//   col  2: name (string)")?;
    writer.line("//   col  3: description (string)")?;
    writer.line("//   col  4: paramId (int) — linked param config entry")?;
    writer.line("//   col  5: typeId (int) — item type/category")?;
    writer.line("//   col  6: value (int, default 99) — shop price")?;
    writer.line("//   col  7: flags (int, default 268435454)")?;
    writer.line("//   col  8: stackable (int, default 1)")?;
    writer.line("//   col 11: membersOnly (boolean, default false)")?;
    writer.line("//   col 13: categoryId (int)")?;
    writer.line("//   col 23: modelId (int)")?;
    writer.line("//   col 24: modelId2 (int)")?;
    writer.line("//   col 26: color (int) — RGBA tint")?;
    writer.line("//   col 30: equipmentOverrides (int[6]) — only for special items")?;
    writer.line("//         index 0-5: stab/slash/crush/magic/range/strength bonus")?;
    writer.line("//   col 31: soundId (int)")?;
    writer.line("//")?;
    writer.line("// Table 29 (105 rows, 46 cols) — NPC stats")?;
    writer.line("//   cols 1-3: model IDs")?;
    writer.line("//   col  5: name (string)")?;
    writer.line("//   col  7: size (int)")?;
    writer.line("//   col  9: combatLevel (int)")?;
    writer.line("//   col 10: hitpoints (int)")?;
    writer.line("//   col 14: attack (int)")?;
    writer.line("//   col 17: defence (int)")?;
    writer.line("//   col 18: accuracy (int)")?;
    writer.line("//")?;
    writer.line("// Note: Most equipment/weapon stats are computed client-side")?;
    writer.line("// from item tier + category, not stored per-item in this table.")?;
    writer.line("// Only override stats (halos, special items) use col 30.")?;
    writer.line("")?;

    if !ctx.dbrows.is_empty() {
        let mut by_table: BTreeMap<u32, Vec<&crate::config::DbRowEntry>> = BTreeMap::new();
        for row in ctx.dbrows.values() {
            if let Some(table) = row.table {
                by_table.entry(table).or_default().push(row);
            }
        }
        writer.line("// DB rows grouped by table ID. Key = tableId, value = rows.")?;
        writer.line("export const DB_ROWS: ReadonlyMap<number, DbRowEntry[]> = new Map([")?;
        for (table_id, rows) in &by_table {
            writer.line(format!("    [{table_id}, ["))?;
            for row in rows {
                writer.line(format!(
                    "        {{ id: {}, table: {}, columns: [",
                    row.id, table_id
                ))?;
                for column in &row.columns {
                    let types = column
                        .tuple_types
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let row_data = column
                        .rows
                        .iter()
                        .map(|tuple| {
                            let values = tuple
                                .iter()
                                .map(format_scalar_value)
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("[{values}]")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    writer.line(format!(
                        "            {{ column: {}, tupleTypes: [{}], rows: [{}] }},",
                        column.column, types, row_data
                    ))?;
                }
                writer.line("        ] },")?;
            }
            writer.line("    ]],")?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const DB_ROW_COUNT = {};", ctx.dbrows.len()))?;
    writer.line("")?;
    writer.line("export interface ItemEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    description: string | null;")?;
    writer.line("    /** Shop / GE price. */")?;
    writer.line("    value: number;")?;
    writer.line("    /** Non-zero means stackable. */")?;
    writer.line("    stackable: boolean;")?;
    writer.line("    membersOnly: boolean;")?;
    writer.line("    categoryId: number | null;")?;
    writer.line("    parentId: number | null;")?;
    writer.line("    modelId: number | null;")?;
    writer.line("    /** RGBA tint (e.g. 16832257). */")?;
    writer.line("    color: number | null;")?;
    writer.line("    paramId: number | null;")?;
    writer.line("    soundId: number | null;")?;
    writer.line("    /** Key→value pairs for linked param configs. */")?;
    writer.line("    params: Array<{ key: number; value: number | string }>;")?;
    writer.line("    /** Equipment stat overrides (only 2 items). */")?;
    writer.line("    equipmentOverrides: number[] | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let items: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(163))
        .collect();
    if !items.is_empty() {
        writer.line("export const ITEMS: ReadonlyMap<number, ItemEntry> = new Map([")?;
        for row in &items {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 2);
            let desc = row_column_str(row, 3);
            let value = row_column_int(row, 6).unwrap_or(99);
            let stackable = row_column_int(row, 8).unwrap_or(1) != 0;
            let members = row_column_bool(row, 11);
            let category = row_column_int_or_null(row, 13);
            let parent = row_column_int_or_null(row, 1);
            let model = row_column_int_or_null(row, 23);
            let color = row_column_int_or_null(row, 26);
            let param = row_column_int_or_null(row, 4);
            let sound = row_column_int_or_null(row, 31);
            let eq_overrides = row_column_int_array(row, 30);
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            let desc_str = desc
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            let eq_str = eq_overrides
                .map(|values| {
                    format!(
                        "[{}]",
                        values
                            .iter()
                            .map(i32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, description: {desc_str}, value: {value}, stackable: {stackable}, membersOnly: {members}, categoryId: {category}, parentId: {parent}, modelId: {model}, color: {color}, paramId: {param}, soundId: {sound}, params: [], equipmentOverrides: {eq_str} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const ITEM_COUNT = {};", items.len()))?;
    writer.line("")?;
    writer.line("export interface NpcStatEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    combatLevel: number;")?;
    writer.line("    hitpoints: number;")?;
    writer.line("    attack: number;")?;
    writer.line("    defence: number;")?;
    writer.line("    accuracy: number;")?;
    writer.line("    size: number;")?;
    writer.line("    respawnMs: number | null;")?;
    writer.line("    modelIds: number[];")?;
    writer.line("}")?;
    writer.line("")?;

    let npc_stats: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(29))
        .collect();
    if !npc_stats.is_empty() {
        writer.line("export const NPC_STATS: ReadonlyMap<number, NpcStatEntry> = new Map([")?;
        for row in &npc_stats {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 5);
            let combat = row_column_int(row, 9).unwrap_or(0);
            let hp = row_column_int(row, 10).unwrap_or(0);
            let atk = row_column_int(row, 14).unwrap_or(0);
            let def = row_column_int(row, 17).unwrap_or(0);
            let acc = row_column_int(row, 18).unwrap_or(0);
            let size = row_column_int(row, 7).unwrap_or(1);
            let respawn = row_column_int_or_null(row, 13);
            let models = [1, 2, 3]
                .iter()
                .filter_map(|&column| row_column_int(row, column))
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, combatLevel: {combat}, hitpoints: {hp}, attack: {atk}, defence: {def}, accuracy: {acc}, size: {size}, respawnMs: {respawn}, modelIds: [{models}] }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const NPC_STAT_COUNT = {};",
        npc_stats.len()
    ))?;
    writer.line("")?;
    writer.line("export interface ClueLocationEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    /** Difficulty tier (1-5). */")?;
    writer.line("    tier: number;")?;
    writer.line("    description: string | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let clue_rows: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(7))
        .collect();
    if !clue_rows.is_empty() {
        writer.line(
            "export const CLUE_LOCATIONS: ReadonlyMap<number, ClueLocationEntry> = new Map([",
        )?;
        for row in &clue_rows {
            let id = row_column_int(row, 0).unwrap_or(0);
            let tier = row_column_int(row, 1).unwrap_or(1);
            let desc = row_column_str(row, 2);
            let desc_str = desc
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, tier: {tier}, description: {desc_str} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const CLUE_LOCATION_COUNT = {};",
        clue_rows.len()
    ))?;
    writer.line("")?;
    writer.line("export interface ItemCategoryEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    modelId: number | null;")?;
    writer.line("    iconId: number | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let categories: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(4))
        .collect();
    if !categories.is_empty() {
        writer.line(
            "export const ITEM_CATEGORIES: ReadonlyMap<number, ItemCategoryEntry> = new Map([",
        )?;
        for row in &categories {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 1);
            let model = row_column_int_or_null(row, 4);
            let icon = row_column_int_or_null(row, 5);
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, modelId: {model}, iconId: {icon} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const ITEM_CATEGORY_COUNT = {};",
        categories.len()
    ))?;
    writer.line("")?;
    writer.line("export interface ItemSetEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    description: string | null;")?;
    writer.line("    representativeItemId: number | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let sets: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(5))
        .collect();
    if !sets.is_empty() {
        writer.line("export const ITEM_SETS: ReadonlyMap<number, ItemSetEntry> = new Map([")?;
        for row in &sets {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 1);
            let desc = row_column_str(row, 2);
            let rep_item = row_column_int_or_null(row, 5);
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            let desc_str = desc
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, description: {desc_str}, representativeItemId: {rep_item} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const ITEM_SET_COUNT = {};", sets.len()))?;
    writer.line("")?;
    writer.line("// Named column indices for table 163 (items).")?;
    writer.line("// Example: row.columns[ItemColumn.NAME]")?;
    writer.line("export const ItemColumn = {")?;
    writer.line("    ID: 0,")?;
    writer.line("    PARENT_ID: 1,")?;
    writer.line("    NAME: 2,")?;
    writer.line("    DESCRIPTION: 3,")?;
    writer.line("    PARAM_ID: 4,")?;
    writer.line("    TYPE_ID: 5,")?;
    writer.line("    VALUE: 6,")?;
    writer.line("    FLAGS: 7,")?;
    writer.line("    STACKABLE: 8,")?;
    writer.line("    MEMBERS_ONLY: 11,")?;
    writer.line("    CATEGORY_ID: 13,")?;
    writer.line("    MODEL_ID: 23,")?;
    writer.line("    MODEL_ID2: 24,")?;
    writer.line("    COLOR: 26,")?;
    writer.line("    EQUIPMENT_OVERRIDES: 30,")?;
    writer.line("    SOUND_ID: 31,")?;
    writer.line("} as const;")?;
    writer.line("export type ItemColumn = (typeof ItemColumn)[keyof typeof ItemColumn];")?;
    writer.line("")?;
    writer.line("// Named column indices for table 29 (NPC stats).")?;
    writer.line("export const NpcColumn = {")?;
    writer.line("    ID: 0,")?;
    writer.line("    MODEL_ID1: 1,")?;
    writer.line("    MODEL_ID2: 2,")?;
    writer.line("    MODEL_ID3: 3,")?;
    writer.line("    NAME: 5,")?;
    writer.line("    SIZE: 7,")?;
    writer.line("    COMBAT_LEVEL: 9,")?;
    writer.line("    HITPOINTS: 10,")?;
    writer.line("    RESPAWN_MS: 13,")?;
    writer.line("    ATTACK: 14,")?;
    writer.line("    DEFENCE: 17,")?;
    writer.line("    ACCURACY: 18,")?;
    writer.line("} as const;")?;
    writer.line("export type NpcColumn = (typeof NpcColumn)[keyof typeof NpcColumn];")?;
    writer.finish()
}

/// Extract the first int value from a specific column in a DB row.
fn row_column_int(row: &crate::config::DbRowEntry, col: u8) -> Option<i32> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| match v {
            crate::config::ScalarValue::Int(i) => Some(*i),
            _ => None,
        })
}

/// Extract the first string value from a specific column in a DB row.
fn row_column_str(row: &crate::config::DbRowEntry, col: u8) -> Option<String> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| match v {
            crate::config::ScalarValue::Str(s) => Some(escape_ts_string(s)),
            _ => None,
        })
}

/// Extract a boolean from a specific column (0=false, non-zero=true).
fn row_column_bool(row: &crate::config::DbRowEntry, col: u8) -> bool {
    row_column_int(row, col).unwrap_or(0) != 0
}

/// Extract an int as a TS null-or-number string.
fn row_column_int_or_null(row: &crate::config::DbRowEntry, col: u8) -> String {
    row_column_int(row, col)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string())
}

/// Extract all ints from a tuple column as a Vec (equipment overrides etc.).
fn row_column_int_array(row: &crate::config::DbRowEntry, col: u8) -> Option<Vec<i32>> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .map(|r| {
            r.iter()
                .filter_map(|v| match v {
                    crate::config::ScalarValue::Int(i) => Some(*i),
                    _ => None,
                })
                .collect()
        })
}

pub fn export_interface_ids(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    struct ComponentExportEntry<'a> {
        uid: u32,
        interface_id: u32,
        component_id: u32,
        deps: &'a crate::interface::ComponentDeps,
    }

    let mut all_components: Vec<ComponentExportEntry<'_>> = Vec::new();
    for (&interface_id, comps) in &ctx.parsed_components {
        for (&component_id, deps) in comps {
            all_components.push(ComponentExportEntry {
                uid: crate::interface::component_uid(interface_id, component_id),
                interface_id,
                component_id,
                deps,
            });
        }
    }
    all_components.sort_by_key(|entry| entry.uid);

    let mut writer = TextFileWriter::create(&out_dir.join("interfaces.ts"))?;
    writer.line("// Auto-generated Interface Component definitions")?;
    writer.line("// Source: RS3 cache interfaces archive (parsed component deps)")?;
    writer.line("")?;

    // ── Interface ID constants ──
    if !ctx.parsed_components.is_empty() {
        writer.line("// Root interface group IDs.")?;
        writer.line("export const InterfaceId = {")?;
        for &interface_id in ctx.parsed_components.keys() {
            writer.line(format!("    interface_{interface_id}: {interface_id},"))?;
        }
        writer.line("} as const;")?;
        writer.line("")?;
        writer.line("export type InterfaceId = (typeof InterfaceId)[keyof typeof InterfaceId];")?;
        writer.line("")?;
    }

    // ── Named component ID constants (full UID keys) ──
    let mut named_entries: Vec<(String, u32, u32, u32, &str)> = Vec::new();
    for entry in &all_components {
        let label = entry
            .deps
            .name
            .as_deref()
            .map(sanitize_ts_prop)
            .filter(|prop| !prop.is_empty() && prop != "unnamed")
            .unwrap_or_else(|| {
                sanitize_ts_prop(&crate::interface::component_fallback_name(
                    entry.interface_id,
                    entry.component_id,
                ))
            });
        named_entries.push((
            label,
            entry.uid,
            entry.interface_id,
            entry.component_id,
            &entry.deps.component_type,
        ));
    }
    named_entries.sort_by(|a, b| a.0.cmp(&b.0));
    named_entries.dedup_by(|a, b| a.0 == b.0);
    named_entries.sort_by_key(|e| e.1);

    if !named_entries.is_empty() {
        writer.line("// Named component UIDs used by CS2 cc_* / if_* opcodes.")?;
        writer.line("export const ComponentId = {")?;
        for (prop, uid, interface_id, component_id, comp_type) in &named_entries {
            writer.line(format!(
                "    /** {comp_type} interface={interface_id} com={component_id} uid={uid} */"
            ))?;
            writer.line(format!("    {prop}: {uid},"))?;
        }
        writer.line("} as const;")?;
        writer.line("")?;
        writer.line("export type ComponentId = (typeof ComponentId)[keyof typeof ComponentId];")?;
        writer.line("")?;
    }

    // ── ComponentInfo interface and data ──
    writer.line("export interface ComponentInfo {")?;
    writer.line("    id: number;")?;
    writer.line("    interfaceId: number;")?;
    writer.line("    componentId: number;")?;
    writer.line("    type: string;")?;
    writer.line("    name: string | null;")?;
    writer.line("    children: number[];")?;
    writer.line("    scripts: number[];")?;
    writer.line("    varps: Array<{domain: string; id: number}>;")?;
    writer.line("    varbits: number[];")?;
    writer.line("    enums: number[];")?;
    writer.line("    params: number[];")?;
    writer.line("    invs: number[];")?;
    writer.line("    models: number[];")?;
    writer.line("    seqs: number[];")?;
    writer.line("}")?;
    writer.line("")?;

    writer.line("export const ALL_COMPONENTS: ReadonlyMap<number, ComponentInfo> = new Map([")?;
    for entry in &all_components {
        let deps = entry.deps;
        let varp_items: Vec<String> = deps
            .varps
            .iter()
            .map(|v| {
                let (domain, id) = match v {
                    crate::interface::VarTransmitRef::Player(id) => ("player", *id),
                    crate::interface::VarTransmitRef::Npc(id) => ("npc", *id),
                    crate::interface::VarTransmitRef::Client(id) => ("client", *id),
                    crate::interface::VarTransmitRef::World(id) => ("world", *id),
                    crate::interface::VarTransmitRef::Region(id) => ("region", *id),
                    crate::interface::VarTransmitRef::Object(id) => ("object", *id),
                    crate::interface::VarTransmitRef::Clan(id) => ("clan", *id),
                    crate::interface::VarTransmitRef::ClanSetting(id) => ("clan_setting", *id),
                    crate::interface::VarTransmitRef::Controller(id) => ("controller", *id),
                    crate::interface::VarTransmitRef::Global(id) => ("global", *id),
                    crate::interface::VarTransmitRef::PlayerGroup(id) => ("player_group", *id),
                    crate::interface::VarTransmitRef::VarClientString(id) => ("client", *id),
                };
                format!("{{domain:'{domain}',id:{id}}}")
            })
            .collect();
        let scripts_json = set_to_json(&deps.scripts);
        let varbits_json = set_to_json(&deps.varbits);
        let enums_json = set_to_json(&deps.enums);
        let params_json = set_to_json(&deps.params);
        let invs_json = set_to_json(&deps.invs);
        let models_json = set_to_json(&deps.models);
        let seqs_json = set_to_json(&deps.seqs);
        let children_json: String = deps
            .children
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let name_str = match &deps.name {
            Some(n) => format!("'{}'", escape_ts_string(n)),
            None => "null".to_string(),
        };
        writer.line(format!(
            "    [{uid}, {{ id:{uid}, interfaceId:{interface_id}, componentId:{component_id}, type:'{type}', name:{name}, children:[{children}], scripts:[{scripts}], varps:[{varps}], varbits:[{varbits}], enums:[{enums}], params:[{params}], invs:[{invs}], models:[{models}], seqs:[{seqs}] }}],",
            uid = entry.uid,
            interface_id = entry.interface_id,
            component_id = entry.component_id,
            type = deps.component_type,
            name = name_str,
            children = children_json,
            scripts = scripts_json,
            varps = varp_items.join(", "),
            varbits = varbits_json,
            enums = enums_json,
            params = params_json,
            invs = invs_json,
            models = models_json,
            seqs = seqs_json,
        ))?;
    }
    writer.line("]);")?;
    writer.line("")?;
    writer.line(format!(
        "export const COMPONENT_COUNT = {};",
        all_components.len()
    ))?;
    writer.finish()
}

fn set_to_json(set: &std::collections::HashSet<u32>) -> String {
    let mut items: Vec<u32> = set.iter().copied().collect();
    items.sort_unstable();
    items
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn export_index(out_dir: &Path) -> Result<()> {
    write_text(&out_dir.join("index.ts"), &index_exports_source())
}

fn index_exports_source() -> String {
    let lines = vec![
        "// Auto-generated index file".to_string(),
        "// Source: RS3 cache ts-export".to_string(),
        String::new(),
        "export {".to_string(),
        "    VARS,".to_string(),
        "    VAR_COUNT,".to_string(),
        "    type VarEntry,".to_string(),
        "    type VarDomain,".to_string(),
        "    type VarType,".to_string(),
        "    type VarLifetime,".to_string(),
        "    type VarTransmitLevel,".to_string(),
        "} from './vars';".to_string(),
        String::new(),
        "export {".to_string(),
        "    VARBITS,".to_string(),
        "    VARBIT_COUNT,".to_string(),
        "    type VarBitEntry,".to_string(),
        "} from './varbits';".to_string(),
        String::new(),
        "export {".to_string(),
        "    ENUMS,".to_string(),
        "    ENUM_COUNT,".to_string(),
        "    ENUM_VALUE_TO_NAME,".to_string(),
        "    type EnumEntry,".to_string(),
        "    type EnumPair,".to_string(),
        "} from './enums';".to_string(),
        String::new(),
        "export {".to_string(),
        "    STRUCTS,".to_string(),
        "    STRUCT_COUNT,".to_string(),
        "    type StructEntry,".to_string(),
        "    type StructParamEntry,".to_string(),
        "} from './structs';".to_string(),
        String::new(),
        "export {".to_string(),
        "    PARAMS,".to_string(),
        "    PARAM_COUNT,".to_string(),
        "    type ParamEntry,".to_string(),
        "    type ParamValue,".to_string(),
        "} from './params';".to_string(),
        String::new(),
        "export {".to_string(),
        "    InterfaceId,".to_string(),
        "    ComponentId,".to_string(),
        "    ALL_COMPONENTS,".to_string(),
        "    COMPONENT_COUNT,".to_string(),
        "    type ComponentInfo,".to_string(),
        "    type InterfaceId as InterfaceIdType,".to_string(),
        "} from './interfaces';".to_string(),
        "export { type InvEntry, INVS, INV_COUNT } from './invs';".to_string(),
        "export { type ObjEntry, OBJS, OBJ_COUNT } from './objs';".to_string(),
        "export { type NpcEntry, NPCS, NPC_COUNT } from './npcs';".to_string(),
        "export { type LocEntry, LOCS, LOC_COUNT } from './locs';".to_string(),
        "export { type SeqEntry, SEQS, SEQ_COUNT } from './seqs';".to_string(),
        "export { type SpotEntry, SPOTS, SPOT_COUNT } from './spots';".to_string(),
        "export { NamedObjIds, NAMED_OBJ_COUNT } from './named_objs';".to_string(),
        "export { NamedNpcIds, NAMED_NPC_COUNT } from './named_npcs';".to_string(),
        "export { NamedLocIds, NAMED_LOC_COUNT } from './named_locs';".to_string(),
        "export { type ItemEntry, ITEMS, ITEM_COUNT,".to_string(),
        "    type ItemCategoryEntry, ITEM_CATEGORIES, ITEM_CATEGORY_COUNT,".to_string(),
        "    type ItemSetEntry, ITEM_SETS, ITEM_SET_COUNT,".to_string(),
        "    type NpcStatEntry, NPC_STATS, NPC_STAT_COUNT,".to_string(),
        "    type ClueLocationEntry, CLUE_LOCATIONS, CLUE_LOCATION_COUNT,".to_string(),
        "    ItemColumn, type ItemColumn, NpcColumn, type NpcColumn,".to_string(),
        "} from './dbtables';".to_string(),
        "export { DB_TABLES, DB_TABLE_COUNT, DB_ROWS, DB_ROW_COUNT,".to_string(),
        "    type DbTableEntry, type DbRowEntry, type DbTableColumn, type DbRowColumn } from './dbtables';".to_string(),
    ];

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_exports_source_has_balanced_unescaped_braces() {
        let source = index_exports_source();

        assert!(!source.contains("{{"));
        assert!(!source.contains("}}"));
        assert_export_braces_are_balanced(&source);
    }

    fn assert_export_braces_are_balanced(source: &str) {
        let mut depth = 0_i32;
        for line in source.lines() {
            for c in line.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                assert!(depth >= 0, "extra closing brace in {line}");
            }
        }
        assert_eq!(0, depth, "unclosed export brace");
    }
}
