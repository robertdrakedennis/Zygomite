use super::ast::{
    Declaration, Expression, ImportStatement, InstructionNode, OperandNode, Program, Statement,
    TypeAnnotation,
};
use std::fmt::Write;

pub struct Writer {
    indent: usize,
}

impl Writer {
    pub fn new() -> Self {
        Self { indent: 0 }
    }

    pub fn write_program(&mut self, program: &Program) -> String {
        let mut out = String::new();
        out.push_str("// @ts-nocheck - Auto-generated CS2 to TypeScript\n");
        for import in &program.imports {
            self.write_import(import, &mut out);
        }
        out.push('\n');
        if !program.comments.is_empty() {
            for comment in &program.comments {
                let _ = writeln!(&mut out, "// {comment}");
            }
        }
        for stmt in &program.statements {
            self.write_statement(stmt, &mut out);
        }
        out
    }

    fn write_import(&self, import: &ImportStatement, out: &mut String) {
        if import.is_type_only {
            let _ = writeln!(
                out,
                "import type {{ {} }} from '{module}';",
                import.named_exports.join(", "),
                module = import.module
            );
        } else {
            let _ = writeln!(
                out,
                "import {{ {} }} from '{module}';",
                import.named_exports.join(", "),
                module = import.module
            );
        }
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
        out.push_str("// --- instruction stream ---\n");
        for instr in &decl.instructions {
            let _ = writeln!(
                &mut out,
                "{:05}: {}",
                instr.index,
                self.write_instruction(instr)
            );
        }
        out
    }

    fn write_instruction(&self, instr: &InstructionNode) -> String {
        match instr.command.as_str() {
            "push_constant_int" => {
                if let OperandNode::Int(v) = instr.operand {
                    format!("push({v});")
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_long_constant" => {
                if let OperandNode::Long(v) = instr.operand {
                    format!("push({v}n);")
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_constant_string" => {
                if let OperandNode::String(s) = &instr.operand {
                    format!("push(\"{}\");", escape_string(s))
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_var" => {
                if let OperandNode::VarRef(vr) = &instr.operand {
                    if let Some(ref name) = vr.name {
                        if vr.is_transmog {
                            format!("push({name} as {});", vr.domain.type_str())
                        } else {
                            format!("push({name});")
                        }
                    } else {
                        format!(
                            "push(VARS.get({} * 1000000 + {})!);",
                            u64::from(vr.domain),
                            vr.id
                        )
                    }
                } else {
                    format!("VAR({});", self.write_operand_raw(&instr.operand))
                }
            }
            "pop_var" => {
                if let OperandNode::VarRef(vr) = &instr.operand {
                    if let Some(ref name) = vr.name {
                        if vr.is_transmog {
                            format!("{name} = pop() as {};", vr.domain.type_str())
                        } else {
                            format!("{name} = pop();")
                        }
                    } else {
                        format!(
                            "VARS.get({} * 1000000 + {}) = pop();",
                            u64::from(vr.domain),
                            vr.id
                        )
                    }
                } else {
                    format!("pop({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_varbit" | "pop_varbit" => {
                if let OperandNode::VarBitRef(vbr) = &instr.operand {
                    match &vbr.name {
                        Some(name) => format!("push({name});"),
                        None => format!("push(VARBITS.get({})!);", vbr.id),
                    }
                } else {
                    format!("VARBIT({});", self.write_operand_raw(&instr.operand))
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
                if let OperandNode::VarRef(vr) = &instr.operand {
                    if let Some(ref name) = vr.name {
                        format!("push({name});")
                    } else {
                        format!(
                            "push(VARS.get({} * 1000000 + {})!);",
                            u64::from(vr.domain),
                            vr.id
                        )
                    }
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "pop_varc_int" | "pop_varc_string" => {
                if let OperandNode::VarRef(vr) = &instr.operand {
                    if let Some(ref name) = vr.name {
                        format!("{name} = pop();")
                    } else {
                        format!(
                            "VARS.get({} * 1000000 + {}) = pop();",
                            u64::from(vr.domain),
                            vr.id
                        )
                    }
                } else {
                    format!("pop({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_varclanbit" | "push_varclansettingbit" => {
                if let OperandNode::VarBitRef(vbr) = &instr.operand {
                    match &vbr.name {
                        Some(name) => format!("push({name});"),
                        None => format!("push(VARBITS.get({})!);", vbr.id),
                    }
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_int_local" => {
                if let OperandNode::Local(idx) = instr.operand {
                    format!("push(local_int_{idx});")
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "pop_int_local" => {
                if let OperandNode::Local(idx) = instr.operand {
                    format!("local_int_{idx} = pop();")
                } else {
                    format!("pop({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_string_local" => {
                if let OperandNode::Local(idx) = instr.operand {
                    format!("push(local_obj_{idx});")
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "pop_string_local" => {
                if let OperandNode::Local(idx) = instr.operand {
                    format!("local_obj_{idx} = pop();")
                } else {
                    format!("pop({});", self.write_operand_raw(&instr.operand))
                }
            }
            "push_long_local" => {
                if let OperandNode::Local(idx) = instr.operand {
                    format!("push(local_long_{idx});")
                } else {
                    format!("push({});", self.write_operand_raw(&instr.operand))
                }
            }
            "pop_long_local" => {
                if let OperandNode::Local(idx) = instr.operand {
                    format!("local_long_{idx} = pop();")
                } else {
                    format!("pop({});", self.write_operand_raw(&instr.operand))
                }
            }
            "branch" => {
                if let OperandNode::Branch(target) = instr.operand {
                    format!("goto({target});")
                } else {
                    format!("goto({});", self.write_operand_raw(&instr.operand))
                }
            }
            "branch_not" => format!(
                "if (!pop()) goto({});",
                self.write_operand_raw(&instr.operand)
            ),
            "branch_equals" => format!(
                "if (pop() == pop()) goto({});",
                self.write_operand_raw(&instr.operand)
            ),
            "branch_if_true" => format!(
                "if (pop()) goto({});",
                self.write_operand_raw(&instr.operand)
            ),
            "branch_if_false" => format!(
                "if (!pop()) goto({});",
                self.write_operand_raw(&instr.operand)
            ),
            "gosub_with_params" => {
                if let OperandNode::Script(id) = instr.operand {
                    format!("{}(pop());", format_ident(&format!("script_{id}")))
                } else {
                    format!("call({});", self.write_operand_raw(&instr.operand))
                }
            }
            "switch" => {
                if let OperandNode::Switch(cases) = &instr.operand {
                    let arms: Vec<String> = cases
                        .iter()
                        .map(|c| format!("case {}: goto({});", c.value, c.target))
                        .collect();
                    format!(
                        "switch(pop()) {{\n        {}\n    }}",
                        arms.join("\n        ")
                    )
                } else {
                    format!("switch({});", self.write_operand_raw(&instr.operand))
                }
            }
            "join_string" => {
                if let OperandNode::Count(n) = instr.operand {
                    format!("push(pop().concat(...pop_multi({n})));")
                } else {
                    format!("concat({});", self.write_operand_raw(&instr.operand))
                }
            }
            "define_array" => {
                if let OperandNode::Array(id) = instr.operand {
                    format!("array_{id} = [];")
                } else {
                    format!("define_array({});", self.write_operand_raw(&instr.operand))
                }
            }
            "cc_create" => {
                if let OperandNode::Int(id) = instr.operand {
                    format!("UI.create({id});")
                } else {
                    format!("UI.create({});", self.write_operand_raw(&instr.operand))
                }
            }
            "cc_delete" => "UI.delete(pop() as number);".to_string(),
            "cc_settext" => "UI.setText(pop() as number, pop() as string);".to_string(),
            "cc_setgraphic" => "UI.setGraphic(pop() as number, pop() as number);".to_string(),
            "cc_sethide" => "UI.setHide(pop() as number, pop() as boolean);".to_string(),
            _ => format!(
                "{}({});",
                format_ident(&instr.command.replace('_', "")),
                self.write_operand_raw(&instr.operand)
            ),
        }
    }

    fn write_operand_raw(&self, operand: &OperandNode) -> String {
        match operand {
            OperandNode::Int(v) => v.to_string(),
            OperandNode::Long(v) => format!("{v}n"),
            OperandNode::String(s) => format!("\"{}\"", escape_string(s)),
            OperandNode::Local(idx) => format!("local_{idx}"),
            OperandNode::VarRef(v) => format!("{}:{}", v.domain.as_label(), v.id),
            OperandNode::VarBitRef(v) => format!("varbit:{}", v.id),
            OperandNode::Branch(target) => format!("->{target}"),
            OperandNode::Switch(cases) => {
                let arms: Vec<String> = cases
                    .iter()
                    .map(|c| format!("{}->{}", c.value, c.target))
                    .collect();
                format!("{{{}}}", arms.join(", "))
            }
            OperandNode::Script(id) => format!("script_{id}"),
            OperandNode::Array(id) => format!("array_{id}"),
            OperandNode::Count(n) => format!("count_{n}"),
            OperandNode::Byte(b) => b.to_string(),
        }
    }

    fn write_statement(&mut self, stmt: &Statement, out: &mut String) {
        match stmt {
            Statement::ExpressionStatement(es) => {
                self.write_expression(&es.expr, out);
                if es.semicolon {
                    out.push(';');
                }
                out.push('\n');
            }
            Statement::VariableDeclaration(vd) => {
                let _ = writeln!(
                    out,
                    "const {}: {} = {:?};",
                    vd.name,
                    vd.type_hint.as_str(),
                    vd.initializer
                );
            }
            Statement::IfStatement(if_) => {
                out.push_str("if (");
                self.write_expression(&if_.condition, out);
                out.push_str(") {\n");
                self.indent += 1;
                self.write_statement(&if_.then_branch, out);
                self.indent -= 1;
                out.push_str("}\n");
            }
            Statement::GotoStatement(gs) => {
                let _ = writeln!(out, "goto({});", gs.target);
            }
            Statement::SwitchStatement(sw) => {
                out.push_str("switch (");
                self.write_expression(&sw.discriminant, out);
                out.push_str(") {\n");
                for case_ in &sw.cases {
                    let _ = writeln!(out, "case {}: goto({});", case_.value, case_.target);
                }
                if let Some(default) = sw.default_target {
                    let _ = writeln!(out, "default: goto({default});");
                }
                out.push_str("}\n");
            }
            Statement::Label(l) => {
                let _ = writeln!(out, "// label {}", l.index);
            }
            Statement::CallStatement(cs) => {
                let _ = writeln!(
                    out,
                    "{}({});",
                    cs.callee,
                    self.format_arguments(&cs.arguments)
                );
            }
            Statement::ReturnStatement(rs) => {
                if let Some(ref val) = rs.value {
                    self.write_expression(val, out);
                    out.push_str(";\n");
                } else {
                    out.push_str("return;\n");
                }
            }
            Statement::Comment(c) => {
                let _ = writeln!(out, "// {c}");
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
            Expression::Identifier(id) => out.push_str(&format_ident(&id.name)),
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
            Expression::CallbackLiteral(callback) => {
                out.push_str("callback(\"");
                out.push_str(&escape_string(&callback.script));
                out.push_str("\", [");
                out.push_str(&callback.watchers.join(", "));
                out.push_str("])");
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

impl Default for Writer {
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

fn format_ident(name: &str) -> String {
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
