use super::ast::{Declaration, TypeAnnotation};
use super::cfg::{StructuredStmt, build_cfg, detect_return_type, emit_structured};
use super::{ScriptCatalog, ScriptId, ScriptSignature, resolve_script_signature};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

pub struct StructuredWriter<'a> {
    indent: usize,
    var_names: &'a HashMap<(crate::vars::VarDomain, u16), String>,
    component_names: &'a HashMap<u32, String>,
    enum_value_names: &'a HashMap<i32, String>,
    script_catalog: &'a ScriptCatalog,
    script_signatures: &'a HashMap<ScriptId, ScriptSignature>,
}

impl<'a> StructuredWriter<'a> {
    pub fn new(
        var_names: &'a HashMap<(crate::vars::VarDomain, u16), String>,
        component_names: &'a HashMap<u32, String>,
        enum_value_names: &'a HashMap<i32, String>,
        script_catalog: &'a ScriptCatalog,
        script_signatures: &'a HashMap<ScriptId, ScriptSignature>,
    ) -> Self {
        Self {
            indent: 0,
            var_names,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
        }
    }

    pub fn write_declaration(&mut self, decl: &Declaration) -> String {
        let mut out = String::new();

        // ── Header comment ──
        let _ = writeln!(&mut out, "// Auto-generated CS2 to TypeScript");
        if let Some(ref name) = decl.name {
            let _ = writeln!(&mut out, "// Script name: {name}");
        }
        if let Some(metadata) = self.script_catalog.get(decl.script_id) {
            let _ = writeln!(
                &mut out,
                "// Meta: packed={} group={} file={} kind={} short_name={}",
                metadata.packed_id,
                metadata.group_id,
                metadata.file_id,
                metadata.kind.as_label(),
                metadata.short_name
            );
        }
        let _ = writeln!(
            &mut out,
            "// script_{}: locals(int={}, obj={}, long={}) args(int={}, obj={}, long={})",
            decl.script_id,
            decl.locals
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::Number))
                .count(),
            decl.locals
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::String))
                .count(),
            decl.locals
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::BigInt))
                .count(),
            decl.arguments
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::Number))
                .count(),
            decl.arguments
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::String))
                .count(),
            decl.arguments
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::BigInt))
                .count(),
        );

        // ── Build function body ──
        let blocks = build_cfg(
            &decl.instructions,
            self.var_names,
            self.component_names,
            self.enum_value_names,
            self.script_catalog,
            self.script_signatures,
        );
        let structured = emit_structured(blocks);

        let mut body = String::new();
        self.indent = 1;
        // Local variable declarations
        for local in &decl.locals {
            let _ = writeln!(
                &mut body,
                "    let {name}: {type_};",
                name = local.name,
                type_ = local.type_annotation.as_str()
            );
        }
        if !decl.locals.is_empty() {
            body.push('\n');
        }

        let array_ids = collect_array_ids(&decl.instructions);
        if !array_ids.is_empty() {
            for array_id in &array_ids {
                let _ = writeln!(&mut body, "    let array_{array_id}: number[] = [];");
            }
            body.push('\n');
        }

        self.write_structured(&structured, &mut body);

        self.write_imports(decl, &mut out);
        out.push('\n');

        // ── Function signature with detected return type ──
        let return_type =
            resolve_script_signature(self.script_catalog, self.script_signatures, decl.script_id)
                .and_then(|signature| {
                    (signature.return_type != "unknown").then_some(signature.return_type.as_str())
                })
                .unwrap_or_else(|| detect_return_type(&structured));
        let function_name = self
            .script_catalog
            .export_name(decl.script_id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script{}", decl.script_id));
        let _ = writeln!(
            &mut out,
            "export function {function_name}({args}): {return_type} {{",
            args = decl
                .arguments
                .iter()
                .map(|a| format!("{}: {}", a.name, a.type_annotation.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        );

        out.push_str(&body);
        out.push_str("}\n");

        out
    }

    fn write_imports(&self, decl: &Declaration, out: &mut String) {
        let mut index_imports = BTreeSet::new();
        let mut enum_imports = BTreeSet::new();
        let mut module_imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for instruction in &decl.instructions {
            match instruction.command.as_str() {
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
                    if let super::ast::OperandNode::VarRef(var_ref) = &instruction.operand
                        && var_ref.name.is_none()
                    {
                        index_imports.insert("VARS");
                    }
                }
                "push_varbit" | "pop_varbit" | "push_varclanbit" | "push_varclansettingbit" => {
                    if let super::ast::OperandNode::VarBitRef(varbit_ref) = &instruction.operand
                        && varbit_ref.name.is_none()
                    {
                        index_imports.insert("VARBITS");
                    }
                }
                "cc_create" => {
                    if let super::ast::OperandNode::Int(component_id) = instruction.operand
                        && self.component_names.contains_key(&(component_id as u32))
                    {
                        index_imports.insert("ComponentId");
                    }
                }
                "push_constant_string" => {
                    if let super::ast::OperandNode::Int(value) = instruction.operand
                        && let Some(qualified) = self.enum_value_names.get(&value)
                        && let Some((object, _)) = qualified.split_once('.')
                    {
                        enum_imports.insert(object.to_string());
                    }
                }
                "gosub_with_params" => {
                    if let super::ast::OperandNode::Script(group_id) = instruction.operand
                        && let Some(target) = self.script_catalog.resolve_call_target(group_id)
                        && target.packed_id != decl.script_id
                    {
                        module_imports
                            .entry(format!("./{}", target.module_name))
                            .or_default()
                            .insert(target.export_name.clone());
                    }
                }
                _ => {}
            }
        }

        if !index_imports.is_empty() {
            let names = index_imports.into_iter().collect::<Vec<_>>().join(", ");
            let _ = writeln!(out, "import {{ {names} }} from './index';");
        }
        if !enum_imports.is_empty() {
            let names = enum_imports.into_iter().collect::<Vec<_>>().join(", ");
            let _ = writeln!(out, "import {{ {names} }} from './enums';");
        }
        for (module, names) in module_imports {
            let names = names.into_iter().collect::<Vec<_>>().join(", ");
            let _ = writeln!(out, "import {{ {names} }} from '{module}';");
        }
    }

    fn write_structured(&mut self, stmts: &[StructuredStmt], out: &mut String) {
        for stmt in stmts {
            self.write_stmt(stmt, out);
        }
    }

    fn write_stmt(&mut self, stmt: &StructuredStmt, out: &mut String) {
        match stmt {
            StructuredStmt::While { body } => {
                self.write_indent(out);
                out.push_str("while (true) {\n");
                self.indent += 1;
                self.write_structured(body, out);
                self.indent -= 1;
                self.write_indent(out);
                out.push_str("}\n");
            }
            StructuredStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                self.write_indent(out);
                out.push_str("if (");
                out.push_str(condition);
                out.push_str(") {\n");
                self.indent += 1;
                self.write_structured(then_body, out);
                self.indent -= 1;
                if let Some(else_b) = else_body {
                    self.write_indent(out);
                    out.push_str("} else {\n");
                    self.indent += 1;
                    self.write_structured(else_b, out);
                    self.indent -= 1;
                }
                self.write_indent(out);
                out.push_str("}\n");
            }
            StructuredStmt::Switch { expr, cases } => {
                self.write_indent(out);
                out.push_str("switch (");
                out.push_str(expr);
                out.push_str(") {\n");
                self.indent += 1;
                for case_ in cases {
                    self.write_indent(out);
                    out.push_str("case ");
                    out.push_str(&case_.value.to_string());
                    out.push_str(":\n");
                    self.indent += 1;
                    self.write_structured(&case_.body, out);
                    self.write_indent(out);
                    out.push_str("break;\n");
                    self.indent -= 1;
                }
                self.indent -= 1;
                self.write_indent(out);
                out.push_str("}\n");
            }
            StructuredStmt::Assignment { target, value } => {
                self.write_indent(out);
                out.push_str(target);
                out.push_str(" = ");
                out.push_str(value);
                out.push_str(";\n");
            }
            StructuredStmt::Expr { expr } => {
                self.write_indent(out);
                out.push_str(expr);
                out.push_str(";\n");
            }
            StructuredStmt::Goto { target } => {
                self.write_indent(out);
                out.push_str("goto(");
                out.push_str(&target.to_string());
                out.push_str(");\n");
            }
            StructuredStmt::Return { value } => {
                self.write_indent(out);
                out.push_str("return");
                if let Some(v) = value {
                    out.push(' ');
                    out.push_str(v);
                }
                out.push_str(";\n");
            }
            StructuredStmt::Break => {
                self.write_indent(out);
                out.push_str("break;\n");
            }
            StructuredStmt::Continue => {
                self.write_indent(out);
                out.push_str("continue;\n");
            }
            StructuredStmt::Comment(text) => {
                self.write_indent(out);
                out.push_str("// ");
                out.push_str(text);
                out.push('\n');
            }
        }
    }

    fn write_indent(&self, out: &mut String) {
        for _ in 0..self.indent {
            out.push_str("    ");
        }
    }
}

fn collect_array_ids(instructions: &[super::ast::InstructionNode]) -> Vec<u32> {
    use super::ast::OperandNode;
    let mut ids = Vec::new();
    for instr in instructions {
        if instr.command == "define_array"
            && let OperandNode::Array(id) = instr.operand
        {
            ids.push(id as u32);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}
