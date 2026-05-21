use super::ast::{Declaration, Expression, Statement, TypeAnnotation};
use super::cfg::ControlFlowGraph;
use std::fmt::Write as _;

pub struct StructuredWriter {
    indent: usize,
}

impl StructuredWriter {
    pub fn new() -> Self {
        Self { indent: 0 }
    }

    pub fn write_declaration_structured(
        &mut self,
        decl: &Declaration,
        cfg: &ControlFlowGraph,
    ) -> String {
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

        for block in &cfg.blocks {
            self.write_block(block, &cfg.blocks, &mut out);
        }

        out
    }

    fn write_block(
        &mut self,
        block: &super::cfg::BasicBlock,
        all_blocks: &[super::cfg::BasicBlock],
        out: &mut String,
    ) {
        let _ = writeln!(
            out,
            "// block_{} (successors: {:?})",
            block.index, block.successors
        );

        for stmt in &block.statements {
            self.write_statement(stmt, out);
        }

        if let Some(&first_successor) = block.successors.first()
            && first_successor > block.index
            && block.index > 0
        {
            let has_back_edge = all_blocks
                .iter()
                .any(|b| b.successors.contains(&block.index) && b.index > block.index);
            if has_back_edge && block.successors.len() == 2 {
                let _ = writeln!(out, "// loop detected");
            }
        }
    }

    fn write_statement(&mut self, stmt: &Statement, out: &mut String) {
        match stmt {
            Statement::Comment(text) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                let _ = writeln!(out, "// {text}");
            }
            Statement::ExpressionStatement(es) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                self.write_expression(&es.expr, out);
                if es.semicolon {
                    out.push(';');
                }
                out.push('\n');
            }
            Statement::VariableDeclaration(vd) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                let _ = writeln!(out, "const {}: {} = pop();", vd.name, vd.type_hint.as_str());
            }
            Statement::GotoStatement(gs) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                let _ = writeln!(out, "goto({});", gs.target);
            }
            Statement::IfStatement(if_) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                out.push_str("if (");
                self.write_expression(&if_.condition, out);
                out.push_str(") {\n");
                self.indent += 1;
                self.write_statement(&if_.then_branch, out);
                self.indent -= 1;
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                out.push_str("}\n");
                if let Some(else_) = &if_.else_branch {
                    for _ in 0..self.indent {
                        out.push_str("    ");
                    }
                    out.push_str("else {\n");
                    self.indent += 1;
                    self.write_statement(else_, out);
                    self.indent -= 1;
                    for _ in 0..self.indent {
                        out.push_str("    ");
                    }
                    out.push_str("}\n");
                }
            }
            Statement::SwitchStatement(sw) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                out.push_str("switch (");
                self.write_expression(&sw.discriminant, out);
                out.push_str(") {\n");
                for case_ in &sw.cases {
                    for _ in 0..self.indent {
                        out.push_str("    ");
                    }
                    let _ = writeln!(out, "case {}: goto({});", case_.value, case_.target);
                }
                if let Some(default) = sw.default_target {
                    for _ in 0..self.indent {
                        out.push_str("    ");
                    }
                    let _ = writeln!(out, "default: goto({default});");
                }
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                out.push_str("}\n");
            }
            Statement::ReturnStatement(rs) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                if let Some(ref val) = rs.value {
                    self.write_expression(val, out);
                    out.push_str(";\n");
                } else {
                    out.push_str("return;\n");
                }
            }
            Statement::CallStatement(cs) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                let _ = writeln!(
                    out,
                    "{}({});",
                    cs.callee,
                    self.format_arguments(&cs.arguments)
                );
            }
            Statement::Label(l) => {
                for _ in 0..self.indent {
                    out.push_str("    ");
                }
                let _ = writeln!(out, "// label {}", l.index);
            }
        }
    }

    fn write_expression(&self, expr: &Expression, out: &mut String) {
        match expr {
            Expression::NumberLiteral(n) => out.push_str(&n.value.to_string()),
            Expression::BigIntLiteral(n) => {
                out.push_str(&n.value.to_string());
                out.push('n');
            }
            Expression::StringLiteral(s) => {
                out.push('"');
                out.push_str(&escape_string(&s.value));
                out.push('"');
            }
            Expression::BooleanLiteral(b) => out.push_str(if b.value { "true" } else { "false" }),
            Expression::Identifier(id) => out.push_str(&id.name),
            Expression::ArrayAccess(aa) => {
                self.write_expression(&aa.array, out);
                out.push('[');
                self.write_expression(&aa.index, out);
                out.push(']');
            }
            Expression::PropertyAccess(pa) => {
                self.write_expression(&pa.object, out);
                out.push('.');
                out.push_str(&pa.property);
            }
            Expression::Call(c) => {
                self.write_expression(&c.callee, out);
                out.push('(');
                out.push_str(&self.format_arguments(&c.arguments));
                out.push(')');
            }
            Expression::BinaryOperation(bin) => {
                self.write_expression(&bin.left, out);
                out.push(' ');
                out.push_str(bin.op.as_str());
                out.push(' ');
                self.write_expression(&bin.right, out);
            }
            Expression::UnaryOperation(un) => {
                out.push_str(un.op.as_str());
                self.write_expression(&un.operand, out);
            }
            Expression::PushOperation(push) => {
                out.push_str("push(");
                self.write_expression(&push.value, out);
                out.push(')');
            }
            Expression::PopOperation(pop) => {
                out.push_str("pop()");
                if let Some(ref target) = pop.target {
                    out.push_str(" as ");
                    self.write_expression(target, out);
                }
            }
            Expression::GotoExpr(g) => {
                let _ = write!(out, "goto({})", g.target);
            }
        }
    }

    fn format_arguments(&self, args: &[Expression]) -> String {
        args.iter()
            .map(|arg| {
                let mut s = String::new();
                self.write_expression(arg, &mut s);
                s
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Default for StructuredWriter {
    fn default() -> Self {
        Self::new()
    }
}

fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
