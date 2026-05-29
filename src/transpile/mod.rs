pub mod ast;
pub mod cfg;
pub mod codegen;
pub mod diagnostics;
pub mod expr_recovery;
pub mod reversible_format;
pub mod scope;
pub mod structured;
pub mod structured_writer;
pub mod ts_lower;
pub mod ts_parse;
pub mod writer;

pub use ast::*;
pub use cfg::{Block, build_cfg, detect_return_type, emit_structured};
pub use codegen::{CodeGen, generate_program};
pub use diagnostics::{Diagnostic, Diagnostics, Severity, Span};
pub use expr_recovery::detect_return_type_from_recovered;
pub use reversible_format::{
    ParsedReversibleSource, REVERSIBLE_FORMAT_VERSION, ReversibleMetadata,
    append_reversible_footer, blocking_diagnostics, editable_structured, is_reversible_source,
    parse_reversible_source, render_reversible_source, structured_digest,
};
pub use scope::{LocalType, Scope, Scopes, Symbol, SymbolKind, SymbolTable};
pub use structured::{AssignmentTarget, StructuredScript, StructuredStmt, SwitchCaseStmt};
pub use structured_writer::StructuredWriter;
pub use ts_lower::{ReverseCompileContext, lower_structured_script};
pub use ts_parse::parse_structured_typescript;
pub use writer::Writer;

use crate::cache_bail as bail;
use crate::config::EnumEntry;
use crate::error::Result;
use crate::script::{CompiledScript, MIN_SCRIPT_BUILD, OpcodeBook, decode_script, script_to_asm};
use crate::vars::VarDomain;
use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

pub const DEFAULT_MAX_TRANSPILE_SCRIPT_BYTES: usize = 16 << 20;
pub const DEFAULT_MAX_TRANSPILE_INSTRUCTIONS: usize = 25_000;
pub const DEFAULT_MAX_TRANSPILE_GENERATED_BYTES: usize = 4 << 20;

#[derive(Debug, Clone, Copy)]
pub struct TranspileLimits {
    pub max_script_bytes: usize,
    pub max_instructions: usize,
    pub max_generated_bytes: usize,
}

impl Default for TranspileLimits {
    fn default() -> Self {
        Self {
            max_script_bytes: DEFAULT_MAX_TRANSPILE_SCRIPT_BYTES,
            max_instructions: DEFAULT_MAX_TRANSPILE_INSTRUCTIONS,
            max_generated_bytes: DEFAULT_MAX_TRANSPILE_GENERATED_BYTES,
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptGroupId(pub i32);

impl std::fmt::Display for ScriptGroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptKind {
    ClientScript,
    Proc,
    Unknown,
}

impl ScriptKind {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::ClientScript => "clientscript",
            Self::Proc => "proc",
            Self::Unknown => "unknown",
        }
    }

    fn disambiguation_suffix(self) -> Option<&'static str> {
        match self {
            Self::ClientScript => Some("clientscript"),
            Self::Proc => Some("proc"),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScriptMetadata {
    pub packed_id: ScriptId,
    pub group_id: ScriptGroupId,
    pub file_id: u16,
    pub kind: ScriptKind,
    pub raw_name: Option<String>,
    pub short_name: String,
    pub export_name: String,
    pub module_name: String,
    pub signature: ScriptSignature,
}

#[derive(Debug, Clone, Default)]
pub struct ScriptCatalog {
    entries: HashMap<ScriptId, ScriptMetadata>,
    by_group: HashMap<ScriptGroupId, Vec<ScriptId>>,
    diagnostics: Vec<Diagnostic>,
}

impl ScriptCatalog {
    pub fn insert(&mut self, metadata: ScriptMetadata) {
        self.by_group
            .entry(metadata.group_id)
            .or_default()
            .push(metadata.packed_id);
        self.entries.insert(metadata.packed_id, metadata);
    }

    pub fn get(&self, script_id: ScriptId) -> Option<&ScriptMetadata> {
        self.entries.get(&script_id)
    }

    pub fn resolve_group(&self, group_id: ScriptGroupId) -> Option<&ScriptMetadata> {
        let packed_id = self
            .by_group
            .get(&group_id)?
            .iter()
            .copied()
            .min_by_key(|script_id| {
                self.entries
                    .get(script_id)
                    .map(|metadata| metadata.file_id)
                    .unwrap_or(u16::MAX)
            })?;
        self.entries.get(&packed_id)
    }

    pub fn resolve_call_target(&self, raw_id: i32) -> Option<&ScriptMetadata> {
        self.resolve_group(ScriptGroupId(raw_id))
            .or_else(|| self.entries.get(&ScriptId(raw_id)))
    }

    pub fn resolve_export_name(&self, export_name: &str) -> Option<&ScriptMetadata> {
        self.entries
            .values()
            .find(|metadata| metadata.export_name == export_name)
    }

    pub fn export_name(&self, script_id: ScriptId) -> Option<&str> {
        self.get(script_id)
            .map(|metadata| metadata.export_name.as_str())
    }

    pub fn module_name(&self, script_id: ScriptId) -> Option<&str> {
        self.get(script_id)
            .map(|metadata| metadata.module_name.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = &ScriptMetadata> {
        self.entries.values()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn export_name_map(&self) -> HashMap<ScriptId, String> {
        self.entries
            .iter()
            .map(|(&script_id, metadata)| (script_id, metadata.export_name.clone()))
            .collect()
    }

    pub fn signature_map(&self) -> HashMap<ScriptId, ScriptSignature> {
        self.entries
            .iter()
            .map(|(&script_id, metadata)| (script_id, metadata.signature.clone()))
            .collect()
    }
}

#[expect(
    clippy::implicit_hasher,
    reason = "transpile APIs use default HashMap aliases across module boundaries"
)]
pub fn resolve_script_signature<'a>(
    script_catalog: &'a ScriptCatalog,
    script_signatures: &'a HashMap<ScriptId, ScriptSignature>,
    script_id: ScriptId,
) -> Option<&'a ScriptSignature> {
    script_signatures.get(&script_id).or_else(|| {
        script_catalog
            .get(script_id)
            .map(|metadata| &metadata.signature)
    })
}

#[expect(
    clippy::implicit_hasher,
    reason = "transpile APIs use default HashMap aliases across module boundaries"
)]
pub fn resolve_call_target_signature<'a>(
    script_catalog: &'a ScriptCatalog,
    script_signatures: &'a HashMap<ScriptId, ScriptSignature>,
    raw_id: i32,
) -> Option<(&'a ScriptMetadata, &'a ScriptSignature)> {
    let metadata = script_catalog.resolve_call_target(raw_id)?;
    let signature =
        resolve_script_signature(script_catalog, script_signatures, metadata.packed_id)?;
    Some((metadata, signature))
}

#[derive(Debug, Clone)]
struct ParsedScriptName {
    kind: ScriptKind,
    short_name: String,
}

#[derive(Debug, Clone)]
struct PendingScriptMetadata {
    packed_id: ScriptId,
    group_id: ScriptGroupId,
    file_id: u16,
    kind: ScriptKind,
    raw_name: Option<String>,
    short_name: String,
    signature: ScriptSignature,
    export_name: String,
}

pub struct ScriptCatalogBuilder<'a, S> {
    group_names: &'a HashMap<u32, String, S>,
    opcode_book: &'a OpcodeBook,
    version: u32,
    infer_return_types: bool,
    pending: Vec<PendingScriptMetadata>,
}

impl<'a, S> ScriptCatalogBuilder<'a, S>
where
    S: BuildHasher,
{
    pub fn new(
        group_names: &'a HashMap<u32, String, S>,
        opcode_book: &'a OpcodeBook,
        version: u32,
    ) -> Self {
        Self {
            group_names,
            opcode_book,
            version,
            infer_return_types: true,
            pending: Vec::new(),
        }
    }

    pub fn without_return_types(mut self) -> Self {
        self.infer_return_types = false;
        self
    }

    pub fn add_script(&mut self, packed_id_raw: u32, data: &[u8]) {
        push_pending_script_metadata(
            &mut self.pending,
            packed_id_raw,
            data,
            self.group_names,
            self.opcode_book,
            self.version,
            self.infer_return_types,
        );
    }

    pub fn build(mut self) -> ScriptCatalog {
        let diagnostics = assign_script_export_names(&mut self.pending);

        let mut catalog = ScriptCatalog {
            diagnostics,
            ..ScriptCatalog::default()
        };
        for entry in self.pending {
            catalog.insert(ScriptMetadata {
                packed_id: entry.packed_id,
                group_id: entry.group_id,
                file_id: entry.file_id,
                kind: entry.kind,
                raw_name: entry.raw_name,
                short_name: entry.short_name,
                module_name: entry.export_name.clone(),
                export_name: entry.export_name,
                signature: entry.signature,
            });
        }
        catalog
    }
}

pub fn build_script_catalog<S: BuildHasher>(
    scripts: &BTreeMap<u32, Vec<u8>>,
    group_names: &HashMap<u32, String, S>,
    opcode_book: &OpcodeBook,
    version: u32,
) -> ScriptCatalog {
    let mut builder = ScriptCatalogBuilder::new(group_names, opcode_book, version);
    for (&packed_id_raw, data) in scripts {
        builder.add_script(packed_id_raw, data);
    }
    builder.build()
}

fn push_pending_script_metadata<S: BuildHasher>(
    pending: &mut Vec<PendingScriptMetadata>,
    packed_id_raw: u32,
    data: &[u8],
    group_names: &HashMap<u32, String, S>,
    opcode_book: &OpcodeBook,
    version: u32,
    infer_return_types: bool,
) {
    let Ok(script) = decode_script(data, opcode_book, version) else {
        return;
    };

    let packed_id = ScriptId(packed_id_raw as i32);
    let group_id = ScriptGroupId((packed_id_raw >> 16) as i32);
    let file_id = (packed_id_raw & 0xffff) as u16;
    let parsed_name = script.name.as_deref().and_then(parse_script_name_tag);
    let short_name = parsed_name
        .as_ref()
        .map(|parsed| parsed.short_name.clone())
        .or_else(|| group_names.get(&(packed_id_raw >> 16)).cloned())
        .unwrap_or_else(|| format!("script{}", group_id.0));
    let kind = parsed_name
        .as_ref()
        .map(|parsed| parsed.kind)
        .unwrap_or(ScriptKind::Unknown);
    let signature = ScriptSignature {
        arg_count_int: script.argument_count_int,
        arg_count_obj: script.argument_count_object,
        arg_count_long: script.argument_count_long,
        return_type: if infer_return_types {
            let empty_components: HashMap<u32, String> = HashMap::new();
            let empty_enums: HashMap<i32, String> = HashMap::new();
            let empty_catalog = ScriptCatalog::default();
            infer_return_type_for_script(
                &script,
                packed_id,
                version,
                &empty_components,
                &empty_enums,
                &empty_catalog,
                &HashMap::new(),
            )
        } else {
            "unknown".to_string()
        },
    };

    pending.push(PendingScriptMetadata {
        packed_id,
        group_id,
        file_id,
        kind,
        raw_name: script.name,
        short_name,
        signature,
        export_name: String::new(),
    });
}

fn assign_script_export_names(entries: &mut [PendingScriptMetadata]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut by_base: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        by_base
            .entry(base_script_export_name(entry))
            .or_default()
            .push(index);
    }

    for (base_name, indices) in by_base {
        if indices.len() == 1 {
            entries[indices[0]].export_name = base_name;
            continue;
        }

        let packed_ids = indices
            .iter()
            .map(|&index| entries[index].packed_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        diagnostics.push(Diagnostic::warning(format!(
            "ambiguous script export name '{base_name}' for packed ids [{packed_ids}]; applying disambiguation"
        )));

        for index in indices {
            let entry = &entries[index];
            let mut export_name = entry
                .kind
                .disambiguation_suffix()
                .map(|suffix| format!("{base_name}_{suffix}"))
                .unwrap_or_else(|| format!("{base_name}_{}", entry.group_id.0));
            if export_name == base_name {
                export_name = format!("{base_name}_{}", entry.group_id.0);
            }
            entries[index].export_name = export_name;
        }
    }

    let mut used: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        used.entry(entry.export_name.clone())
            .or_default()
            .push(index);
    }

    for indices in used.into_values().filter(|indices| indices.len() > 1) {
        for index in indices {
            let packed_id = entries[index].packed_id;
            diagnostics.push(Diagnostic::warning(format!(
                "ambiguous disambiguated export '{}' still collided; appending packed id {}",
                entries[index].export_name, packed_id
            )));
            entries[index].export_name = format!("{}_{packed_id}", entries[index].export_name);
        }
    }

    diagnostics
}

fn base_script_export_name(entry: &PendingScriptMetadata) -> String {
    let base_name = sanitize_export_name(&entry.short_name);
    if base_name.is_empty() || base_name == "script" {
        format!("script{}", entry.group_id.0)
    } else {
        base_name
    }
}

fn parse_script_name_tag(value: &str) -> Option<ParsedScriptName> {
    let trimmed = value.trim();
    let close = trimmed.find(']')?;
    if !trimmed.starts_with('[') {
        return None;
    }

    let inner = &trimmed[1..close];
    let (kind, suffix) = inner.split_once(',')?;
    let kind = match kind {
        "clientscript" => ScriptKind::ClientScript,
        "proc" => ScriptKind::Proc,
        _ => ScriptKind::Unknown,
    };

    Some(ParsedScriptName {
        kind,
        short_name: suffix.trim().to_string(),
    })
}

pub struct Transpiler {
    symbol_table: SymbolTable,
    script_signatures: HashMap<ScriptId, ScriptSignature>,
    script_catalog: ScriptCatalog,
    limits: TranspileLimits,
    build: u32,
    subbuild: u32,
}

impl Transpiler {
    pub fn new() -> Self {
        Self {
            symbol_table: SymbolTable::new(),
            script_signatures: HashMap::new(),
            script_catalog: ScriptCatalog::default(),
            limits: TranspileLimits::default(),
            build: 0,
            subbuild: 0,
        }
    }

    pub fn with_version(mut self, build: u32, subbuild: u32) -> Self {
        self.build = build;
        self.subbuild = subbuild;
        self
    }

    pub fn with_script_catalog(mut self, script_catalog: ScriptCatalog) -> Self {
        self.symbol_table.script_names.clear();
        self.script_signatures.clear();
        self.script_catalog = script_catalog;
        self
    }

    pub fn with_limits(mut self, limits: TranspileLimits) -> Self {
        self.limits = limits;
        self
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
                names.insert(ScriptId(script_id as i32), extract_script_name_suffix(name));
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
                let name = deps.name.clone().unwrap_or_else(|| {
                    crate::interface::component_fallback_name(interface_id, comp_id)
                });
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
        let empty_catalog = ScriptCatalog::default();
        for (&id, data) in scripts {
            if let Ok(script) = decode_script(data, opcode_book, version) {
                let script_id = ScriptId(id as i32);
                let return_type = infer_return_type_for_script(
                    &script,
                    script_id,
                    version,
                    &empty_components,
                    &empty_enums,
                    &empty_catalog,
                    &self.script_signatures,
                );
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

    pub fn set_script_signature(&mut self, script_id: ScriptId, signature: ScriptSignature) {
        self.script_signatures.insert(script_id, signature);
    }

    /// Get a script's signature for cross-script call typing.
    pub fn script_signature(&self, id: ScriptId) -> Option<&ScriptSignature> {
        resolve_script_signature(&self.script_catalog, &self.script_signatures, id)
    }

    pub fn script_name_for(&self, script_id: ScriptId) -> Option<String> {
        self.script_catalog
            .export_name(script_id)
            .map(str::to_owned)
            .or_else(|| self.symbol_table.script_names.get(&script_id).cloned())
    }

    pub fn transpile_from_bytes(
        &self,
        data: &[u8],
        opcode_book: &OpcodeBook,
        version: u32,
        script_id: ScriptId,
    ) -> Result<TranspiledScript> {
        self.check_script_byte_limit(script_id, data.len())?;
        let script = decode_script(data, opcode_book, version)?;
        self.transpile(&script, script_id)
    }

    pub fn transpile(
        &self,
        script: &CompiledScript,
        script_id: ScriptId,
    ) -> Result<TranspiledScript> {
        self.transpile_structured(script, script_id)
    }

    pub fn transpile_to_ast(&self, script: &CompiledScript, script_id: ScriptId) -> Declaration {
        let codegen = CodeGen::new(&self.symbol_table);
        codegen.generate(script, script_id)
    }

    pub fn transpile_structured(
        &self,
        script: &CompiledScript,
        script_id: ScriptId,
    ) -> Result<TranspiledScript> {
        self.check_instruction_limit(script_id, script.code.len())?;

        let codegen = CodeGen::new(&self.symbol_table);
        let decl = codegen.generate(script, script_id);
        let diagnostics = self.collect_transpile_diagnostics(script_id, &decl);
        let writer = StructuredWriter::new(
            &self.symbol_table.var_map,
            &self.symbol_table.component_names,
            &self.symbol_table.enum_value_names,
            &self.script_catalog,
            &self.script_signatures,
        );
        let structured_script = writer.build_script(&decl);
        let body_source = structured_script.render();
        self.check_generated_source_limit(script_id, body_source.len())?;
        let diagnostics = self.finish_transpile_diagnostics(diagnostics, script_id, &body_source);
        let metadata = build_reversible_metadata(
            &structured_script,
            &diagnostics,
            self.build,
            self.subbuild,
            self.script_catalog.get(script_id),
        );
        let mut source = body_source;
        append_reversible_footer(&mut source, &metadata, &script_to_asm(script))?;
        Ok(TranspiledScript {
            source,
            referenced_vars: collect_var_refs(script),
            referenced_varbits: collect_varbit_refs(script),
            referenced_enums: collect_enum_refs(script),
            referenced_scripts: collect_script_refs(script),
            editable_structured: metadata.editable_structured,
            blocking_diagnostics: metadata.blocking_diagnostics.clone(),
            diagnostics,
        })
    }

    fn check_script_byte_limit(&self, script_id: ScriptId, script_bytes: usize) -> Result<()> {
        if self.limits.max_script_bytes != 0 && script_bytes > self.limits.max_script_bytes {
            bail!(
                "transpile guard hit for script {script_id}: compiled size {} bytes exceeds limit {}",
                script_bytes,
                self.limits.max_script_bytes
            );
        }
        Ok(())
    }

    fn check_instruction_limit(&self, script_id: ScriptId, instruction_count: usize) -> Result<()> {
        if self.limits.max_instructions != 0 && instruction_count > self.limits.max_instructions {
            bail!(
                "transpile guard hit for script {script_id}: instruction count {} exceeds limit {}",
                instruction_count,
                self.limits.max_instructions
            );
        }
        Ok(())
    }

    fn check_generated_source_limit(
        &self,
        script_id: ScriptId,
        generated_bytes: usize,
    ) -> Result<()> {
        if self.limits.max_generated_bytes != 0 && generated_bytes > self.limits.max_generated_bytes
        {
            bail!(
                "transpile guard hit for script {script_id}: generated source {} bytes exceeds limit {}",
                generated_bytes,
                self.limits.max_generated_bytes
            );
        }
        Ok(())
    }

    fn collect_transpile_diagnostics(
        &self,
        script_id: ScriptId,
        decl: &Declaration,
    ) -> Diagnostics {
        let mut diagnostics = Diagnostics::new();
        let source_id = self
            .script_catalog
            .export_name(script_id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script{script_id}"));

        for instruction in &decl.instructions {
            let span = Span::new(instruction.index, instruction.index + 1).with_source(&source_id);
            match instruction.command.as_str() {
                "gosub_with_params" => {
                    if let OperandNode::Script(id) = instruction.operand
                        && self.script_catalog.resolve_call_target(id).is_none()
                    {
                        diagnostics.error_at(span, format!("unresolved script ref {id}"));
                    }
                }
                "push_var"
                | "pop_var"
                | "push_varc_int"
                | "pop_varc_int"
                | "push_varc_string"
                | "pop_varc_string"
                | "push_varclan"
                | "push_varclan_long"
                | "push_varclan_string"
                | "push_varclansetting"
                | "push_varclansetting_long"
                | "push_varclansetting_string" => {
                    if let OperandNode::VarRef(var_ref) = &instruction.operand
                        && var_ref.name.is_none()
                    {
                        diagnostics.note_at(
                            span,
                            format!(
                                "unresolved var ref {}:{}",
                                var_ref.domain.as_label(),
                                var_ref.id
                            ),
                        );
                    }
                }
                "push_varbit" | "pop_varbit" | "push_varclanbit" | "push_varclansettingbit" => {
                    if let OperandNode::VarBitRef(varbit_ref) = &instruction.operand
                        && varbit_ref.name.is_none()
                    {
                        diagnostics
                            .note_at(span, format!("unresolved varbit ref {}", varbit_ref.id));
                    }
                }
                _ => {}
            }
        }

        diagnostics
    }

    fn finish_transpile_diagnostics(
        &self,
        mut diagnostics: Diagnostics,
        script_id: ScriptId,
        source: &str,
    ) -> Diagnostics {
        let source_id = self
            .script_catalog
            .export_name(script_id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script{script_id}"));
        let script_span = Span::new(0, 0).with_source(source_id);

        if source.contains("goto(") {
            diagnostics.warning_at(script_span.clone(), "parity miss: residual goto in output");
        }
        if source.contains("pop()") {
            diagnostics.warning_at(
                script_span.clone(),
                "bad stack state: residual pop() in output",
            );
        }
        if source.contains("// if (") {
            diagnostics.note_at(
                script_span,
                "parity miss: commented branch remains in structured output",
            );
        }

        diagnostics
    }
}

impl Default for Transpiler {
    fn default() -> Self {
        Self::new()
    }
}

fn build_reversible_metadata(
    script: &StructuredScript,
    diagnostics: &Diagnostics,
    build: u32,
    subbuild: u32,
    metadata: Option<&ScriptMetadata>,
) -> ReversibleMetadata {
    let packed_id = metadata.map_or(script.script_id.0, |entry| entry.packed_id.0);
    let group_id = metadata.map_or(packed_id >> 16, |entry| entry.group_id.0);
    let file_id = metadata.map_or((packed_id & 0xffff) as u16, |entry| entry.file_id);
    let blocking_diagnostics = blocking_diagnostics(diagnostics);
    ReversibleMetadata {
        format_version: reversible_format::REVERSIBLE_FORMAT_VERSION,
        build,
        subbuild,
        packed_id,
        group_id,
        file_id,
        script_id: script.script_id.0,
        export_name: script.function_name.clone(),
        raw_name: script.raw_name.clone(),
        editable_structured: editable_structured(diagnostics),
        structured_digest: structured_digest(script),
        blocking_diagnostics,
    }
}

pub struct TranspiledScript {
    pub source: String,
    pub referenced_vars: Vec<(VarDomain, u16)>,
    pub referenced_varbits: Vec<u16>,
    pub referenced_enums: Vec<u32>,
    pub referenced_scripts: Vec<ScriptId>,
    pub editable_structured: bool,
    pub blocking_diagnostics: Vec<String>,
    pub diagnostics: Diagnostics,
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
    let out = sanitize_ts_ident(value);
    if out == "unnamed" { String::new() } else { out }
}

pub fn script_function_name(script_id: ScriptId, script_name: Option<&str>) -> String {
    script_name
        .map(extract_script_name_suffix)
        .map(|name| sanitize_export_name(&name))
        .filter(|name| !name.is_empty() && name != "script")
        .unwrap_or_else(|| format!("script{script_id}"))
}

/// Strip `[clientscript,name]` / `[proc,name]` tag syntax to the suffix identifier.
pub fn extract_script_name_suffix(value: &str) -> String {
    parse_script_name_tag(value)
        .map(|parsed| parsed.short_name)
        .unwrap_or_else(|| value.trim().to_string())
}

#[expect(
    clippy::implicit_hasher,
    reason = "transpile APIs use default HashMap aliases across module boundaries"
)]
pub fn infer_return_type_for_script<S>(
    script: &CompiledScript,
    script_id: ScriptId,
    build: u32,
    component_names: &HashMap<u32, String, S>,
    enum_value_names: &HashMap<i32, String, S>,
    script_catalog: &ScriptCatalog,
    script_signatures: &HashMap<ScriptId, ScriptSignature>,
) -> String
where
    S: BuildHasher + Clone,
{
    let symbol_table = SymbolTable::new();
    let codegen = CodeGen::new(&symbol_table);
    let decl = codegen.generate(script, script_id);
    if build >= MIN_SCRIPT_BUILD {
        let blocks = build_cfg(
            &decl.instructions,
            &symbol_table.var_map,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
        );
        let structured = emit_structured(blocks);
        return detect_return_type(&structured).to_string();
    }

    let recovered = expr_recovery::ExprRecovery::new(
        &decl.instructions,
        &symbol_table.var_map,
        component_names,
        enum_value_names,
        script_catalog,
        script_signatures,
    )
    .recover();
    detect_return_type_from_recovered(&recovered).to_string()
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
    use super::{
        ScriptCatalog, ScriptId, ScriptKind, TranspileLimits, Transpiler,
        extract_script_name_suffix, infer_return_type_for_script, parse_script_name_tag,
        sanitize_export_name, sanitize_ts_ident,
    };
    use crate::script::{CompiledScript, Instruction, MIN_SCRIPT_BUILD, Operand};
    use std::collections::HashMap;

    fn test_script(code: Vec<Instruction>) -> CompiledScript {
        CompiledScript {
            name: Some("[proc,test_script]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code,
        }
    }

    #[test]
    fn extract_script_name_suffix_parses_tag_syntax() {
        assert_eq!(
            "bank_build_init",
            extract_script_name_suffix("[clientscript,bank_build_init]")
        );
        assert_eq!(
            "script621",
            extract_script_name_suffix("[proc,script621](int $int0)")
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

    #[test]
    fn parse_script_name_tag_preserves_kind() {
        let parsed = parse_script_name_tag("[proc,stockmarket_choosecancel]").expect("tag");
        assert_eq!(ScriptKind::Proc, parsed.kind);
        assert_eq!("stockmarket_choosecancel", parsed.short_name);
    }

    #[test]
    fn sanitize_export_name_emits_valid_identifier() {
        assert_eq!("foo_bar", sanitize_export_name("foo-bar"));
        assert_eq!("_123abc", sanitize_export_name("123abc"));
    }

    #[test]
    fn transpile_guard_rejects_large_instruction_stream() {
        let script = test_script(vec![
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Int(0),
            };
            2
        ]);
        let Err(err) = Transpiler::new()
            .with_limits(TranspileLimits {
                max_script_bytes: 0,
                max_instructions: 1,
                max_generated_bytes: 0,
            })
            .transpile(&script, ScriptId(42))
        else {
            panic!("instruction guard should fail");
        };
        assert!(
            err.to_string()
                .contains("instruction count 2 exceeds limit 1"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn transpile_guard_rejects_large_generated_source() {
        let script = test_script(vec![Instruction {
            opcode: 0,
            command: "return".to_string(),
            operand: Operand::Int(0),
        }]);
        let Err(err) = Transpiler::new()
            .with_limits(TranspileLimits {
                max_script_bytes: 0,
                max_instructions: 0,
                max_generated_bytes: 16,
            })
            .transpile(&script, ScriptId(7))
        else {
            panic!("generated source guard should fail");
        };
        assert!(
            err.to_string().contains("generated source"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn infer_return_type_detects_value_return_without_cfg() {
        let script = test_script(vec![
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Int(0),
            },
        ]);
        let component_names = HashMap::<u32, String>::new();
        let enum_value_names = HashMap::<i32, String>::new();

        assert_eq!(
            infer_return_type_for_script(
                &script,
                ScriptId(91),
                MIN_SCRIPT_BUILD,
                &component_names,
                &enum_value_names,
                &ScriptCatalog::default(),
                &HashMap::new(),
            ),
            "number"
        );
    }

    #[test]
    fn infer_return_type_ignores_unreachable_void_return_with_cfg() {
        let script = test_script(vec![
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Int(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Int(0),
            },
        ]);
        let component_names = HashMap::<u32, String>::new();
        let enum_value_names = HashMap::<i32, String>::new();

        assert_eq!(
            infer_return_type_for_script(
                &script,
                ScriptId(92),
                MIN_SCRIPT_BUILD,
                &component_names,
                &enum_value_names,
                &ScriptCatalog::default(),
                &HashMap::new(),
            ),
            "number"
        );
    }
}
