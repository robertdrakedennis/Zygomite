use super::ast::{
    BinaryOp, CallExpr, Expression, Identifier, InstructionNode, NumberLiteral, OperandNode,
    StringLiteral,
};

pub struct StackEffect {
    pub pops: usize,
    pub pushes: usize,
}

/// Classifies opcode commands by semantic prefix for dispatch in
/// catch-all arms. Specific opcodes are matched by exact name first.
enum OpcodeCategory {
    Push,
    Pop,
    Branch,
    CC,
    IF,
    Other,
}

fn categorize(cmd: &str) -> OpcodeCategory {
    if cmd.starts_with("push_") {
        OpcodeCategory::Push
    } else if cmd.starts_with("pop_") {
        OpcodeCategory::Pop
    } else if cmd.starts_with("branch_") || cmd.starts_with("long_branch_") {
        OpcodeCategory::Branch
    } else if cmd.starts_with("cc_") {
        OpcodeCategory::CC
    } else if cmd.starts_with("if_") {
        OpcodeCategory::IF
    } else {
        OpcodeCategory::Other
    }
}

fn stack_effect(cmd: &str, operand: &OperandNode) -> StackEffect {
    match cmd {
        // Push: pops 0, pushes 1
        "push_constant_int"
        | "push_long_constant"
        | "push_constant_string"
        | "push_var"
        | "push_varbit"
        | "pop_varbit"
        | "push_int_local"
        | "push_string_local"
        | "push_long_local"
        | "push_varc_int"
        | "push_varc_string"
        | "push_varclan"
        | "push_varclanbit"
        | "push_varclan_long"
        | "push_varclan_string"
        | "push_varclansetting"
        | "push_varclansettingbit"
        | "push_varclansetting_long"
        | "push_varclansetting_string" => StackEffect { pops: 0, pushes: 1 },

        // Array push: pop index, push value
        "push_array_int" | "push_array_string" => StackEffect { pops: 1, pushes: 1 },

        // Pop/discard: pops 1, pushes 0
        "pop_int_local" | "pop_string_local" | "pop_long_local" | "pop_var" | "pop_varc_int"
        | "pop_varc_string" | "pop_int_discard" | "pop_string_discard" | "pop_long_discard" => {
            StackEffect { pops: 1, pushes: 0 }
        }

        // Array pop: pop value, pop index, store
        "pop_array_int" | "pop_array_string" => StackEffect { pops: 2, pushes: 0 },

        // Binary arithmetic: pops 2, pushes 1
        "add" | "sub" | "multiply" | "divide" | "mod" => StackEffect { pops: 2, pushes: 1 },

        // Comparison/logical: pops 2, pushes 1 (result int)
        "compare" | "and" | "or" => StackEffect { pops: 2, pushes: 1 },

        // Unary: pops 1, pushes 1
        "lowercase" | "uppercase" | "length" | "neg" => StackEffect { pops: 1, pushes: 1 },

        // String join: pops N, pushes 1
        "join_string" => {
            if let OperandNode::Count(n) = operand {
                StackEffect {
                    pops: *n,
                    pushes: 1,
                }
            } else {
                StackEffect { pops: 0, pushes: 1 }
            }
        }

        // Array define: pops 0, pushes 0
        "define_array" => StackEffect { pops: 0, pushes: 0 },

        // Control flow
        "branch" => StackEffect { pops: 0, pushes: 0 },
        "branch_not" | "branch_if_true" | "branch_if_false" => StackEffect { pops: 1, pushes: 0 },
        "branch_equals"
        | "branch_less_than"
        | "branch_greater_than"
        | "branch_less_than_or_equals"
        | "branch_greater_than_or_equals"
        | "long_branch_equals"
        | "long_branch_less_than"
        | "long_branch_greater_than"
        | "long_branch_less_than_or_equals"
        | "long_branch_greater_than_or_equals" => StackEffect { pops: 2, pushes: 0 },
        "switch" => StackEffect { pops: 1, pushes: 0 },
        "return" => StackEffect { pops: 0, pushes: 0 },

        // Script call: pops 1 (computed expression), pushes 1 (result)
        "gosub_with_params" => StackEffect { pops: 1, pushes: 1 },

        // CC ops: various stack effects
        "cc_delete"
        | "cc_deleteall"
        | "cc_find"
        | "cc_sendtofront"
        | "cc_sendtoback"
        | "if_find"
        | "if_sendtofront"
        | "if_sendtoback"
        | "cc_setnoclickthrough"
        | "cc_setscrollpos"
        | "cc_set2dangle"
        | "cc_settiling" => StackEffect { pops: 1, pushes: 0 },
        "if_gettext" => StackEffect { pops: 1, pushes: 1 },
        "cc_settext" | "cc_setgraphic" | "cc_sethide" | "cc_setcolour" | "cc_setfill"
        | "cc_settrans" | "cc_setlinewid" | "cc_setmodel" | "cc_setaspect" | "cc_setposition"
        | "cc_setsize" => StackEffect { pops: 2, pushes: 0 },

        // Misc known ops
        "baseidkit" | "basecolour" | "setgender" | "setobj" | "cc_create" => {
            StackEffect { pops: 0, pushes: 0 }
        }

        _ => match categorize(cmd) {
            OpcodeCategory::Push => StackEffect { pops: 0, pushes: 1 },
            OpcodeCategory::Pop
            | OpcodeCategory::Branch
            | OpcodeCategory::CC
            | OpcodeCategory::IF => StackEffect { pops: 1, pushes: 0 },
            OpcodeCategory::Other => StackEffect { pops: 0, pushes: 0 },
        },
    }
}

#[derive(Debug, Clone)]
pub enum RecoveredStmt {
    Expression(Expression),
    Assignment {
        target: String,
        value: Expression,
        var_type: String,
    },
    Goto(usize),
    Branch {
        condition: Expression,
        target: usize,
        negated: bool,
    },
    BranchBinary {
        op: BinaryOp,
        left: Expression,
        right: Expression,
        target: usize,
    },
    Switch {
        discriminant: Expression,
        cases: Vec<(i32, usize)>,
    },
    Return(Option<Expression>),
    Comment(String),
}

pub struct ExprRecovery<'a, S: std::hash::BuildHasher = std::collections::hash_map::RandomState> {
    instructions: &'a [InstructionNode],
    stack: Vec<Expression>,
    locals: std::collections::HashMap<String, Expression>,
    /// Maps component IDs to their RS3 names (e.g. 5 → "`chat_box`").
    component_names: &'a std::collections::HashMap<u32, String, S>,
    /// Maps enum key values to qualified names (e.g. 0 → "`Enum_1234.ATTACK`").
    enum_value_names: &'a std::collections::HashMap<i32, String, S>,
    /// Maps script IDs to their parameter/return types for cross-script calls.
    script_signatures: &'a std::collections::HashMap<super::ScriptId, super::ScriptSignature, S>,
    /// Maps script IDs to decoded script names from cache metadata.
    script_names: &'a std::collections::HashMap<super::ScriptId, String, S>,
}

impl<'a, S: std::hash::BuildHasher> ExprRecovery<'a, S> {
    pub fn new(
        instructions: &'a [InstructionNode],
        component_names: &'a std::collections::HashMap<u32, String, S>,
        enum_value_names: &'a std::collections::HashMap<i32, String, S>,
        script_signatures: &'a std::collections::HashMap<
            super::ScriptId,
            super::ScriptSignature,
            S,
        >,
        script_names: &'a std::collections::HashMap<super::ScriptId, String, S>,
    ) -> Self {
        Self {
            instructions,
            stack: Vec::new(),
            locals: std::collections::HashMap::new(),
            component_names,
            enum_value_names,
            script_signatures,
            script_names,
        }
    }

    /// Process all instructions and return recovered statements.
    /// The result may have fewer entries than instructions (push/pop are
    /// folded into expressions).
    pub fn recover(mut self) -> Vec<Option<RecoveredStmt>> {
        let len = self.instructions.len();
        let mut stmts: Vec<Option<RecoveredStmt>> = vec![None; len];

        for (i, stmt_slot) in stmts.iter_mut().enumerate().take(len) {
            let instr = self.instructions[i].clone();
            let effect = stack_effect(&instr.command, &instr.operand);
            *stmt_slot = self.process_instruction(&instr, &effect);
        }

        stmts
    }

    // Process is a large match with many `if let` arms that need early
    // return to avoid nested else branches. Converting to expression-
    // based returns would obscure the opcode dispatch pattern.
    #[allow(clippy::needless_return)]
    fn process_instruction(
        &mut self,
        instr: &InstructionNode,
        effect: &StackEffect,
    ) -> Option<RecoveredStmt> {
        let cmd = instr.command.as_str();
        let op = &instr.operand;

        match cmd {
            // ── Push operations: build an expression and push onto stack ──
            "push_constant_int" => {
                if let OperandNode::Int(v) = op {
                    self.stack
                        .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                }
                None
            }
            "push_long_constant" => {
                if let OperandNode::Long(v) = op {
                    self.stack
                        .push(Expression::BigIntLiteral(super::ast::BigIntLiteral {
                            value: *v,
                        }));
                }
                None
            }
            "push_constant_string" => {
                if let OperandNode::String(s) = op {
                    self.stack.push(Expression::StringLiteral(StringLiteral {
                        value: s.clone(),
                    }));
                } else if let OperandNode::Int(v) = op {
                    // Only resolve non-negative values as enum keys.
                    // Negative values (e.g. -1) are sentinel/not-found markers.
                    if *v >= 0 {
                        if let Some(qualified) = self.enum_value_names.get(v) {
                            if let Some(dot) = qualified.find('.') {
                                let obj = &qualified[..dot];
                                let prop = &qualified[dot + 1..];
                                self.stack.push(Expression::PropertyAccess(
                                    super::ast::PropertyAccess {
                                        object: Box::new(Expression::Identifier(Identifier {
                                            name: obj.to_string(),
                                        })),
                                        property: prop.to_string(),
                                    },
                                ));
                            } else {
                                self.stack
                                    .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                            }
                        } else {
                            self.stack
                                .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                        }
                    } else {
                        self.stack
                            .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                    }
                }
                None
            }
            "push_var" => {
                if let OperandNode::VarRef(vr) = op {
                    let name = vr.name.clone().unwrap_or_else(|| {
                        format!("VARS.get({} * 1000000 + {})", u64::from(vr.domain), vr.id)
                    });
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "push_varbit" | "pop_varbit" => {
                if let OperandNode::VarBitRef(vbr) = op {
                    let name = vbr
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("VARBITS.get({})", vbr.id));
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "push_int_local" | "push_string_local" | "push_long_local" => {
                if let OperandNode::Local(idx) = op {
                    let (prefix, _) = local_type(cmd);
                    let name = format!("{prefix}_{idx}");
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "push_array_int" | "push_array_string" => {
                if let OperandNode::Array(id) = op {
                    let idx = self.pop_expr()?;
                    let arr = Expression::Identifier(Identifier {
                        name: format!("array_{id}"),
                    });
                    self.stack
                        .push(Expression::ArrayAccess(super::ast::ArrayAccess {
                            array: Box::new(arr),
                            index: Box::new(idx),
                        }));
                }
                None
            }
            // ── Pop operations: pop from stack, produce assignment or discard ──
            "pop_int_local" | "pop_string_local" | "pop_long_local" => {
                if let OperandNode::Local(idx) = op {
                    let value = self.pop_expr().unwrap_or_else(|| {
                        Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })
                    });
                    let (prefix, var_type) = local_type(cmd);
                    let name = format!("{prefix}_{idx}");
                    self.locals.insert(name.clone(), value.clone());
                    return Some(RecoveredStmt::Assignment {
                        target: name,
                        value,
                        var_type: var_type.to_string(),
                    });
                }
                None
            }
            "pop_var" => {
                if let OperandNode::VarRef(vr) = op {
                    let value = self.pop_or_unknown();
                    let name = vr.name.clone().unwrap_or_else(|| {
                        format!("VARS.get({} * 1000000 + {})", u64::from(vr.domain), vr.id)
                    });
                    return Some(RecoveredStmt::Assignment {
                        target: name,
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }
            "pop_int_discard" | "pop_string_discard" | "pop_long_discard" => {
                self.stack.pop();
                None
            }
            "pop_array_int" | "pop_array_string" => {
                if let OperandNode::Array(id) = op {
                    let value = self.stack.pop().unwrap_or_else(|| {
                        Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })
                    });
                    let idx = self
                        .pop_expr()
                        .unwrap_or(Expression::NumberLiteral(NumberLiteral { value: 0 }));
                    let idx_str = expr_str(&idx);
                    return Some(RecoveredStmt::Assignment {
                        target: format!("array_{id}[{idx_str}]"),
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }

            // ── Binary arithmetic: pop 2, build expression, push result ──
            "add" | "sub" | "multiply" | "divide" | "mod" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                let op = match cmd {
                    "add" => BinaryOp::Add,
                    "sub" => BinaryOp::Sub,
                    "multiply" => BinaryOp::Mul,
                    "divide" => BinaryOp::Div,
                    "mod" => BinaryOp::Mod,
                    _ => unreachable!(),
                };
                let expr = Expression::BinaryOperation(super::ast::BinaryOperation {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                });
                self.stack.push(expr);
                None
            }
            "compare" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "compare".to_string(),
                    })),
                    arguments: vec![left, right],
                });
                self.stack.push(expr);
                None
            }
            "and" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                self.stack
                    .push(Expression::BinaryOperation(super::ast::BinaryOperation {
                        op: BinaryOp::And,
                        left: Box::new(left),
                        right: Box::new(right),
                    }));
                None
            }
            "or" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                self.stack
                    .push(Expression::BinaryOperation(super::ast::BinaryOperation {
                        op: BinaryOp::Or,
                        left: Box::new(left),
                        right: Box::new(right),
                    }));
                None
            }

            // ── Unary ops ──
            "lowercase" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "lowercase".to_string(),
                    })),
                    arguments: vec![arg],
                });
                self.stack.push(expr);
                None
            }
            "uppercase" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "uppercase".to_string(),
                    })),
                    arguments: vec![arg],
                });
                self.stack.push(expr);
                None
            }
            "length" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::PropertyAccess(super::ast::PropertyAccess {
                    object: Box::new(arg),
                    property: "length".to_string(),
                });
                self.stack.push(expr);
                None
            }
            "neg" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::UnaryOperation(super::ast::UnaryOperation {
                    op: super::ast::UnaryOp::Neg,
                    operand: Box::new(arg),
                });
                self.stack.push(expr);
                None
            }

            // ── String join ──
            "join_string" => {
                if let OperandNode::Count(n) = op {
                    let mut parts: Vec<Expression> =
                        (0..*n).map(|_| self.pop_or_unknown()).collect();
                    parts.reverse();
                    let expr = Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "concat".to_string(),
                        })),
                        arguments: parts,
                    });
                    self.stack.push(expr);
                }
                None
            }

            // ── Control flow ──
            "branch" => {
                if let OperandNode::Branch(target) = op {
                    return Some(RecoveredStmt::Goto(*target));
                }
                None
            }
            "branch_not" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: true,
                    });
                }
                None
            }
            "branch_if_true" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: false,
                    });
                }
                None
            }
            "branch_if_false" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: true,
                    });
                }
                None
            }
            "branch_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Eq,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_less_than" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Lt,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_greater_than" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Gt,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_less_than_or_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Le,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_greater_than_or_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Ge,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "switch" => {
                if let OperandNode::Switch(cases) = op {
                    let discriminant = self.pop_or_unknown();
                    let case_pairs: Vec<(i32, usize)> =
                        cases.iter().map(|c| (c.value, c.target)).collect();
                    return Some(RecoveredStmt::Switch {
                        discriminant,
                        cases: case_pairs,
                    });
                }
                None
            }
            "return" => {
                let val = self.stack.pop();
                return Some(RecoveredStmt::Return(val));
            }

            // ── Script call ──
            "gosub_with_params" => {
                if let OperandNode::Script(id) = op {
                    let sid = super::ScriptId(*id);
                    let total_args = self
                        .script_signatures
                        .get(&sid)
                        .map(super::ScriptSignature::total_args)
                        .unwrap_or(1);
                    let mut args: Vec<Expression> =
                        (0..total_args).map(|_| self.pop_or_unknown()).collect();
                    args.reverse();
                    let callee_name = self
                        .script_names
                        .get(&sid)
                        .map(|name| {
                            super::sanitize_export_name(&super::extract_script_name_suffix(name))
                        })
                        .unwrap_or_else(|| format!("script_{id}"));
                    let expr = Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier { name: callee_name })),
                        arguments: args,
                    });
                    self.stack.push(expr);
                }
                None
            }

            "db_getrowtable" | "db_find" | "db_find_with_count" => {
                if let OperandNode::Int(table_id) = op {
                    let expr = Expression::Call(CallExpr {
                        callee: Box::new(Expression::PropertyAccess(super::ast::PropertyAccess {
                            object: Box::new(Expression::Identifier(Identifier {
                                name: "DB_TABLES".to_string(),
                            })),
                            property: "get".to_string(),
                        })),
                        arguments: vec![Expression::NumberLiteral(NumberLiteral {
                            value: *table_id,
                        })],
                    });
                    self.stack.push(expr);
                }
                None
            }

            // ── CC / UI ops ──
            "cc_create" => {
                if let OperandNode::Int(id) = op {
                    return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "UI.create".to_string(),
                        })),
                        arguments: vec![self.component_ref(*id as u32)],
                    })));
                }
                None
            }
            "cc_delete" => {
                let arg = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.delete".to_string(),
                    })),
                    arguments: vec![arg],
                })));
            }
            "if_gettext" => {
                let id = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.getText".to_string(),
                    })),
                    arguments: vec![id],
                });
                self.stack.push(expr);
                None
            }
            "cc_settext" => {
                let text = self.pop_or_unknown();
                let id = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.setText".to_string(),
                    })),
                    arguments: vec![id, text],
                })));
            }
            "cc_setgraphic" => {
                let graphic = self.pop_or_unknown();
                let id = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.setGraphic".to_string(),
                    })),
                    arguments: vec![id, graphic],
                })));
            }
            "cc_sethide" => {
                let hidden = self.pop_or_unknown();
                let id = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.setHide".to_string(),
                    })),
                    arguments: vec![id, hidden],
                })));
            }
            "cc_setcolour" => {
                let colour = self.pop_or_unknown();
                let id = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.setColour".to_string(),
                    })),
                    arguments: vec![id, colour],
                })));
            }
            "cc_setsize" => {
                let size = self.pop_or_unknown();
                let id = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.setSize".to_string(),
                    })),
                    arguments: vec![id, size],
                })));
            }
            _ => {
                match categorize(cmd) {
                    OpcodeCategory::CC | OpcodeCategory::IF => {
                        let mut args: Vec<Expression> =
                            (0..effect.pops).map(|_| self.pop_or_unknown()).collect();
                        args.reverse();
                        return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: format!("UI.{}", sanitize_camel(&cmd[3..])),
                            })),
                            arguments: args,
                        })));
                    }
                    OpcodeCategory::Push => {
                        self.stack.push(operand_expr(op));
                    }
                    OpcodeCategory::Pop => {
                        let _val = self.stack.pop();
                    }
                    OpcodeCategory::Branch | OpcodeCategory::Other => {
                        // define_array is a known op that falls through to Other
                        if let OperandNode::Array(id) = op
                            && cmd == "define_array"
                        {
                            return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier {
                                    name: format!("define_array_{id}"),
                                })),
                                arguments: vec![],
                            })));
                        }
                        // Unknown: emit as call if it consumes or produces stack values
                        if effect.pops > 0 {
                            let mut args: Vec<Expression> =
                                (0..effect.pops).map(|_| self.pop_or_unknown()).collect();
                            args.reverse();
                            if effect.pushes > 0 {
                                let expr = Expression::Call(CallExpr {
                                    callee: Box::new(Expression::Identifier(Identifier {
                                        name: sanitize_command(cmd),
                                    })),
                                    arguments: args,
                                });
                                self.stack.push(expr);
                            } else {
                                return Some(RecoveredStmt::Expression(Expression::Call(
                                    CallExpr {
                                        callee: Box::new(Expression::Identifier(Identifier {
                                            name: sanitize_command(cmd),
                                        })),
                                        arguments: args,
                                    },
                                )));
                            }
                        } else if effect.pushes > 0 {
                            self.stack.push(Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier {
                                    name: sanitize_command(cmd),
                                })),
                                arguments: vec![],
                            }));
                        }
                    }
                }
                None
            }
        }
    }

    fn pop_expr(&mut self) -> Option<Expression> {
        self.stack.pop()
    }

    fn pop_or_unknown(&mut self) -> Expression {
        self.stack.pop().unwrap_or_else(|| {
            Expression::Call(CallExpr {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "pop".to_string(),
                })),
                arguments: vec![],
            })
        })
    }

    /// Converts a component ID to a TypeScript expression.
    /// If the component has a known name, emits `ComponentId.name`;
    /// otherwise emits the raw number.
    fn component_ref(&self, id: u32) -> Expression {
        if let Some(name) = self.component_names.get(&id) {
            Expression::PropertyAccess(super::ast::PropertyAccess {
                object: Box::new(Expression::Identifier(Identifier {
                    name: "ComponentId".to_string(),
                })),
                property: super::sanitize_ts_ident(name),
            })
        } else {
            Expression::NumberLiteral(NumberLiteral { value: id as i32 })
        }
    }
}

fn local_type(cmd: &str) -> (&'static str, &'static str) {
    if cmd.contains("long") {
        ("local_long", "bigint")
    } else if cmd.contains("string") || cmd.contains("obj") {
        ("local_obj", "string")
    } else {
        ("local_int", "number")
    }
}

fn operand_expr(op: &OperandNode) -> Expression {
    match op {
        OperandNode::Int(v) => Expression::NumberLiteral(NumberLiteral { value: *v }),
        OperandNode::Long(v) => Expression::BigIntLiteral(super::ast::BigIntLiteral { value: *v }),
        OperandNode::String(s) => Expression::StringLiteral(StringLiteral { value: s.clone() }),
        OperandNode::Local(idx) => Expression::Identifier(Identifier {
            name: format!("local_{idx}"),
        }),
        OperandNode::VarRef(vr) => {
            let name = vr
                .name
                .clone()
                .unwrap_or_else(|| format!("var_{}:{}", vr.domain.as_label(), vr.id));
            Expression::Identifier(Identifier { name })
        }
        OperandNode::VarBitRef(vbr) => {
            let name = vbr
                .name
                .clone()
                .unwrap_or_else(|| format!("varbit_{}", vbr.id));
            Expression::Identifier(Identifier { name })
        }
        OperandNode::Array(id) => {
            let arr = Expression::Identifier(Identifier {
                name: format!("array_{id}"),
            });
            Expression::ArrayAccess(super::ast::ArrayAccess {
                array: Box::new(arr),
                index: Box::new(Expression::Identifier(Identifier {
                    name: "idx".to_string(),
                })),
            })
        }
        _ => Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "pop".to_string(),
            })),
            arguments: vec![],
        }),
    }
}

fn sanitize_command(cmd: &str) -> String {
    super::sanitize_ts_ident(&cmd.replace('_', ""))
}

fn sanitize_camel(s: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for c in s.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            out.push(c.to_ascii_uppercase());
            capitalize = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn expr_str(expr: &Expression) -> String {
    match expr {
        Expression::NumberLiteral(n) => n.value.to_string(),
        Expression::Identifier(id) => id.name.clone(),
        Expression::StringLiteral(s) => format!("\"{}\"", s.value),
        _ => "expr".to_string(),
    }
}
