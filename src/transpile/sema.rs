use super::ast::{
    ArgumentVariable, Declaration, Expression, InstructionNode, LocalVariable, OperandNode,
    Program, Statement, TypeAnnotation,
};
use super::diagnostics::Diagnostics;
use super::scope::{LocalType, Scopes, Symbol, SymbolKind};
use crate::vars::VarDomain;

pub struct Sema {
    pub diagnostics: Diagnostics,
    scopes: Scopes,
}

impl Sema {
    pub fn new() -> Self {
        Self {
            diagnostics: Diagnostics::new(),
            scopes: Scopes::new(),
        }
    }

    pub fn analyze_program(&mut self, program: &mut Program) {
        self.scopes.push_scope();
        for stmt in &mut program.statements {
            self.analyze_statement(stmt);
        }
        self.scopes.pop_scope();
    }

    pub fn analyze_declaration(&mut self, decl: &mut Declaration) {
        self.scopes.push_scope();
        self.declare_arguments(&decl.arguments);
        self.declare_locals(&decl.locals);
        for instr in &mut decl.instructions {
            self.analyze_instruction(instr);
        }
        self.scopes.pop_scope();
    }

    fn declare_arguments(&mut self, args: &[ArgumentVariable]) {
        for arg in args {
            let type_ = match arg.type_annotation {
                TypeAnnotation::Number => LocalType::Int,
                TypeAnnotation::BigInt => LocalType::Long,
                TypeAnnotation::String => LocalType::Object,
                TypeAnnotation::Unknown | TypeAnnotation::Boolean => LocalType::Int,
            };
            let sym = Symbol::new(
                arg.name.clone(),
                SymbolKind::Argument {
                    index: arg.index,
                    type_,
                },
            );
            self.scopes.define(sym);
        }
    }

    fn declare_locals(&mut self, locals: &[LocalVariable]) {
        for local in locals {
            let type_ = match local.type_annotation {
                TypeAnnotation::Number => LocalType::Int,
                TypeAnnotation::BigInt => LocalType::Long,
                TypeAnnotation::String => LocalType::Object,
                TypeAnnotation::Unknown | TypeAnnotation::Boolean => LocalType::Int,
            };
            let sym = Symbol::new(
                local.name.clone(),
                SymbolKind::Local {
                    index: local.index,
                    type_,
                },
            );
            self.scopes.define(sym);
        }
    }

    fn analyze_statement(&mut self, stmt: &mut Statement) {
        match stmt {
            Statement::ExpressionStatement(es) => {
                self.analyze_expression(&mut es.expr);
            }
            Statement::VariableDeclaration(vd) => {
                if let Some(ref mut init) = vd.initializer {
                    self.analyze_expression(init);
                }
            }
            Statement::IfStatement(if_) => {
                self.analyze_expression(&mut if_.condition);
                self.analyze_statement(&mut if_.then_branch);
                if let Some(ref mut else_) = if_.else_branch {
                    self.analyze_statement(else_);
                }
            }
            Statement::GotoStatement(_) => {}
            Statement::SwitchStatement(sw) => {
                self.analyze_expression(&mut sw.discriminant);
            }
            Statement::Label(_) => {}
            Statement::CallStatement(cs) => {
                for arg in &mut cs.arguments {
                    self.analyze_expression(arg);
                }
            }
            Statement::ReturnStatement(rs) => {
                if let Some(ref mut val) = rs.value {
                    self.analyze_expression(val);
                }
            }
            Statement::Comment(_) => {}
        }
    }

    fn analyze_instruction(&mut self, instr: &mut InstructionNode) {
        match &mut instr.operand {
            OperandNode::Int(_)
            | OperandNode::Long(_)
            | OperandNode::String(_)
            | OperandNode::Byte(_)
            | OperandNode::Array(_)
            | OperandNode::Count(_) => {}
            OperandNode::Local(idx) => {
                let name = format!("local_{idx}");
                if self.scopes.lookup(&name).is_none() {
                    self.diagnostics.warning(format!("undefined local: {name}"));
                }
            }
            OperandNode::VarRef(vr) => {
                if vr.name.is_none() {
                    self.diagnostics.note(format!(
                        "unresolved var {}:{}",
                        vr.domain.as_label(),
                        vr.id
                    ));
                }
            }
            OperandNode::VarBitRef(vbr) => {
                if vbr.name.is_none() {
                    self.diagnostics
                        .note(format!("unresolved varbit {}", vbr.id));
                }
            }
            OperandNode::Branch(_) | OperandNode::Switch(_) | OperandNode::Script(_) => {}
        }
    }

    fn analyze_expression(&mut self, expr: &mut Expression) {
        match expr {
            Expression::NumberLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_) => {}
            Expression::Identifier(id) => {
                if self.scopes.lookup(&id.name).is_none() {
                    self.diagnostics
                        .warning(format!("undefined identifier: {}", id.name));
                }
            }
            Expression::ArrayAccess(aa) => {
                self.analyze_expression(&mut aa.array);
                self.analyze_expression(&mut aa.index);
            }
            Expression::PropertyAccess(pa) => {
                self.analyze_expression(&mut pa.object);
            }
            Expression::Call(c) => {
                self.analyze_expression(&mut c.callee);
                for arg in &mut c.arguments {
                    self.analyze_expression(arg);
                }
            }
            Expression::CallbackLiteral(_) => {}
            Expression::BinaryOperation(bin) => {
                self.analyze_expression(&mut bin.left);
                self.analyze_expression(&mut bin.right);
            }
            Expression::UnaryOperation(un) => {
                self.analyze_expression(&mut un.operand);
            }
            Expression::PushOperation(push) => {
                self.analyze_expression(&mut push.value);
            }
            Expression::PopOperation(_) => {}
            Expression::GotoExpr(_) => {}
        }
    }

    pub fn finish(mut self) -> Diagnostics {
        std::mem::take(&mut self.diagnostics)
    }
}

impl Default for Sema {
    fn default() -> Self {
        Self::new()
    }
}

pub fn var_domain_to_type(domain: VarDomain) -> TypeAnnotation {
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
        | VarDomain::Global => TypeAnnotation::Number,
    }
}
