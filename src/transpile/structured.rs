use super::ast::{
    ArgumentVariable, BinaryOp, Expression, ImportStatement, LocalVariable, ScriptId,
    TypeAnnotation, UnaryOp,
};
use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub enum AssignmentTarget {
    Identifier(String),
    ArrayAccess { array: String, index: Expression },
    Opaque(String),
}

#[derive(Debug, Clone)]
pub enum StructuredStmt {
    While {
        body: Vec<Self>,
    },
    If {
        condition: Expression,
        then_body: Vec<Self>,
        else_body: Option<Vec<Self>>,
    },
    Switch {
        expr: Expression,
        cases: Vec<SwitchCaseStmt>,
    },
    Assignment {
        target: AssignmentTarget,
        value: Expression,
    },
    Expr {
        expr: Expression,
    },
    Goto {
        target: usize,
    },
    /// A jump target for `goto`, at the instruction index `target` (a block
    /// start). Emitted only by the linear fallback for irreducible control flow;
    /// lowers to a label, not an instruction.
    Label {
        target: usize,
    },
    Return {
        value: Option<Expression>,
    },
    Comment(String),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub struct SwitchCaseStmt {
    pub value: i32,
    pub body: Vec<StructuredStmt>,
}

/// Whether a statement unconditionally leaves its block — it returns, breaks,
/// continues, gotos, or is an `if` whose arms all do.
pub fn stmt_terminates(stmt: &StructuredStmt) -> bool {
    match stmt {
        StructuredStmt::Return { .. }
        | StructuredStmt::Break
        | StructuredStmt::Continue
        | StructuredStmt::Goto { .. } => true,
        StructuredStmt::If {
            then_body,
            else_body: Some(else_body),
            ..
        } => stmts_terminate(then_body) && stmts_terminate(else_body),
        _ => false,
    }
}

/// Whether a statement sequence unconditionally leaves its block (its last
/// statement does).
pub fn stmts_terminate(stmts: &[StructuredStmt]) -> bool {
    stmts.last().is_some_and(stmt_terminates)
}

#[derive(Debug, Clone)]
pub struct StructuredScript {
    pub script_id: ScriptId,
    pub raw_name: Option<String>,
    pub header_comments: Vec<String>,
    pub imports: Vec<ImportStatement>,
    pub function_name: String,
    pub arguments: Vec<ArgumentVariable>,
    pub locals: Vec<LocalVariable>,
    pub arrays: Vec<u32>,
    pub return_type: String,
    pub body: Vec<StructuredStmt>,
}

impl StructuredScript {
    pub fn render(&self) -> String {
        self.render_with_options(false)
    }

    pub fn canonical_source(&self) -> String {
        self.render_with_options(true)
    }

    fn render_with_options(&self, canonical: bool) -> String {
        let mut out = String::new();
        if !canonical {
            for comment in &self.header_comments {
                let _ = writeln!(&mut out, "// {comment}");
            }
        }
        for import in &self.imports {
            write_import(&mut out, import);
        }
        if !self.imports.is_empty() {
            out.push('\n');
        }

        let args = self
            .arguments
            .iter()
            .map(|a| format!("{}: {}", a.name, a.type_annotation.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            &mut out,
            "export function {}({args}): {} {{",
            self.function_name, self.return_type
        );
        for local in &self.locals {
            let _ = writeln!(
                &mut out,
                "    let {}: {};",
                local.name,
                local.type_annotation.as_str()
            );
        }
        if !self.locals.is_empty() {
            out.push('\n');
        }
        for array_id in &self.arrays {
            let _ = writeln!(&mut out, "    let array_{array_id}: number[] = [];");
        }
        if !self.arrays.is_empty() {
            out.push('\n');
        }
        let mut renderer = StructuredRenderer {
            indent: 1,
            canonical,
        };
        renderer.write_stmts(&self.body, &mut out);
        out.push_str("}\n");
        out
    }
}

fn write_import(out: &mut String, import: &ImportStatement) {
    let keyword = if import.is_type_only {
        "import type"
    } else {
        "import"
    };
    let names = import.named_exports.join(", ");
    let _ = writeln!(out, "{keyword} {{ {names} }} from '{}';", import.module);
}

struct StructuredRenderer {
    indent: usize,
    canonical: bool,
}

impl StructuredRenderer {
    fn write_stmts(&mut self, stmts: &[StructuredStmt], out: &mut String) {
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
                self.write_stmts(body, out);
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
                out.push_str(&format_expression(condition));
                out.push_str(") {\n");
                self.indent += 1;
                self.write_stmts(then_body, out);
                self.indent -= 1;
                if let Some(else_body) = else_body {
                    self.write_indent(out);
                    out.push_str("} else {\n");
                    self.indent += 1;
                    self.write_stmts(else_body, out);
                    self.indent -= 1;
                }
                self.write_indent(out);
                out.push_str("}\n");
            }
            StructuredStmt::Switch { expr, cases } => {
                self.write_indent(out);
                out.push_str("switch (");
                out.push_str(&format_expression(expr));
                out.push_str(") {\n");
                self.indent += 1;
                for case_ in cases {
                    self.write_indent(out);
                    let _ = writeln!(out, "case {}:", case_.value);
                    self.indent += 1;
                    self.write_stmts(&case_.body, out);
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
                out.push_str(&format_assignment_target(target));
                out.push_str(" = ");
                out.push_str(&format_expression(value));
                out.push_str(";\n");
            }
            StructuredStmt::Expr { expr } => {
                self.write_indent(out);
                out.push_str(&format_expression(expr));
                out.push_str(";\n");
            }
            StructuredStmt::Goto { target } => {
                self.write_indent(out);
                let _ = writeln!(out, "goto({target});");
            }
            StructuredStmt::Label { target } => {
                self.write_indent(out);
                let _ = writeln!(out, "label({target});");
            }
            StructuredStmt::Return { value } => {
                self.write_indent(out);
                out.push_str("return");
                if let Some(value) = value {
                    out.push(' ');
                    out.push_str(&format_expression(value));
                }
                out.push_str(";\n");
            }
            StructuredStmt::Comment(text) => {
                if !self.canonical {
                    self.write_indent(out);
                    let _ = writeln!(out, "// {text}");
                }
            }
            StructuredStmt::Break => {
                self.write_indent(out);
                out.push_str("break;\n");
            }
            StructuredStmt::Continue => {
                self.write_indent(out);
                out.push_str("continue;\n");
            }
        }
    }

    fn write_indent(&self, out: &mut String) {
        for _ in 0..self.indent {
            out.push_str("    ");
        }
    }
}

pub fn format_assignment_target(target: &AssignmentTarget) -> String {
    match target {
        AssignmentTarget::Identifier(name) | AssignmentTarget::Opaque(name) => name.clone(),
        AssignmentTarget::ArrayAccess { array, index } => {
            format!("{array}[{}]", format_expression(index))
        }
    }
}

pub fn format_expression(expr: &Expression) -> String {
    match expr {
        Expression::NumberLiteral(value) => value.value.to_string(),
        Expression::BigIntLiteral(value) => format!("{}n", value.value),
        Expression::StringLiteral(value) => format!("\"{}\"", escape_string(&value.value)),
        Expression::BooleanLiteral(value) => value.value.to_string(),
        Expression::Identifier(identifier) => identifier.name.clone(),
        Expression::ArrayAccess(access) => format!(
            "{}[{}]",
            format_wrapped_expression(&access.array),
            format_expression(&access.index)
        ),
        Expression::PropertyAccess(access) => format!(
            "{}.{}",
            format_wrapped_expression(&access.object),
            access.property
        ),
        Expression::Call(call) => format!(
            "{}({})",
            format_wrapped_expression(&call.callee),
            call.arguments
                .iter()
                .map(format_expression)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::CallbackLiteral(callback) => format!(
            "callback(\"{}\", [{}], [{}], \"{}\")",
            escape_string(&callback.script),
            callback
                .arguments
                .iter()
                .map(format_expression)
                .collect::<Vec<_>>()
                .join(", "),
            callback.watchers.join(", "),
            escape_string(&callback.raw_descriptor)
        ),
        Expression::BinaryOperation(binary) => format!(
            "({} {} {})",
            format_expression(&binary.left),
            format_binary_op(binary.op),
            format_expression(&binary.right)
        ),
        Expression::UnaryOperation(unary) => {
            format!(
                "({}{})",
                format_unary_op(unary.op),
                format_expression(&unary.operand)
            )
        }
        Expression::PushOperation(push) => format!("push({})", format_expression(&push.value)),
        Expression::PopOperation(pop) => pop.target.as_ref().map_or_else(
            || "pop()".to_string(),
            |target| format!("pop({})", format_expression(target)),
        ),
        Expression::GotoExpr(goto) => format!("goto({})", goto.target),
    }
}

fn format_wrapped_expression(expr: &Expression) -> String {
    match expr {
        Expression::Identifier(_)
        | Expression::NumberLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::Call(_)
        | Expression::PropertyAccess(_)
        | Expression::ArrayAccess(_) => format_expression(expr),
        _ => format!("({})", format_expression(expr)),
    }
}

fn format_binary_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
    }
}

fn format_unary_op(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "-",
        UnaryOp::Not => "!",
    }
}

fn escape_string(value: &str) -> String {
    // Escape every char that would break a double-quoted TS string literal on
    // re-parse (oxc) during recompile. A CS2 string constant containing a raw
    // CR previously emitted a literal CR inside the quotes → unterminated-string
    // parse error in assemble-script.
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn parse_type_annotation(value: &str) -> TypeAnnotation {
    match value.trim() {
        "number" => TypeAnnotation::Number,
        "bigint" => TypeAnnotation::BigInt,
        "string" => TypeAnnotation::String,
        "boolean" => TypeAnnotation::Boolean,
        _ => TypeAnnotation::Unknown,
    }
}
