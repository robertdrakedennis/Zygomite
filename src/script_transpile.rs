// Legacy transpiler: builder methods return Self for chaining;
// unused_self on helper methods that mirror the new transpile API.
#![allow(clippy::return_self_not_must_use, clippy::unused_self)]

use crate::config::EnumEntry;
use crate::error::Result;
use crate::script::{
    CompiledScript, Instruction, OpcodeBook, Operand, VarBitRef, VarRef, decode_script,
};
use crate::vars::VarDomain;
use std::collections::{BTreeMap, HashMap};

pub struct Transpiler {
    enum_map: HashMap<u32, String>,
    var_map: HashMap<(VarDomain, u16), String>,
    varbit_map: HashMap<u16, String>,
    param_map: HashMap<u32, String>,
    script_names: HashMap<i32, String>,
}

impl Transpiler {
    pub fn new() -> Self {
        Self {
            enum_map: HashMap::new(),
            var_map: HashMap::new(),
            varbit_map: HashMap::new(),
            param_map: HashMap::new(),
            script_names: HashMap::new(),
        }
    }

    pub fn with_enums(mut self, enums: &BTreeMap<u32, EnumEntry>) -> Self {
        for (id, entry) in enums {
            self.enum_map.insert(*id, format!("enum_{}", entry.id));
        }
        self
    }

    pub fn with_vars(
        mut self,
        varps: &HashMap<VarDomain, BTreeMap<u32, crate::vars::VarEntry>>,
    ) -> Self {
        for (domain, vars) in varps {
            for (id, var) in vars {
                let key = (*domain, *id as u16);
                self.var_map.insert(key, var.var_name.clone());
            }
        }
        self
    }

    pub fn with_varbits(mut self, varbits: &BTreeMap<u32, crate::vars::VarBitEntry>) -> Self {
        for (id, varbit) in varbits {
            self.varbit_map
                .insert(*id as u16, varbit.varbit_name.clone());
        }
        self
    }

    pub fn with_params(mut self, params: &BTreeMap<u32, crate::config::ParamEntry>) -> Self {
        for (id, param) in params {
            self.param_map.insert(*id, format!("param_{}", param.id));
        }
        self
    }

    pub fn with_script_names(
        mut self,
        scripts: &BTreeMap<u32, Vec<u8>>,
        opcode_book: &OpcodeBook,
        version: u32,
    ) -> Self {
        for (&script_id, data) in scripts {
            if let Ok(script) = decode_script(data, opcode_book, version)
                && let Some(name) = &script.name
            {
                self.script_names.insert(script_id as i32, name.clone());
            }
        }
        self
    }

    pub fn script_name_for(&self, script_id: i32) -> Option<String> {
        self.script_names.get(&script_id).cloned()
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
        let mut out = String::new();
        out.push_str("// @ts-nocheck - Auto-generated CS2 to TypeScript\n");
        out.push_str("import { VARS, VARBITS, ENUMS, PARAMS } from './index';\n");
        out.push('\n');
        if let Some(name) = &script.name {
            out.push_str("// script name: ");
            out.push_str(name);
            out.push('\n');
        }
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "// script_{script_id}: locals(int={}, obj={}, long={}) args(int={}, obj={}, long={})",
            script.local_count_int,
            script.local_count_object,
            script.local_count_long,
            script.argument_count_int,
            script.argument_count_object,
            script.argument_count_long
        );

        let locals_int = script.local_count_int as usize;
        let locals_obj = script.local_count_object as usize;
        let locals_long = script.local_count_long as usize;
        let args_int = script.argument_count_int as usize;
        let args_obj = script.argument_count_object as usize;
        let args_long = script.argument_count_long as usize;

        out.push('\n');
        out.push_str("const ");
        out.push_str(&Self::format_locals_decl(
            locals_int,
            locals_obj,
            locals_long,
            args_int,
            args_obj,
            args_long,
        ));
        out.push_str(";\n\n");
        out.push_str("// --- instruction stream ---\n");
        for (i, instruction) in script.code.iter().enumerate() {
            let _ = writeln!(
                out,
                "{:05}: {}",
                i,
                self.format_instruction_body(instruction)
            );
        }

        TranspiledScript {
            source: out,
            referenced_vars: collect_var_refs(script),
            referenced_varbits: collect_varbit_refs(script),
            referenced_enums: collect_enum_refs(script),
            referenced_scripts: collect_script_refs(script),
        }
    }

    fn format_locals_decl(
        locals_int: usize,
        locals_obj: usize,
        locals_long: usize,
        args_int: usize,
        args_obj: usize,
        args_long: usize,
    ) -> String {
        let mut parts = Vec::new();
        for i in 0..args_int {
            parts.push(format!("arg_int_{i}: number"));
        }
        for i in 0..args_obj {
            parts.push(format!("arg_obj_{i}: string"));
        }
        for i in 0..args_long {
            parts.push(format!("arg_long_{i}: bigint"));
        }
        for i in 0..locals_int {
            parts.push(format!("local_int_{i}: number"));
        }
        for i in 0..locals_obj {
            parts.push(format!("local_obj_{i}: string"));
        }
        for i in 0..locals_long {
            parts.push(format!("local_long_{i}: bigint"));
        }
        if parts.is_empty() {
            "_".to_string()
        } else {
            parts.join(", ")
        }
    }

    fn format_instruction_body(&self, instruction: &Instruction) -> String {
        let op_raw = |operand: &Operand| format_operand_raw(operand);
        match instruction.command.as_str() {
            "push_constant_int" => {
                if let Operand::Int(v) = &instruction.operand {
                    format!("push({v});")
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "push_long_constant" => {
                if let Operand::Long(v) = &instruction.operand {
                    format!("push({v}n);")
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "push_constant_string" => {
                if let Operand::Str(s) = &instruction.operand {
                    format!("push(\"{}\");", escape_ts_string(s))
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "push_var" | "pop_var" => {
                if let Operand::VarRef(var_ref) = &instruction.operand {
                    self.format_var_access(var_ref)
                } else {
                    format!("VAR({});", op_raw(&instruction.operand))
                }
            }
            "push_varbit" | "pop_varbit" => {
                if let Operand::VarBitRef(varbit_ref) = &instruction.operand {
                    self.format_varbit_access(varbit_ref)
                } else {
                    format!("VARBIT({});", op_raw(&instruction.operand))
                }
            }
            "push_varc_int"
            | "push_varc_string"
            | "push_varclan"
            | "push_varclan_long"
            | "push_varclan_string"
            | "push_varclansetting"
            | "push_varclansetting_long"
            | "push_varclansetting_string" => {
                if let Operand::VarRef(var_ref) = &instruction.operand {
                    self.format_var_access(var_ref)
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "pop_varc_int" | "pop_varc_string" => {
                if let Operand::VarRef(var_ref) = &instruction.operand {
                    if let Some(name) = self.var_map.get(&(var_ref.domain, var_ref.id)) {
                        format!("{name} = pop();")
                    } else {
                        format!(
                            "VARS.get({} * 1000000 + {}) = pop();",
                            u64::from(var_ref.domain),
                            var_ref.id
                        )
                    }
                } else {
                    format!("pop({});", op_raw(&instruction.operand))
                }
            }
            "push_varclanbit" | "push_varclansettingbit" => {
                if let Operand::VarBitRef(varbit_ref) = &instruction.operand {
                    self.format_varbit_access(varbit_ref)
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "push_int_local" => {
                if let Operand::Local(idx) = &instruction.operand {
                    format!("push(local_int_{idx});")
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "pop_int_local" => {
                if let Operand::Local(idx) = &instruction.operand {
                    format!("local_int_{idx} = pop();")
                } else {
                    format!("pop({});", op_raw(&instruction.operand))
                }
            }
            "push_string_local" => {
                if let Operand::Local(idx) = &instruction.operand {
                    format!("push(local_obj_{idx});")
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "pop_string_local" => {
                if let Operand::Local(idx) = &instruction.operand {
                    format!("local_obj_{idx} = pop();")
                } else {
                    format!("pop({});", op_raw(&instruction.operand))
                }
            }
            "push_long_local" => {
                if let Operand::Local(idx) = &instruction.operand {
                    format!("push(local_long_{idx});")
                } else {
                    format!("push({});", op_raw(&instruction.operand))
                }
            }
            "pop_long_local" => {
                if let Operand::Local(idx) = &instruction.operand {
                    format!("local_long_{idx} = pop();")
                } else {
                    format!("pop({});", op_raw(&instruction.operand))
                }
            }
            "branch" => {
                if let Operand::Branch(target) = &instruction.operand {
                    format!("goto({target});")
                } else {
                    format!("goto({});", op_raw(&instruction.operand))
                }
            }
            "branch_not" => format!("if (!pop()) goto({});", op_raw(&instruction.operand)),
            "branch_equals" => format!(
                "if (pop() == pop()) goto({});",
                op_raw(&instruction.operand)
            ),
            "branch_if_true" => {
                format!("if (pop()) goto({});", op_raw(&instruction.operand))
            }
            "branch_if_false" => {
                format!("if (!pop()) goto({});", op_raw(&instruction.operand))
            }
            "gosub_with_params" => {
                if let Operand::Script(id) = &instruction.operand {
                    if let Some(name) = self.script_names.get(id) {
                        format!("{}(pop());", sanitize_ts_ident(name))
                    } else {
                        format!("script_{id}(pop());")
                    }
                } else {
                    format!("call({});", op_raw(&instruction.operand))
                }
            }
            "switch" => {
                let Operand::Switch(cases) = &instruction.operand else {
                    return format!("switch({});", op_raw(&instruction.operand));
                };
                let arms: Vec<String> = cases
                    .iter()
                    .map(|c| format!("case {}: goto({});", c.value, c.target))
                    .collect();
                format!(
                    "switch(pop()) {{\n        {}\n    }}",
                    arms.join("\n        ")
                )
            }
            "join_string" => {
                if let Operand::Count(n) = &instruction.operand {
                    format!("push(pop().concat(...pop_multi({n})));")
                } else {
                    format!("concat({});", op_raw(&instruction.operand))
                }
            }
            "define_array" => {
                if let Operand::Array(id) = &instruction.operand {
                    format!("array_{id} = [];")
                } else {
                    format!("define_array({});", op_raw(&instruction.operand))
                }
            }
            "cc_create" => {
                if let Operand::Int(id) = &instruction.operand {
                    format!("UI.create({id});")
                } else {
                    format!("UI.create({});", op_raw(&instruction.operand))
                }
            }
            "cc_delete" => "UI.delete(pop() as number);".to_string(),
            "cc_settext" => "UI.setText(pop() as number, pop() as string);".to_string(),
            "cc_setgraphic" => "UI.setGraphic(pop() as number, pop() as number);".to_string(),
            "cc_sethide" => "UI.setHide(pop() as number, pop() as boolean);".to_string(),
            _ => format!(
                "{}({});",
                format_command_name(&instruction.command),
                op_raw(&instruction.operand)
            ),
        }
    }

    fn format_var_access(&self, var_ref: &VarRef) -> String {
        let key = (var_ref.domain, var_ref.id);
        if let Some(name) = self.var_map.get(&key) {
            if var_ref.transmog {
                format!(
                    "push(VARS.get({} * 1000000 + {}) as {} /* {name}:transmog */);",
                    u64::from(var_ref.domain),
                    var_ref.id,
                    var_type_hint(var_ref.domain)
                )
            } else {
                format!("push({name});")
            }
        } else {
            format!(
                "push(VARS.get({} * 1000000 + {})!);",
                u64::from(var_ref.domain),
                var_ref.id
            )
        }
    }

    fn format_varbit_access(&self, varbit_ref: &VarBitRef) -> String {
        if let Some(name) = self.varbit_map.get(&varbit_ref.id) {
            if varbit_ref.transmog {
                format!(
                    "push(VARBITS.get({}) as {} /* {name}:transmog */);",
                    varbit_ref.id, name
                )
            } else {
                format!("push({name});")
            }
        } else {
            format!("push(VARBITS.get({})!);", varbit_ref.id)
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

fn var_type_hint(domain: VarDomain) -> &'static str {
    match domain {
        VarDomain::Player
        | VarDomain::Npc
        | VarDomain::Client
        | VarDomain::World
        | VarDomain::Region
        | VarDomain::Object
        | VarDomain::Clan
        | VarDomain::ClanSetting
        | VarDomain::Controller
        | VarDomain::PlayerGroup
        | VarDomain::Global => "number",
    }
}

fn format_command_name(cmd: &str) -> String {
    let cmd = cmd.replace('_', "");
    sanitize_ts_ident(&cmd)
}

fn format_operand_raw(operand: &Operand) -> String {
    match operand {
        Operand::Int(v) => v.to_string(),
        Operand::Long(v) => format!("{v}n"),
        Operand::Str(s) => format!("\"{}\"", escape_ts_string(s)),
        Operand::Local(idx) => format!("local_{idx}"),
        Operand::VarRef(v) => format!("{}:{}", v.domain.as_label(), v.id),
        Operand::VarBitRef(v) => format!("varbit:{}", v.id),
        Operand::Branch(target) => format!("->{target}"),
        Operand::Switch(cases) => {
            let arms: Vec<String> = cases
                .iter()
                .map(|c| format!("{}->{}", c.value, c.target))
                .collect();
            format!("{{{}}}", arms.join(", "))
        }
        Operand::Script(id) => format!("script_{id}"),
        Operand::Array(id) => format!("array_{id}"),
        Operand::Count(n) => format!("count_{n}"),
        Operand::Byte(b) => b.to_string(),
    }
}

fn collect_var_refs(script: &CompiledScript) -> Vec<(VarDomain, u16)> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let Operand::VarRef(v) = &instruction.operand {
            refs.push((v.domain, v.id));
        }
    }
    refs
}

fn collect_varbit_refs(script: &CompiledScript) -> Vec<u16> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let Operand::VarBitRef(v) = &instruction.operand {
            refs.push(v.id);
        }
    }
    refs
}

fn collect_enum_refs(script: &CompiledScript) -> Vec<u32> {
    let mut refs = Vec::new();
    for instruction in &script.code {
        if let Operand::Int(v) = &instruction.operand
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
        if let Operand::Script(id) = &instruction.operand {
            refs.push(*id);
        }
    }
    refs
}

fn escape_ts_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn sanitize_ts_ident(name: &str) -> String {
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
    use super::*;

    #[test]
    fn sanitize_ts_ident_normalizes_invalid_identifiers() {
        assert_eq!("hello_world", sanitize_ts_ident("hello/world"));
        assert_eq!("_123abc", sanitize_ts_ident("123abc"));
        assert_eq!("unnamed", sanitize_ts_ident(""));
        assert_eq!("foo_bar", sanitize_ts_ident("foo bar"));
    }
}
