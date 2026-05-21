use super::ast::{Declaration, TypeAnnotation};
use super::cfg::{StructuredStmt, build_cfg, emit_structured};
use std::fmt::Write as _;

pub struct StructuredWriter {
    indent: usize,
}

impl StructuredWriter {
    pub fn new() -> Self {
        Self { indent: 0 }
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
        let blocks = build_cfg(decl.instructions.clone());
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
        self.write_structured(&structured, &mut body);

        // ── Detect which imports are needed ──
        let needs_vars = body.contains("VARS.");
        let needs_varbits = body.contains("VARBITS.");
        let needs_enums = body.contains("ENUMS.");
        let needs_params = body.contains("PARAMS.");

        // ── Emit imports ──
        if needs_vars || needs_varbits || needs_enums || needs_params {
            let mut imports = Vec::new();
            if needs_vars {
                imports.push("VARS");
            }
            if needs_varbits {
                imports.push("VARBITS");
            }
            if needs_enums {
                imports.push("ENUMS");
            }
            if needs_params {
                imports.push("PARAMS");
            }
            let _ = writeln!(
                &mut out,
                "import {{ {} }} from './index';",
                imports.join(", ")
            );
        }
        out.push('\n');

        // ── Function signature ──
        let _ = writeln!(
            &mut out,
            "export function script_{id}({args}): void {{",
            id = decl.script_id,
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

impl Default for StructuredWriter {
    fn default() -> Self {
        Self::new()
    }
}
