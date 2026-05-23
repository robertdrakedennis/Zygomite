pub mod ast;
pub mod cfg;
pub mod codegen;
pub mod diagnostics;
pub mod expr_recovery;
pub mod scope;
pub mod sema;
pub mod structured_writer;
pub mod writer;

pub use ast::*;
pub use cfg::{
    Block, StructuredStmt, SwitchCaseStmt, build_cfg, detect_return_type, emit_structured,
};
pub use codegen::{CodeGen, generate_program};
pub use diagnostics::{Diagnostic, Diagnostics, Severity, Span};
pub use scope::{LocalType, Scope, Scopes, Symbol, SymbolKind, SymbolTable};
pub use sema::Sema;
pub use structured_writer::StructuredWriter;
pub use writer::Writer;

use crate::config::EnumEntry;
use crate::script::{CompiledScript, OpcodeBook, decode_script};
use crate::vars::VarDomain;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};

/// Describes a script's parameter and return types for cross-script call typing.
#[derive(Debug, Clone)]
pub struct ScriptSignature {
    pub arg_count_int: u16,
    pub arg_count_obj: u16,
    pub arg_count_long: u16,
    pub return_type: String,
}

impl ScriptSignature {
    pub fn total_args(&self) -> usize {
        self.arg_count_int as usize + self.arg_count_obj as usize + self.arg_count_long as usize
    }
}

pub struct Transpiler {
    symbol_table: SymbolTable,
    script_signatures: HashMap<ScriptId, ScriptSignature>,
}

impl Transpiler {
    pub fn new() -> Self {
        Self {
            symbol_table: SymbolTable::new(),
            script_signatures: HashMap::new(),
        }
    }

    pub fn with_enums(mut self, enums: &BTreeMap<u32, EnumEntry>) -> Self {
        for (id, entry) in enums {
            self.symbol_table
                .enum_map
                .insert(*id, format!("enum_{}", entry.id));
        }
        self
    }

    pub fn with_vars(
        mut self,
        varps: &HashMap<VarDomain, BTreeMap<u32, crate::vars::VarEntry>>,
    ) -> Self {
        for (domain, vars) in varps {
            for (id, var) in vars {
                self.symbol_table
                    .var_map
                    .insert((*domain, *id as u16), var.var_name.clone());
            }
        }
        self
    }

    pub fn with_varbits(mut self, varbits: &BTreeMap<u32, crate::vars::VarBitEntry>) -> Self {
        for (id, varbit) in varbits {
            self.symbol_table
                .varbit_map
                .insert(*id as u16, varbit.varbit_name.clone());
        }
        self
    }

    pub fn with_params(mut self, params: &BTreeMap<u32, crate::config::ParamEntry>) -> Self {
        for (id, param) in params {
            self.symbol_table
                .param_map
                .insert(*id, format!("param_{}", param.id));
        }
        self
    }

    pub fn with_script_names(
        mut self,
        scripts: &BTreeMap<u32, Vec<u8>>,
        opcode_book: &OpcodeBook,
        version: u32,
    ) -> Self {
        let mut names = HashMap::new();
        for (&script_id, data) in scripts {
            if let Ok(script) = decode_script(data, opcode_book, version)
                && let Some(name) = &script.name
            {
                names.insert(
                    ScriptId(script_id as i32),
                    extract_script_name_suffix(name),
                );
            }
        }
        self.symbol_table.script_names = names;
        self
    }

    /// Fill script names from archive group name hashes (`names/scripts.txt`).
    pub fn with_script_group_names(
        mut self,
        scripts: &BTreeMap<u32, Vec<u8>>,
        group_names: &HashMap<u32, String>,
    ) -> Self {
        for &script_id_raw in scripts.keys() {
            let group = script_id_raw >> 16;
            if let Some(name) = group_names.get(&group) {
                self.symbol_table
                    .script_names
                    .entry(ScriptId(script_id_raw as i32))
                    .or_insert_with(|| name.clone());
            }
        }
        self
    }

    pub fn with_components(
        mut self,
        parsed_components: &BTreeMap<u32, BTreeMap<u32, crate::interface::ComponentDeps>>,
    ) -> Self {
        let mut names = HashMap::new();
        for (&interface_id, comps) in parsed_components {
            for (&comp_id, deps) in comps {
                let uid = crate::interface::component_uid(interface_id, comp_id);
                let name = deps
                    .name
                    .clone()
                    .unwrap_or_else(|| crate::interface::component_fallback_name(interface_id, comp_id));
                names.insert(uid, name);
            }
        }
        self.symbol_table.component_names = names;
        self
    }

    pub fn with_enums_map(mut self, enums: &BTreeMap<u32, crate::config::EnumEntry>) -> Self {
        let mut names = HashMap::new();
        for entry in enums.values() {
            let obj = format!("Enum_{id}", id = entry.id);
            for pair in &entry.values {
                let prop = match &pair.value {
                    crate::config::ScalarValue::Str(s) => {
                        let name = str_to_screaming_snake(s);
                        if name.is_empty() {
                            format!("KEY_{key}", key = pair.key)
                        } else {
                            name
                        }
                    }
                    _ => format!("KEY_{key}", key = pair.key),
                };
                names.insert(pair.key, format!("{obj}.{prop}"));
            }
        }
        self.symbol_table.enum_value_names = names;
        self
    }

    /// Preload all script argument counts for cross-script call typing.
    /// Decodes every script to extract parameter counts so that
    /// `gosub_with_params` can emit typed calls.
    pub fn with_script_signatures(
        mut self,
        scripts: &BTreeMap<u32, Vec<u8>>,
        opcode_book: &OpcodeBook,
        version: u32,
    ) -> Self {
        let empty_components = HashMap::new();
        let empty_enums = HashMap::new();
        let empty_sigs = HashMap::new();
        for (&id, data) in scripts {
            if let Ok(script) = decode_script(data, opcode_book, version) {
                let script_id = ScriptId(id as i32);
                let return_type =
                    infer_return_type_for_script(&script, script_id, &empty_components, &empty_enums, &empty_sigs);
                self.script_signatures.insert(
                    script_id,
                    ScriptSignature {
                        arg_count_int: script.argument_count_int,
                        arg_count_obj: script.argument_count_object,
                        arg_count_long: script.argument_count_long,
                        return_type,
                    },
                );
            }
        }
        self
    }

    /// Get a script's signature for cross-script call typing.
    pub fn script_signature(&self, id: ScriptId) -> Option<&ScriptSignature> {
        self.script_signatures.get(&id)
    }

    pub fn script_name_for(&self, script_id: ScriptId) -> Option<String> {
        self.symbol_table.script_names.get(&script_id).cloned()
    }

    pub fn transpile_from_bytes(
        &self,
        data: &[u8],
        opcode_book: &OpcodeBook,
        version: u32,
        script_id: ScriptId,
    ) -> Result<TranspiledScript> {
        let script = decode_script(data, opcode_book, version)?;
        Ok(self.transpile(&script, script_id))
    }

    pub fn transpile(&self, script: &CompiledScript, script_id: ScriptId) -> TranspiledScript {
        self.transpile_structured(script, script_id)
    }

    pub fn transpile_to_ast(&self, script: &CompiledScript, script_id: ScriptId) -> Declaration {
        let codegen = CodeGen::new(self.symbol_table.clone());
        codegen.generate(script, script_id)
    }

    pub fn transpile_structured(
        &self,
        script: &CompiledScript,
        script_id: ScriptId,
    ) -> TranspiledScript {
        let codegen = CodeGen::new(self.symbol_table.clone());
        let decl = codegen.generate(script, script_id);
        let mut writer = StructuredWriter::new(
            self.symbol_table.component_names.clone(),
            self.symbol_table.enum_value_names.clone(),
            self.script_signatures.clone(),
            self.symbol_table.script_names.clone(),
        );
        let source = writer.write_declaration(&decl);
        TranspiledScript {
            source,
            referenced_vars: collect_var_refs(script),
            referenced_varbits: collect_varbit_refs(script),
            referenced_enums: collect_enum_refs(script),
            referenced_scripts: collect_script_refs(script),
        }
    }
}

impl Default for Transpiler {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TranspiledScript {
    pub source: String,
    pub referenced_vars: Vec<(VarDomain, u16)>,
    pub referenced_varbits: Vec<u16>,
    pub referenced_enums: Vec<u32>,
    pub referenced_scripts: Vec<ScriptId>,
}

fn collect_var_refs(script: &CompiledScript) -> Vec<(VarDomain, u16)> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let crate::script::Operand::VarRef(v) = &instruction.operand {
            refs.push((v.domain, v.id));
        }
    }
    refs
}

fn collect_varbit_refs(script: &CompiledScript) -> Vec<u16> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let crate::script::Operand::VarBitRef(v) = &instruction.operand {
            refs.push(v.id);
        }
    }
    refs
}

fn collect_enum_refs(script: &CompiledScript) -> Vec<u32> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let crate::script::Operand::Int(v) = &instruction.operand
            && *v > 0
        {
            refs.push(*v as u32);
        }
    }
    refs
}

fn collect_script_refs(script: &CompiledScript) -> Vec<ScriptId> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let crate::script::Operand::Script(id) = &instruction.operand {
            refs.push(ScriptId(*id));
        }
    }
    refs
}

pub fn sanitize_export_name(value: &str) -> String {
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

pub fn script_function_name(script_id: ScriptId, script_name: Option<&str>) -> String {
    script_name
        .map(extract_script_name_suffix)
        .map(|name| sanitize_export_name(&name))
        .filter(|name| name != "script")
        .unwrap_or_else(|| format!("script_{script_id}"))
}

/// Strip `[clientscript,name]` / `[proc,name]` tag syntax to the suffix identifier.
pub fn extract_script_name_suffix(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if let Some((_, suffix)) = inner.split_once(',') {
            return suffix.to_string();
        }
    }
    trimmed.to_string()
}

pub fn infer_return_type_for_script(
    script: &CompiledScript,
    script_id: ScriptId,
    component_names: &HashMap<u32, String>,
    enum_value_names: &HashMap<i32, String>,
    script_signatures: &HashMap<ScriptId, ScriptSignature>,
) -> String {
    let codegen = CodeGen::new(SymbolTable::new());
    let decl = codegen.generate(script, script_id);
    let empty_names: HashMap<ScriptId, String> = HashMap::new();
    let blocks = build_cfg(
        decl.instructions,
        component_names,
        enum_value_names,
        script_signatures,
        &empty_names,
    );
    let structured = emit_structured(blocks);
    detect_return_type(&structured).to_string()
}

fn str_to_screaming_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if c == ' ' || c == '-' || c == '/' || c == '.' {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{trimmed}")
    } else if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub fn sanitize_ts_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if i == 0 && c.is_ascii_digit() {
            out.push('_');
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unnamed".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_script_name_suffix, sanitize_ts_ident};

    #[test]
    fn extract_script_name_suffix_parses_tag_syntax() {
        assert_eq!(
            "bank_build_init",
            extract_script_name_suffix("[clientscript,bank_build_init]")
        );
        assert_eq!("plain_name", extract_script_name_suffix("plain_name"));
    }

    #[test]
    fn sanitize_ident() {
        assert_eq!("hello_world", sanitize_ts_ident("hello/world"));
        assert_eq!("_123abc", sanitize_ts_ident("123abc"));
        assert_eq!("unnamed", sanitize_ts_ident(""));
        assert_eq!("foo_bar", sanitize_ts_ident("foo bar"));
    }
}
