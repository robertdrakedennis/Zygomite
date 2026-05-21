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
pub use cfg::{Block, StructuredStmt, SwitchCaseStmt, build_cfg, emit_structured};
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

pub struct Transpiler {
    symbol_table: SymbolTable,
}

impl Transpiler {
    pub fn new() -> Self {
        Self {
            symbol_table: SymbolTable::new(),
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
                names.insert(script_id as i32, name.clone());
            }
        }
        self.symbol_table.script_names = names;
        self
    }

    pub fn script_name_for(&self, script_id: i32) -> Option<String> {
        self.symbol_table.script_names.get(&script_id).cloned()
    }

    pub fn transpile_from_bytes(
        &self,
        data: &[u8],
        opcode_book: &OpcodeBook,
        version: u32,
        script_id: i32,
    ) -> Result<TranspiledScript> {
        let script = decode_script(data, opcode_book, version)?;
        Ok(self.transpile(&script, script_id))
    }

    pub fn transpile(&self, script: &CompiledScript, script_id: i32) -> TranspiledScript {
        self.transpile_structured(script, script_id)
    }

    pub fn transpile_to_ast(&self, script: &CompiledScript, script_id: i32) -> Declaration {
        let codegen = CodeGen::new(self.symbol_table.clone());
        codegen.generate(script, script_id)
    }

    pub fn transpile_structured(
        &self,
        script: &CompiledScript,
        script_id: i32,
    ) -> TranspiledScript {
        let codegen = CodeGen::new(self.symbol_table.clone());
        let decl = codegen.generate(script, script_id);
        let mut writer = StructuredWriter::new();
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
    pub referenced_scripts: Vec<i32>,
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

fn collect_script_refs(script: &CompiledScript) -> Vec<i32> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let crate::script::Operand::Script(id) = &instruction.operand {
            refs.push(*id);
        }
    }
    refs
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
    use super::sanitize_ts_ident;

    #[test]
    fn sanitize_ident() {
        assert_eq!("hello_world", sanitize_ts_ident("hello/world"));
        assert_eq!("_123abc", sanitize_ts_ident("123abc"));
        assert_eq!("unnamed", sanitize_ts_ident(""));
        assert_eq!("foo_bar", sanitize_ts_ident("foo bar"));
    }
}
