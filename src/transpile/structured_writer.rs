use super::ast::{Declaration, TypeAnnotation};
use super::cfg::{StructuredStatement, build_cfg, generate_structured};
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
        out.push_str("// @ts-nocheck - Auto-generated CS2 to TypeScript\n");
        out.push_str("import { VARS, VARBITS, ENUMS, PARAMS } from './index';\n\n");

        if let Some(ref name) = decl.name {
            out.push_str("// script name: ");
            out.push_str(name);
            out.push('\n');
        }

        let _ = writeln!(
            &mut out,
            "// script_{}: locals(int={}, obj={}, long={}) args(int={}, obj={}, long={})\n",
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

        out.push_str("const ");
        let mut locals_parts: Vec<String> = Vec::new();
        for arg in &decl.arguments {
            locals_parts.push(format!("{}: {}", arg.name, arg.type_annotation.as_str()));
        }
        for local in &decl.locals {
            locals_parts.push(format!(
                "{}: {}",
                local.name,
                local.type_annotation.as_str()
            ));
        }
        out.push_str(&locals_parts.join(", "));
        out.push_str(";\n\n");

        let blocks = build_cfg(decl.instructions.clone());
        let structured = generate_structured(blocks);

        self.write_structured(&structured, &mut out);

        out
    }

    fn write_structured(&mut self, stmts: &[StructuredStatement], out: &mut String) {
        for stmt in stmts {
            self.write_structured_stmt(stmt, out);
        }
    }

    fn write_structured_stmt(&mut self, stmt: &StructuredStatement, out: &mut String) {
        match stmt {
            StructuredStatement::While { condition, body } => {
                self.write_indent(out);
                if condition == "true" {
                    out.push_str("while (true) {\n");
                } else {
                    out.push_str("while (");
                    out.push_str(condition);
                    out.push_str(") {\n");
                }
                self.indent += 1;
                self.write_structured(body, out);
                self.indent -= 1;
                self.write_indent(out);
                out.push('}');
                out.push('\n');
            }
            StructuredStatement::If {
                condition,
                then_case,
                else_case,
            } => {
                self.write_indent(out);
                out.push_str("if (");
                out.push_str(condition);
                out.push_str(") {\n");
                self.indent += 1;
                self.write_structured(then_case, out);
                self.indent -= 1;
                self.write_indent(out);
                out.push('}');
                if let Some(else_body) = else_case {
                    out.push_str(" else {\n");
                    self.indent += 1;
                    self.write_structured(else_body, out);
                    self.indent -= 1;
                    self.write_indent(out);
                    out.push('}');
                }
                out.push('\n');
            }
            StructuredStatement::Switch {
                expression,
                cases,
                default,
            } => {
                self.write_indent(out);
                out.push_str("switch (");
                out.push_str(expression);
                out.push_str(") {\n");
                self.indent += 1;
                for case_ in cases {
                    self.write_indent(out);
                    out.push_str("case ");
                    out.push_str(&case_.value.to_string());
                    out.push('\n');
                    self.indent += 1;
                    self.write_structured(&case_.body, out);
                    self.write_indent(out);
                    out.push_str("break;\n");
                    self.indent -= 1;
                }
                if let Some(default_body) = default {
                    self.write_indent(out);
                    out.push_str("default:\n");
                    self.indent += 1;
                    self.write_structured(default_body, out);
                    self.indent -= 1;
                }
                self.indent -= 1;
                self.write_indent(out);
                out.push('}');
                out.push('\n');
            }
            StructuredStatement::Assignment { target, value } => {
                self.write_indent(out);
                out.push_str(target);
                out.push_str(" = ");
                out.push_str(value);
                out.push_str(";\n");
            }
            StructuredStatement::Expression { expr } => {
                self.write_indent(out);
                out.push_str(expr);
                out.push_str(";\n");
            }
            StructuredStatement::Goto { target } => {
                self.write_indent(out);
                let _ = writeln!(out, "goto({target}); // block_{target}");
            }
            StructuredStatement::Return { value } => {
                self.write_indent(out);
                out.push_str("return");
                if let Some(v) = value {
                    out.push(' ');
                    out.push_str(v);
                }
                out.push_str(";\n");
            }
            StructuredStatement::Comment(text) => {
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
