use super::ScriptId;
use super::ScriptSignature;
use super::ast::{Declaration, TypeAnnotation};
use super::cfg::{StructuredStmt, build_cfg, detect_return_type, emit_structured};
use std::collections::HashMap;
use std::fmt::Write as _;

pub struct StructuredWriter {
    indent: usize,
    component_names: HashMap<u32, String>,
    enum_value_names: HashMap<i32, String>,
    script_signatures: HashMap<ScriptId, ScriptSignature>,
    script_names: HashMap<ScriptId, String>,
}

impl StructuredWriter {
    pub fn new(
        component_names: HashMap<u32, String>,
        enum_value_names: HashMap<i32, String>,
        script_signatures: HashMap<ScriptId, ScriptSignature>,
        script_names: HashMap<ScriptId, String>,
    ) -> Self {
        Self {
            indent: 0,
            component_names,
            enum_value_names,
            script_signatures,
            script_names,
        }
    }

    pub fn write_declaration(&mut self, decl: &Declaration) -> String {
        let mut out = String::new();

        // ── Header comment ──
        let _ = writeln!(&mut out, "// Auto-generated CS2 to TypeScript");
        if let Some(ref name) = decl.name {
            let _ = writeln!(&mut out, "// Script name: {name}");
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
            decl.instructions.clone(),
            &self.component_names,
            &self.enum_value_names,
            &self.script_signatures,
            &self.script_names,
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

        // ── Detect which imports are needed ──
        let needs_vars = body.contains("VARS.");
        let needs_varbits = body.contains("VARBITS.");
        let needs_enums = body.contains("ENUMS.");
        let needs_params = body.contains("PARAMS.");
        let needs_components = body.contains("ComponentId.");
        let needs_enum_refs = body.contains("Enum_");
        let needs_db = body.contains("DB_TABLES.");

        // ── Emit imports ──
        if needs_vars
            || needs_varbits
            || needs_enums
            || needs_params
            || needs_components
            || needs_enum_refs
            || needs_db
        {
            let mut imports = Vec::new();
            if needs_vars {
                imports.push("VARS");
            }
            if needs_varbits {
                imports.push("VARBITS");
            }
            if needs_enums || needs_enum_refs {
                imports.push("ENUMS");
            }
            if needs_params {
                imports.push("PARAMS");
            }
            if needs_components {
                imports.push("ComponentId");
            }
            if needs_db {
                imports.push("DB_TABLES");
            }
            let _ = writeln!(
                &mut out,
                "import {{ {} }} from './index';",
                imports.join(", ")
            );
        }
        out.push('\n');

        // ── Function signature with detected return type ──
        let return_type = detect_return_type(&structured);
        let resolved_name = decl
            .name
            .as_deref()
            .or_else(|| self.script_names.get(&decl.script_id).map(String::as_str));
        let function_name = super::script_function_name(decl.script_id, resolved_name);
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

impl Default for StructuredWriter {
    fn default() -> Self {
        Self::new(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
    }
}
