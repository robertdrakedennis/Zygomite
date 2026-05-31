use super::ast::{BinaryOp, CallbackLiteral, Expression, ScriptId, TypeAnnotation, UnaryOp};
use super::reversible_format::ReversibleMetadata;
use super::structured::{
    AssignmentTarget, StructuredScript, StructuredStmt, parse_type_annotation, stmts_terminate,
};
use super::{ScriptCatalog, ScriptSignature};
use crate::cache_bail as bail;
use crate::error::Result;
use crate::script::{CompiledScript, Instruction, Operand, VarBitRef, VarRef};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ReverseCompileContext {
    pub build: u32,
    pub script_catalog: ScriptCatalog,
    pub script_signatures: HashMap<ScriptId, ScriptSignature>,
    pub var_refs_by_name: HashMap<String, VarRef>,
    pub varbit_refs_by_name: HashMap<String, VarBitRef>,
    pub string_param_ids: HashSet<i32>,
    pub enum_values_by_name: HashMap<String, i32>,
    pub component_ids_by_name: HashMap<String, i32>,
    pub opcode_commands: HashSet<String>,
}

impl ReverseCompileContext {
    pub fn has_command(&self, command: &str) -> bool {
        self.opcode_commands.contains(command)
    }

    /// Invert the decompiler's generic command rendering. A CS2 command with no
    /// dedicated lowering is decompiled as `sanitize_command(cmd)(args)`, which
    /// strips underscores and TS-sanitizes — lossy — so recover the opcode by
    /// matching that transform against every command name. Deterministic across
    /// runs (`HashSet` order is not): ties break by shortest then lexicographic.
    pub fn resolve_command(&self, sanitized: &str) -> Option<&str> {
        let mut best: Option<&str> = None;
        for cmd in &self.opcode_commands {
            if super::sanitize_ts_ident(&cmd.replace('_', "")) != sanitized {
                continue;
            }
            best = Some(match best {
                Some(cur) if (cur.len(), cur) <= (cmd.len(), cmd.as_str()) => cur,
                _ => cmd.as_str(),
            });
        }
        best
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Int,
    Long,
    Object,
    Unknown,
    Void,
}

/// Result kind of a generic CS2 command, from the client-extracted stack-effect
/// table. A void command (or one absent from the table) is `Void`; a multi-value
/// push is treated as `Void` too since the recovery models it as a single value
/// and the gate will block any genuine value use it can't represent.
fn command_result_kind(
    command: &str,
    arguments: &[Expression],
    ctx: &ReverseCompileContext,
) -> ValueKind {
    if let Some(param_index) = param_result_command_param_index(command) {
        let param_id = arguments
            .get(param_index)
            .and_then(|arg| literal_i32(arg).ok());
        return if param_id.is_some_and(|id| ctx.string_param_ids.contains(&id)) {
            ValueKind::Object
        } else {
            ValueKind::Int
        };
    }

    use super::expr_recovery::PushedType;
    match super::expr_recovery::opcode_stack_effect(command).map(|e| e.pushed_type()) {
        Some(PushedType::Int) => ValueKind::Int,
        Some(PushedType::Obj) => ValueKind::Object,
        Some(PushedType::Long) => ValueKind::Long,
        _ => ValueKind::Void,
    }
}

fn param_result_command_param_index(command: &str) -> Option<usize> {
    match command {
        "cc_param" => Some(0),
        "inv_totalparam"
        | "inv_totalparam_stack"
        | "lc_param"
        | "mec_param"
        | "nc_param"
        | "oc_param"
        | "quest_param"
        | "seq_param"
        | "struct_param" => Some(1),
        _ => None,
    }
}

/// Lowering label for a `goto`/`label` target (an instruction-start index).
fn block_label(target: usize) -> String {
    format!("block_{target}")
}

pub fn lower_structured_script(
    script: &StructuredScript,
    metadata: &ReversibleMetadata,
    ctx: &ReverseCompileContext,
) -> Result<CompiledScript> {
    let mut lowerer = StructuredLowerer::new(script, metadata, ctx);
    lowerer.lower()?;
    lowerer.finish()
}

struct StructuredLowerer<'a> {
    script: &'a StructuredScript,
    metadata: &'a ReversibleMetadata,
    ctx: &'a ReverseCompileContext,
    instructions: Vec<Instruction>,
    labels: HashMap<String, usize>,
    branch_fixups: Vec<(usize, String)>,
    switch_fixups: Vec<(usize, Vec<(i32, String)>)>,
    next_label: usize,
    loop_labels: Vec<LoopLabels>,
    local_types: HashMap<String, ValueKind>,
}

#[derive(Debug, Clone)]
struct LoopLabels {
    continue_label: String,
    break_label: String,
}

impl<'a> StructuredLowerer<'a> {
    fn new(
        script: &'a StructuredScript,
        metadata: &'a ReversibleMetadata,
        ctx: &'a ReverseCompileContext,
    ) -> Self {
        let mut local_types = HashMap::new();
        for argument in &script.arguments {
            local_types.insert(
                argument.name.clone(),
                value_kind_for_type(argument.type_annotation),
            );
        }
        for local in &script.locals {
            local_types.insert(
                local.name.clone(),
                value_kind_for_type(local.type_annotation),
            );
        }
        Self {
            script,
            metadata,
            ctx,
            instructions: Vec::new(),
            labels: HashMap::new(),
            branch_fixups: Vec::new(),
            switch_fixups: Vec::new(),
            next_label: 0,
            loop_labels: Vec::new(),
            local_types,
        }
    }

    fn lower(&mut self) -> Result<()> {
        self.lower_stmts(&self.script.body)
    }

    fn finish(mut self) -> Result<CompiledScript> {
        for (index, label) in &self.branch_fixups {
            let Some(target) = self.labels.get(label).copied() else {
                bail!("missing branch label {label}");
            };
            self.instructions[*index].operand = Operand::Branch(target as i32);
        }
        for (index, cases) in &self.switch_fixups {
            let resolved = cases
                .iter()
                .map(|(value, label)| {
                    let Some(target) = self.labels.get(label).copied() else {
                        bail!("missing switch label {label}");
                    };
                    Ok(crate::script::SwitchCase {
                        value: *value,
                        target: target as i32,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            self.instructions[*index].operand = Operand::Switch(resolved);
        }

        // Reject any emitted command absent from the target build's opcode table
        // before it reaches the encoder. This catches lowering that names an
        // opcode that does not exist for this build (e.g. `sub` on 910, which
        // has no subtraction opcode) with a source-attributed error instead of
        // a bare "missing opcode mapping" at encode time, and makes
        // `--strict-structured` a real guarantee.
        for (index, instruction) in self.instructions.iter().enumerate() {
            if !self.ctx.has_command(&instruction.command) {
                bail!(
                    "instruction {index} uses command `{}`, which is not available in build {}",
                    instruction.command,
                    self.ctx.build
                );
            }
        }

        Ok(CompiledScript {
            name: self.metadata.raw_name.clone(),
            local_count_int: count_locals(&self.script.locals, TypeAnnotation::Number),
            local_count_object: count_locals(&self.script.locals, TypeAnnotation::String),
            local_count_long: count_locals(&self.script.locals, TypeAnnotation::BigInt),
            argument_count_int: count_args(&self.script.arguments, TypeAnnotation::Number),
            argument_count_object: count_args(&self.script.arguments, TypeAnnotation::String),
            argument_count_long: count_args(&self.script.arguments, TypeAnnotation::BigInt),
            code: self.instructions,
        })
    }

    fn lower_stmts(&mut self, stmts: &[StructuredStmt]) -> Result<()> {
        let mut index = 0;
        while index < stmts.len() {
            if let Some(consumed) = self.try_lower_multi_result_assignment_group(stmts, index)? {
                index += consumed;
                continue;
            }
            self.lower_stmt(&stmts[index])?;
            index += 1;
        }
        Ok(())
    }

    fn try_lower_multi_result_assignment_group(
        &mut self,
        stmts: &[StructuredStmt],
        start: usize,
    ) -> Result<Option<usize>> {
        let Some((first_target, first_value)) = assignment_parts(&stmts[start]) else {
            return Ok(None);
        };
        let Some(access) = MultiResultAccess::from_expr(first_value, self.ctx) else {
            return Ok(None);
        };
        let Some(pop_order) = multi_result_pop_order(&access.command) else {
            return Ok(None);
        };
        if pop_order.first() != Some(&access.field) {
            return Ok(None);
        }

        let mut targets = vec![first_target];
        for expected in pop_order.iter().skip(1) {
            let Some(stmt) = stmts.get(start + targets.len()) else {
                return Ok(None);
            };
            let Some((target, value)) = assignment_parts(stmt) else {
                return Ok(None);
            };
            let Some(next_access) = MultiResultAccess::from_expr(value, self.ctx) else {
                return Ok(None);
            };
            if next_access.command != access.command
                || next_access.field != *expected
                || format!("{:?}", next_access.arguments) != format!("{:?}", access.arguments)
            {
                return Ok(None);
            }
            targets.push(target);
        }

        for argument in &access.arguments {
            self.emit_expr(argument)?;
        }
        self.emit_instruction(&access.command, Operand::Byte(0));
        for target in targets {
            self.emit_store_assignment(target)?;
        }
        Ok(Some(pop_order.len()))
    }

    fn lower_stmt(&mut self, stmt: &StructuredStmt) -> Result<()> {
        match stmt {
            StructuredStmt::While { body } => self.lower_while(body),
            StructuredStmt::If {
                condition,
                then_body,
                else_body,
            } => self.lower_if(condition, then_body, else_body.as_deref()),
            StructuredStmt::Switch {
                expr,
                cases,
                default_body,
            } => self.lower_switch(expr, cases, default_body.as_deref()),
            StructuredStmt::Assignment { target, value } => self.lower_assignment(target, value),
            StructuredStmt::Expr { expr } => {
                let kind = self.emit_expr(expr)?;
                if is_stackpush_then_call(expr) {
                    return Ok(());
                }
                // A value-producing call used as a bare statement discards its
                // result, which in CS2 is an explicit pop_*_discard after the
                // call. Emit it so such statements round-trip (the recompile gate
                // confirms byte-identity).
                match kind {
                    ValueKind::Void => {}
                    ValueKind::Int => self.emit_instruction("pop_int_discard", Operand::Byte(0)),
                    ValueKind::Object => {
                        self.emit_instruction("pop_string_discard", Operand::Byte(0));
                    }
                    ValueKind::Long => self.emit_instruction("pop_long_discard", Operand::Byte(0)),
                    ValueKind::Unknown => {
                        bail!(
                            "expression statement leaves value of unknown type on stack: {expr:?}"
                        )
                    }
                }
                Ok(())
            }
            StructuredStmt::Goto { target } => {
                self.emit_branch_to("branch", &block_label(*target));
                Ok(())
            }
            StructuredStmt::StackGoto { target, values } => {
                self.emit_stack_values(values)?;
                self.emit_branch_to("branch", &block_label(*target));
                Ok(())
            }
            StructuredStmt::Label { target } => {
                self.mark_label(&block_label(*target));
                Ok(())
            }
            StructuredStmt::Return { value } => {
                if let Some(value) = value {
                    self.emit_return_value(value)?;
                }
                self.emit_instruction("return", Operand::Byte(0));
                Ok(())
            }
            StructuredStmt::Comment(text) => {
                if text.is_empty() {
                    Ok(())
                } else {
                    bail!("structured recompilation does not support comment-only control flow")
                }
            }
            StructuredStmt::Break => self.emit_loop_break(),
            StructuredStmt::Continue => self.emit_loop_continue(),
        }
    }

    fn lower_while(&mut self, body: &[StructuredStmt]) -> Result<()> {
        let continue_label = self.new_label("loop_continue");
        let break_label = self.new_label("loop_break");
        self.mark_label(&continue_label);
        self.loop_labels.push(LoopLabels {
            continue_label: continue_label.clone(),
            break_label: break_label.clone(),
        });
        self.lower_stmts(body)?;
        self.loop_labels.pop();
        if !stmts_terminate(body) {
            self.emit_branch_to("branch", &continue_label);
        }
        self.mark_label(&break_label);
        Ok(())
    }

    fn lower_if(
        &mut self,
        condition: &Expression,
        then_body: &[StructuredStmt],
        else_body: Option<&[StructuredStmt]>,
    ) -> Result<()> {
        // Conditional goto (`if (cond) goto(N)`) from the linear fallback: emit a
        // single conditional branch to the target — matching the original
        // opcode, with the false path falling through to the next block.
        if else_body.is_none()
            && let [StructuredStmt::Goto { target }] = then_body
        {
            return self.emit_goto_condition(condition, &block_label(*target));
        }

        let direct_then_label = self.direct_jump_label(then_body);
        let direct_else_label = else_body.and_then(|body| self.direct_jump_label(body));
        let then_label = direct_then_label
            .clone()
            .unwrap_or_else(|| self.new_label("if_then"));
        let end_label = self.new_label("if_end");
        let else_label = if let Some(label) = direct_else_label.clone() {
            label
        } else {
            else_body
                .map(|_| self.new_label("if_else"))
                .unwrap_or_else(|| end_label.clone())
        };

        self.emit_condition(condition, &then_label, &else_label)?;
        if direct_then_label.is_none() {
            self.mark_label(&then_label);
            self.lower_stmts(then_body)?;
        }
        if let Some(else_body) = else_body {
            // The jump over the else body is unreachable — and the original
            // compiler omits it — when the then body already terminates
            // (returns / breaks / continues). Emitting it anyway adds a stray
            // `branch` that shifts every downstream target (the dominant
            // branch:operand mismatch). Only emit it when control can fall
            // through the then body into the else.
            if direct_then_label.is_none() && !stmts_terminate(then_body) {
                self.emit_branch_to("branch", &end_label);
            }
            if direct_else_label.is_none() {
                self.mark_label(&else_label);
                self.lower_stmts(else_body)?;
            }
        }
        self.mark_label(&end_label);
        Ok(())
    }

    fn direct_jump_label(&self, body: &[StructuredStmt]) -> Option<String> {
        let [stmt] = body else {
            return None;
        };
        match stmt {
            StructuredStmt::Goto { target } => Some(block_label(*target)),
            StructuredStmt::Break => self
                .loop_labels
                .last()
                .map(|labels| labels.break_label.clone()),
            StructuredStmt::Continue => self
                .loop_labels
                .last()
                .map(|labels| labels.continue_label.clone()),
            _ => None,
        }
    }

    fn lower_switch(
        &mut self,
        expr: &Expression,
        cases: &[super::SwitchCaseStmt],
        default_body: Option<&[StructuredStmt]>,
    ) -> Result<()> {
        self.emit_expr(expr)?;
        if default_body.is_none()
            && let Some(case_labels) = direct_goto_switch_case_labels(cases)
        {
            let index = self.instructions.len();
            self.instructions.push(Instruction {
                opcode: 0,
                command: "switch".to_string(),
                operand: Operand::Switch(Vec::new()),
            });
            self.switch_fixups.push((index, case_labels));
            return Ok(());
        }
        let end_label = self.new_label("switch_end");
        let default_label = default_body.map(|_| self.new_label("switch_default"));
        let mut case_labels = cases
            .iter()
            .map(|case| (case.value, self.new_label("switch_case")))
            .collect::<Vec<_>>();
        for index in (0..cases.len()).rev() {
            if cases[index].fallthrough && cases[index].body.is_empty() {
                let fallthrough_label = case_labels
                    .get(index + 1)
                    .map(|(_, label)| label.clone())
                    .or_else(|| default_label.clone())
                    .unwrap_or_else(|| end_label.clone());
                case_labels[index].1 = fallthrough_label;
            }
        }
        let index = self.instructions.len();
        self.instructions.push(Instruction {
            opcode: 0,
            command: "switch".to_string(),
            operand: Operand::Switch(Vec::new()),
        });
        self.switch_fixups.push((index, case_labels.clone()));
        if let Some(default_body) = default_body {
            if let Some(label) = &default_label {
                self.mark_label(label);
            }
            self.lower_stmts(default_body)?;
            if !stmts_terminate(default_body) {
                self.emit_branch_to("branch", &end_label);
            }
        } else {
            self.emit_branch_to("branch", &end_label);
        }
        for (case_index, ((_, label), case)) in case_labels.iter().zip(cases).enumerate() {
            if case.fallthrough && case.body.is_empty() {
                continue;
            }
            self.mark_label(label);
            self.lower_stmts(&case.body)?;
            let is_last_case = case_index + 1 == cases.len();
            if case.break_after || (!is_last_case && !stmts_terminate(&case.body)) {
                self.emit_branch_to("branch", &end_label);
            }
        }
        self.mark_label(&end_label);
        Ok(())
    }

    fn lower_assignment(&mut self, target: &AssignmentTarget, value: &Expression) -> Result<()> {
        if let AssignmentTarget::ArrayAccess { array, index } = target {
            return self.lower_array_assignment(array, index, value);
        }
        if is_pop_call(value) {
            return self.emit_pop_assignment(target);
        }
        self.emit_expr(value)?;
        self.emit_store_assignment(target)
    }

    fn lower_array_assignment(
        &mut self,
        array: &str,
        index: &Expression,
        value: &Expression,
    ) -> Result<()> {
        let Some(array_id) = array.strip_prefix("array_") else {
            bail!("unsupported array target {array}");
        };
        let array_id = array_id.parse::<i32>()?;

        self.emit_expr(index)?;
        let value_kind = if is_pop_call(value) {
            ValueKind::Unknown
        } else {
            self.emit_expr(value)?
        };
        match value_kind {
            ValueKind::Object => {
                bail!("string arrays are not supported (no pop_array_string opcode): {array}[..]")
            }
            ValueKind::Int | ValueKind::Long | ValueKind::Unknown | ValueKind::Void => {
                self.emit_instruction("pop_array_int", Operand::Array(array_id));
                Ok(())
            }
        }
    }

    fn emit_pop_assignment(&mut self, target: &AssignmentTarget) -> Result<()> {
        self.emit_store_assignment(target)
    }

    fn emit_store_assignment(&mut self, target: &AssignmentTarget) -> Result<()> {
        match target {
            AssignmentTarget::Identifier(name) => {
                if let Some(kind) = self.local_types.get(name).copied() {
                    let command = match kind {
                        ValueKind::Int | ValueKind::Unknown => "pop_int_local",
                        ValueKind::Object => "pop_string_local",
                        ValueKind::Long => "pop_long_local",
                        ValueKind::Void => bail!("cannot assign void to local {name}"),
                    };
                    let index = parse_numeric_suffix(name)? as i32;
                    self.emit_instruction(command, Operand::Local(index));
                    return Ok(());
                }
                if let Some(var_ref) = self.ctx.var_refs_by_name.get(name) {
                    self.emit_instruction("pop_var", Operand::VarRef(var_ref.clone()));
                    return Ok(());
                }
                if let Some(varbit_ref) = self.ctx.varbit_refs_by_name.get(name) {
                    self.emit_instruction("pop_varbit", Operand::VarBitRef(varbit_ref.clone()));
                    return Ok(());
                }
                bail!("unsupported assignment target identifier: {name}");
            }
            AssignmentTarget::ArrayAccess { array, .. } => {
                bail!("array assignment target must be lowered before value emission: {array}[..]")
            }
            AssignmentTarget::Opaque(target) => {
                bail!("opaque assignment target is not reversible: {target}")
            }
        }
    }

    /// Emit a single conditional branch to `target_label` for an
    /// `if (cond) goto(N)` (linear control flow), reproducing the original
    /// branch opcode. The false path falls through.
    fn emit_goto_condition(&mut self, condition: &Expression, target_label: &str) -> Result<()> {
        match condition {
            Expression::BinaryOperation(binary) => {
                let left_kind = self.emit_expr(&binary.left)?;
                let right_kind = self.emit_expr(&binary.right)?;
                let branch = branch_command_for_binary(binary.op, left_kind, right_kind)?;
                self.emit_branch_to(branch, target_label);
                Ok(())
            }
            Expression::UnaryOperation(unary) if unary.op == UnaryOp::Not => {
                self.emit_expr(&unary.operand)?;
                self.emit_branch_to("branch_if_false", target_label);
                Ok(())
            }
            other => {
                self.emit_expr(other)?;
                self.emit_branch_to("branch_if_true", target_label);
                Ok(())
            }
        }
    }

    fn emit_condition(
        &mut self,
        condition: &Expression,
        true_label: &str,
        false_label: &str,
    ) -> Result<()> {
        match condition {
            Expression::BooleanLiteral(value) => {
                self.emit_branch_to("branch", if value.value { true_label } else { false_label });
                Ok(())
            }
            Expression::UnaryOperation(unary) if unary.op == UnaryOp::Not => {
                self.emit_condition(&unary.operand, false_label, true_label)
            }
            Expression::BinaryOperation(binary) => match binary.op {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let left_kind = self.emit_expr(&binary.left)?;
                    let right_kind = self.emit_expr(&binary.right)?;
                    let branch = branch_command_for_binary(binary.op, left_kind, right_kind)?;
                    self.emit_branch_to(branch, true_label);
                    self.emit_branch_to("branch", false_label);
                    Ok(())
                }
                BinaryOp::And => {
                    let rhs_label = self.new_label("if_and_rhs");
                    self.emit_condition(&binary.left, &rhs_label, false_label)?;
                    self.mark_label(&rhs_label);
                    self.emit_condition(&binary.right, true_label, false_label)
                }
                BinaryOp::Or => {
                    let rhs_label = self.new_label("if_or_rhs");
                    self.emit_condition(&binary.left, true_label, &rhs_label)?;
                    self.mark_label(&rhs_label);
                    self.emit_condition(&binary.right, true_label, false_label)
                }
                _ => {
                    self.emit_expr(condition)?;
                    self.emit_branch_to("branch_if_true", true_label);
                    self.emit_branch_to("branch", false_label);
                    Ok(())
                }
            },
            _ => {
                self.emit_expr(condition)?;
                self.emit_branch_to("branch_if_true", true_label);
                self.emit_branch_to("branch", false_label);
                Ok(())
            }
        }
    }

    fn emit_expr(&mut self, expr: &Expression) -> Result<ValueKind> {
        match expr {
            Expression::NumberLiteral(value) => {
                // Int constants have two CS2 encodings: the legacy
                // push_constant_int (4-byte int) and the typed-constant
                // push_constant_string with an int tag (1-byte tag + 4-byte
                // int). The RT7 corpus (910/947) emits the typed form
                // universally — push_constant_int as an int-constant origin is
                // essentially absent — so lowering to it is the byte-faithful
                // default and the largest single recompile_mismatch cause
                // (2527 on 947). The recompile-fidelity gate is the backstop:
                // any script whose original genuinely used push_constant_int
                // recompiles non-identically and is correctly marked blocked.
                self.emit_int_constant(value.value);
                Ok(ValueKind::Int)
            }
            Expression::BigIntLiteral(value) => {
                self.emit_instruction("push_long_constant", Operand::Long(value.value));
                Ok(ValueKind::Long)
            }
            Expression::StringLiteral(value) => {
                self.emit_instruction("push_constant_string", Operand::Str(value.value.clone()));
                Ok(ValueKind::Object)
            }
            Expression::BooleanLiteral(value) => {
                self.emit_int_constant(i32::from(value.value));
                Ok(ValueKind::Int)
            }
            Expression::Identifier(identifier) => self.emit_identifier(&identifier.name),
            Expression::ArrayAccess(access) => self.emit_array_access(access),
            Expression::PropertyAccess(access) => self.emit_property_access(access),
            Expression::Call(call) => self.emit_call(call),
            Expression::CallbackLiteral(_) => {
                bail!("callback literals may only appear as hook arguments")
            }
            Expression::BinaryOperation(binary) => self.emit_binary_expr(binary),
            Expression::UnaryOperation(unary) => self.emit_unary_expr(unary),
            Expression::PushOperation(_)
            | Expression::PopOperation(_)
            | Expression::GotoExpr(_) => {
                bail!("stack pseudo-operations are not reversible")
            }
        }
    }

    fn emit_return_value(&mut self, value: &Expression) -> Result<()> {
        if let Expression::Call(call) = value
            && let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "stack"
        {
            self.emit_stack_values(&call.arguments)?;
            return Ok(());
        }

        self.emit_expr(value)?;
        Ok(())
    }

    fn emit_stack_values(&mut self, values: &[Expression]) -> Result<()> {
        let mut index = 0;
        while index < values.len() {
            if let Some(consumed) = self.try_emit_multi_result_stack_prefix(&values[index..])? {
                index += consumed;
                continue;
            }
            self.emit_expr(&values[index])?;
            index += 1;
        }
        Ok(())
    }

    fn try_emit_multi_result_stack_prefix(
        &mut self,
        values: &[Expression],
    ) -> Result<Option<usize>> {
        if let Some(consumed) = self.try_emit_multi_result_script_call_prefix(values)? {
            return Ok(Some(consumed));
        }

        let Some(first) = values.first() else {
            return Ok(None);
        };
        let Some(access) = MultiResultAccess::from_expr(first, self.ctx) else {
            return Ok(None);
        };
        let Some(fields) = multi_result_field_order(&access.command) else {
            return Ok(None);
        };
        if fields.len() > values.len() {
            return Ok(None);
        }

        let access_arguments = format!("{:?}", access.arguments);
        for (value, expected_field) in values.iter().zip(&fields) {
            let Some(next_access) = MultiResultAccess::from_expr(value, self.ctx) else {
                return Ok(None);
            };
            if next_access.command != access.command
                || next_access.field != *expected_field
                || format!("{:?}", next_access.arguments) != access_arguments
            {
                return Ok(None);
            }
        }

        for argument in &access.arguments {
            self.emit_expr(argument)?;
        }
        self.emit_instruction(&access.command, Operand::Byte(0));
        Ok(Some(fields.len()))
    }

    fn try_emit_multi_result_script_call_prefix(
        &mut self,
        values: &[Expression],
    ) -> Result<Option<usize>> {
        let Some(first) = values.first() else {
            return Ok(None);
        };
        let Some(first_access) = ScriptMultiResultAccess::from_expr(first, self.ctx) else {
            return Ok(None);
        };
        let count = first_access.return_count;
        if count <= 1 || count > values.len() {
            return Ok(None);
        }

        let arguments = format!("{:?}", first_access.arguments);
        for (index, value) in values.iter().take(count).enumerate() {
            let Some(access) = ScriptMultiResultAccess::from_expr(value, self.ctx) else {
                return Ok(None);
            };
            if access.script_id != first_access.script_id
                || access.index != index
                || format!("{:?}", access.arguments) != arguments
            {
                return Ok(None);
            }
        }

        self.emit_call_arguments(&first_access.arguments)?;
        self.emit_instruction("gosub_with_params", Operand::Script(first_access.group_id));
        Ok(Some(count))
    }

    /// Emit an int constant using the typed-constant `push_constant_string`
    /// (int tag), the RT7 corpus's universal int-constant encoding (see the
    /// `NumberLiteral` lowering). Centralizes the choice so every plain int
    /// constant — literals, booleans, ids, enum/component constants — recompiles
    /// to the same opcode the original used. The recompile gate is the backstop.
    fn emit_int_constant(&mut self, value: i32) {
        self.emit_instruction("push_constant_string", Operand::Int(value));
    }

    fn emit_identifier(&mut self, name: &str) -> Result<ValueKind> {
        if let Some(kind) = self.local_types.get(name).copied() {
            let command = match kind {
                ValueKind::Int | ValueKind::Unknown => "push_int_local",
                ValueKind::Object => "push_string_local",
                ValueKind::Long => "push_long_local",
                ValueKind::Void => bail!("void local {name} cannot be loaded"),
            };
            let index = parse_numeric_suffix(name)? as i32;
            self.emit_instruction(command, Operand::Local(index));
            return Ok(kind);
        }
        if let Some(var_ref) = self.ctx.var_refs_by_name.get(name) {
            self.emit_instruction("push_var", Operand::VarRef(var_ref.clone()));
            return Ok(ValueKind::Int);
        }
        if let Some(varbit_ref) = self.ctx.varbit_refs_by_name.get(name) {
            self.emit_instruction("push_varbit", Operand::VarBitRef(varbit_ref.clone()));
            return Ok(ValueKind::Int);
        }
        bail!("unsupported identifier expression: {name}")
    }

    fn emit_array_access(&mut self, access: &super::ArrayAccess) -> Result<ValueKind> {
        if let Expression::Identifier(array) = &*access.array
            && let Some(array_id) = array.name.strip_prefix("array_")
        {
            self.emit_expr(&access.index)?;
            self.emit_instruction("push_array_int", Operand::Array(array_id.parse::<i32>()?));
            return Ok(ValueKind::Int);
        }
        bail!("unsupported array access expression")
    }

    fn emit_property_access(&mut self, access: &super::PropertyAccess) -> Result<ValueKind> {
        if let Expression::Identifier(object) = &*access.object {
            let qualified = format!("{}.{}", object.name, access.property);
            if let Some(value) = self.ctx.enum_values_by_name.get(&qualified) {
                self.emit_int_constant(*value);
                return Ok(ValueKind::Int);
            }
            if object.name == "ComponentId" {
                let key = format!("{}.{}", object.name, access.property);
                let Some(value) = self.ctx.component_ids_by_name.get(&key).copied() else {
                    bail!("unknown component constant {key}");
                };
                self.emit_int_constant(value);
                return Ok(ValueKind::Int);
            }
        }
        bail!("property access expressions are only supported for enum and component constants")
    }

    fn emit_call(&mut self, call: &super::CallExpr) -> Result<ValueKind> {
        if let Expression::Identifier(identifier) = &*call.callee
            && call.arguments.is_empty()
        {
            let command = match identifier.name.as_str() {
                "popintdiscard" => Some("pop_int_discard"),
                "popstringdiscard" => Some("pop_string_discard"),
                "poplongdiscard" => Some("pop_long_discard"),
                _ => None,
            };
            if let Some(command) = command {
                self.emit_instruction(command, Operand::Byte(0));
                return Ok(ValueKind::Void);
            }
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "pop"
            && call.arguments.is_empty()
        {
            return Ok(ValueKind::Unknown);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "push"
        {
            let [value] = call.arguments.as_slice() else {
                bail!("push expects 1 argument, got {}", call.arguments.len());
            };
            self.emit_expr(value)?;
            return Ok(ValueKind::Void);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "stackpush_then"
        {
            let Some((statement, values)) = call.arguments.split_last() else {
                bail!("stackpush_then expects at least 1 argument");
            };
            if let Some(kind) = self.try_emit_stackpush_then_stackassign(values, statement)? {
                return Ok(kind);
            }
            self.emit_stack_values(values)?;
            let kind = self.emit_expr(statement)?;
            return Ok(kind);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "concat"
        {
            self.emit_call_arguments(&call.arguments)?;
            self.emit_instruction(
                "join_string",
                Operand::Count(i32::try_from(call.arguments.len())?),
            );
            return Ok(ValueKind::Object);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "intconst"
        {
            let [value] = call.arguments.as_slice() else {
                bail!("intconst expects 1 argument, got {}", call.arguments.len());
            };
            self.emit_instruction("push_constant_int", Operand::Int(literal_i32(value)?));
            return Ok(ValueKind::Int);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && identifier.name == "longconst"
        {
            let [value] = call.arguments.as_slice() else {
                bail!("longconst expects 1 argument, got {}", call.arguments.len());
            };
            self.emit_instruction("push_constant_string", Operand::Long(literal_i64(value)?));
            return Ok(ValueKind::Long);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && let Some(count) = identifier.name.strip_prefix("stackassign_")
        {
            self.emit_stackassign_call(count, &call.arguments)?;
            return Ok(ValueKind::Void);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && let Some(array_id) = identifier.name.strip_prefix("define_array_")
        {
            let [size] = call.arguments.as_slice() else {
                bail!(
                    "{} expects 1 size argument, got {}",
                    identifier.name,
                    call.arguments.len()
                );
            };
            self.emit_expr(size)?;
            self.emit_instruction("define_array", Operand::Array(array_id.parse::<i32>()?));
            return Ok(ValueKind::Void);
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && let Some(array_id) = identifier
                .name
                .strip_prefix("push_array_int_leave_index_on_stack_")
        {
            let [index] = call.arguments.as_slice() else {
                bail!(
                    "{} expects 1 index argument, got {}",
                    identifier.name,
                    call.arguments.len()
                );
            };
            if !self.ctx.has_command("push_array_int_leave_index_on_stack") {
                bail!("push_array_int_leave_index_on_stack is not available in this build");
            }
            self.emit_expr(index)?;
            self.emit_instruction(
                "push_array_int_leave_index_on_stack",
                Operand::Array(array_id.parse::<i32>()?),
            );
            return Ok(ValueKind::Int);
        }

        // Command names are not imported in generated TS, while script calls
        // are. Prefer known opcodes for unimported symbols so command/script
        // name collisions such as `openurlraw` do not get mis-lowered as
        // `gosub_with_params`.
        if let Expression::Identifier(identifier) = &*call.callee
            && !self.is_imported_symbol(&identifier.name)
        {
            let (command_name, command_arguments, operand) =
                if let Some(base_name) = identifier.name.strip_suffix("WithMode") {
                    let Some((mode, arguments)) = call.arguments.split_last() else {
                        bail!("{} expects trailing mode argument", identifier.name);
                    };
                    (base_name, arguments, Operand::Byte(literal_u8(mode)?))
                } else {
                    (
                        identifier.name.as_str(),
                        call.arguments.as_slice(),
                        Operand::Byte(0),
                    )
                };
            if let Some(command) = self.ctx.resolve_command(command_name).map(str::to_string) {
                self.emit_call_arguments(command_arguments)?;
                self.emit_instruction(&command, operand);
                return Ok(command_result_kind(&command, command_arguments, self.ctx));
            }
        }

        if let Expression::Identifier(identifier) = &*call.callee
            && let Some(script_metadata) = self
                .ctx
                .script_catalog
                .resolve_export_name(&identifier.name)
        {
            let emitted_kinds = self.emit_call_arguments(&call.arguments)?;
            let arg_kinds = if emitted_kinds.len() == call.arguments.len() {
                call.arguments
                    .iter()
                    .zip(emitted_kinds)
                    .map(|(argument, kind)| self.script_call_argument_kind(argument, kind))
                    .collect::<Vec<_>>()
            } else {
                emitted_kinds
            };
            let signature = self.ctx.script_signatures.get(&script_metadata.packed_id);
            if let Some(signature) = signature {
                check_call_arity(&identifier.name, signature, &arg_kinds)?;
            }
            self.emit_instruction(
                "gosub_with_params",
                Operand::Script(script_metadata.group_id.0),
            );
            let return_kind = signature.map_or(ValueKind::Unknown, |signature| {
                kind_from_return_type(&signature.return_type)
            });
            return Ok(return_kind);
        }

        if let Expression::PropertyAccess(callee) = &*call.callee
            && let Expression::Identifier(object) = &*callee.object
            && object.name == "UI"
        {
            return self.emit_ui_call(&callee.property, &call.arguments);
        }

        bail!("unsupported call expression: {call:?}")
    }

    fn emit_stackassign_call(&mut self, count: &str, arguments: &[Expression]) -> Result<()> {
        let count = count.parse::<usize>()?;
        if count == 0 || arguments.len() != count * 2 {
            bail!(
                "stackassign_{count} expects {} argument(s), got {}",
                count * 2,
                arguments.len()
            );
        }

        for value in &arguments[count..] {
            self.emit_expr(value)?;
        }
        for target in &arguments[..count] {
            let Expression::StringLiteral(target) = target else {
                bail!("stackassign target must be string literal");
            };
            self.emit_store_assignment(&AssignmentTarget::Identifier(target.value.clone()))?;
        }
        Ok(())
    }

    fn try_emit_stackpush_then_stackassign(
        &mut self,
        preserved_values: &[Expression],
        statement: &Expression,
    ) -> Result<Option<ValueKind>> {
        let Expression::Call(call) = statement else {
            return Ok(None);
        };
        let Expression::Identifier(identifier) = &*call.callee else {
            return Ok(None);
        };
        let Some(count) = identifier.name.strip_prefix("stackassign_") else {
            return Ok(None);
        };
        let count = count.parse::<usize>()?;
        if count == 0 || call.arguments.len() != count * 2 {
            return Ok(None);
        }

        let (targets, assignment_values) = call.arguments.split_at(count);
        let mut values = Vec::with_capacity(preserved_values.len() + assignment_values.len());
        values.extend(preserved_values.iter().cloned());
        values.extend(assignment_values.iter().cloned());
        self.emit_stack_values(&values)?;
        for target in targets {
            let Expression::StringLiteral(target) = target else {
                bail!("stackassign target must be string literal");
            };
            self.emit_store_assignment(&AssignmentTarget::Identifier(target.value.clone()))?;
        }
        Ok(Some(ValueKind::Void))
    }

    fn emit_call_arguments(&mut self, arguments: &[Expression]) -> Result<Vec<ValueKind>> {
        let mut kinds = Vec::with_capacity(arguments.len());
        let mut index = 0;
        while index < arguments.len() {
            if let Some(consumed) = self.try_emit_multi_result_stack_prefix(&arguments[index..])? {
                kinds.extend(std::iter::repeat_n(ValueKind::Unknown, consumed));
                index += consumed;
                continue;
            }
            let kind = self.emit_expr(&arguments[index])?;
            kinds.extend(self.expression_result_kinds(&arguments[index], kind));
            index += 1;
        }
        Ok(kinds)
    }

    fn expression_result_kinds(&self, expr: &Expression, fallback: ValueKind) -> Vec<ValueKind> {
        let Expression::Call(call) = expr else {
            return vec![fallback];
        };
        let Expression::Identifier(identifier) = &*call.callee else {
            return vec![fallback];
        };
        let Some(metadata) = self
            .ctx
            .script_catalog
            .resolve_export_name(&identifier.name)
        else {
            return vec![fallback];
        };
        let signature = self
            .ctx
            .script_signatures
            .get(&metadata.packed_id)
            .unwrap_or(&metadata.signature);
        if signature.total_returns() <= 1 {
            return vec![fallback];
        }
        let mut kinds = Vec::with_capacity(signature.total_returns());
        kinds.extend(std::iter::repeat_n(
            ValueKind::Int,
            signature.return_count_int as usize,
        ));
        kinds.extend(std::iter::repeat_n(
            ValueKind::Object,
            signature.return_count_obj as usize,
        ));
        kinds.extend(std::iter::repeat_n(
            ValueKind::Long,
            signature.return_count_long as usize,
        ));
        kinds
    }

    fn is_imported_symbol(&self, name: &str) -> bool {
        self.script
            .imports
            .iter()
            .any(|import| import.named_exports.iter().any(|export| export == name))
    }

    fn script_call_argument_kind(&self, argument: &Expression, kind: ValueKind) -> ValueKind {
        if matches!(argument, Expression::Call(_)) {
            return ValueKind::Unknown;
        }
        if let Expression::Identifier(identifier) = argument
            && (self.local_types.contains_key(&identifier.name)
                || self.ctx.var_refs_by_name.contains_key(&identifier.name))
        {
            return ValueKind::Unknown;
        }
        kind
    }

    fn emit_ui_call(&mut self, method: &str, arguments: &[Expression]) -> Result<ValueKind> {
        let mut method = method;
        let mut arguments = arguments;
        let mut mode_operand = Operand::Byte(0);
        if let Some(base_method) = method.strip_suffix("WithMode") {
            let Some((mode, base_arguments)) = arguments.split_last() else {
                bail!("UI.{method} expects trailing mode argument");
            };
            mode_operand = Operand::Byte(literal_u8(mode)?);
            method = base_method;
            arguments = base_arguments;
        }

        match method {
            "create" => {
                let ([parent, kind, id], operand) = match arguments {
                    [parent, kind, id] => ([parent, kind, id], Operand::Byte(0)),
                    [parent, kind, id, mode] => {
                        ([parent, kind, id], Operand::Byte(literal_u8(mode)?))
                    }
                    _ => bail!(
                        "UI.create expects 3 or 4 arguments, got {}",
                        arguments.len()
                    ),
                };
                self.emit_expr(parent)?;
                self.emit_expr(kind)?;
                self.emit_expr(id)?;
                self.emit_instruction("cc_create", operand);
                Ok(ValueKind::Void)
            }
            "delete" => {
                self.emit_instruction("cc_delete", mode_operand);
                Ok(ValueKind::Void)
            }
            "deleteAll" => {
                let [target] = arguments else {
                    bail!("UI.deleteAll expects 1 argument, got {}", arguments.len());
                };
                self.emit_expr(target)?;
                self.emit_instruction("cc_deleteall", mode_operand);
                Ok(ValueKind::Void)
            }
            "find" => {
                let (command, operand) = match arguments {
                    [component] => {
                        self.emit_expr(component)?;
                        ("if_find", Operand::Byte(0))
                    }
                    [parent, child] => {
                        self.emit_expr(parent)?;
                        self.emit_expr(child)?;
                        ("cc_find", Operand::Byte(0))
                    }
                    [parent, child, mode] => {
                        self.emit_expr(parent)?;
                        self.emit_expr(child)?;
                        ("cc_find", Operand::Byte(literal_u8(mode)?))
                    }
                    _ => bail!("UI.find expects 1, 2, or 3 arguments"),
                };
                self.emit_instruction(command, operand);
                Ok(ValueKind::Int)
            }
            "findInterface" => {
                let [component, mode] = arguments else {
                    bail!("UI.findInterface expects component and mode");
                };
                self.emit_expr(component)?;
                self.emit_instruction("if_find", Operand::Byte(literal_u8(mode)?));
                Ok(ValueKind::Int)
            }
            "getText" => {
                let [component] = arguments else {
                    bail!("UI.getText expects 1 argument, got {}", arguments.len());
                };
                self.emit_expr(component)?;
                self.emit_instruction("if_gettext", mode_operand);
                Ok(ValueKind::Object)
            }
            "sendToFront" => {
                for argument in arguments {
                    self.emit_expr(argument)?;
                }
                let command = match arguments.len() {
                    0 => "cc_sendtofront",
                    1 => "if_sendtofront",
                    _ => bail!("UI.sendToFront expects 0 or 1 arguments"),
                };
                self.emit_instruction(command, mode_operand);
                Ok(ValueKind::Void)
            }
            "sendToBack" => {
                for argument in arguments {
                    self.emit_expr(argument)?;
                }
                let command = match arguments.len() {
                    0 => "cc_sendtoback",
                    1 => "if_sendtoback",
                    _ => bail!("UI.sendToBack expects 0 or 1 arguments"),
                };
                self.emit_instruction(command, mode_operand);
                Ok(ValueKind::Void)
            }
            method if method.starts_with("Get") => {
                self.emit_ui_getter(method, arguments, mode_operand)
            }
            method if method.contains("Seton") => {
                self.emit_ui_hook_call(method, arguments, mode_operand)
            }
            "ListAddentry" => self.emit_ui_list_addentry(arguments, mode_operand),
            "ScriptqueueAdd" => self.emit_ui_scriptqueue_add(arguments, mode_operand),
            method => {
                // Generic inverse of the decompiler's UI naming. It renders
                // unhandled `cc_*`/`if_*` commands as `UI.{sanitize_camel(suffix)}`,
                // so recover the original command by matching that transform
                // and the observed argument count.
                let command =
                    if let Some(command) = self.resolve_ui_command(method, arguments.len()) {
                        command
                    } else {
                        let lower = method.to_ascii_lowercase();
                        let starts_upper = method.starts_with(|c: char| c.is_ascii_uppercase());
                        let if_form = format!("if_{lower}");
                        let cc_form = format!("cc_{lower}");
                        if starts_upper && self.ctx.has_command(&if_form) {
                            if_form
                        } else if self.ctx.has_command(&cc_form) {
                            cc_form
                        } else {
                            match method {
                                "setParam" => "cc_setparam".to_string(),
                                "setParamInt" => "cc_setparam_int".to_string(),
                                "setParamString" => "cc_setparam_string".to_string(),
                                _ => bail!("unsupported UI method {method}"),
                            }
                        }
                    };
                for argument in arguments {
                    self.emit_expr(argument)?;
                }
                self.emit_instruction(&command, mode_operand);
                Ok(command_result_kind(&command, arguments, self.ctx))
            }
        }
    }

    /// Lower a component getter (`UI.Getwidth`, `UI.Getx`, ...). These come from
    /// the recovery's generic `UI.Get*` rendering of `cc_get*`/`if_get*`: the
    /// current-component `cc_` form takes no argument, the interface `if_` form
    /// takes an explicit component, so pick by arg count. Getters push a value,
    /// so return its kind (string for `gettext`, int otherwise) — the inverse of
    /// the recovery modelling them as value-producing.
    fn emit_ui_getter(
        &mut self,
        method: &str,
        arguments: &[Expression],
        operand: Operand,
    ) -> Result<ValueKind> {
        let lower = method.to_ascii_lowercase();
        let cc_form = format!("cc_{lower}");
        let if_form = format!("if_{lower}");
        let command = if let Some(command) = self.resolve_ui_command(method, arguments.len()) {
            command
        } else if arguments.is_empty() && self.ctx.has_command(&cc_form) {
            cc_form
        } else if self.ctx.has_command(&if_form) {
            if_form
        } else if self.ctx.has_command(&cc_form) {
            cc_form
        } else {
            bail!("unsupported UI getter {method}");
        };
        for argument in arguments {
            self.emit_expr(argument)?;
        }
        self.emit_instruction(&command, operand);
        Ok(if lower == "gettext" {
            ValueKind::Object
        } else {
            ValueKind::Int
        })
    }

    fn resolve_ui_command(&self, method: &str, arg_count: usize) -> Option<String> {
        let mut candidates = Vec::new();
        for command in &self.ctx.opcode_commands {
            let Some(suffix) = ui_command_suffix(command) else {
                continue;
            };
            if ui_method_from_suffix(suffix) != method {
                continue;
            }
            if ui_command_arg_count(command) != Some(arg_count) {
                continue;
            }
            candidates.push(command.clone());
        }
        candidates.sort();
        candidates.into_iter().next()
    }

    fn emit_ui_hook_call(
        &mut self,
        method: &str,
        arguments: &[Expression],
        operand: Operand,
    ) -> Result<ValueKind> {
        // The decompiler renders every `cc_seton*`/`if_seton*` hook as
        // `UI.Seton<suffix>` (sanitize_camel). Derive the opcode pair generically
        // instead of a hardcoded list so the whole hook family round-trips: the
        // component-context form takes just the callback, the interface form an
        // extra explicit component (same arg-count split as the cc_/if_ set
        // methods).
        let (callback_expr, component_expr, has_component) = match arguments {
            [callback] => (callback, None, false),
            [callback, component] => (callback, Some(component), true),
            _ => bail!("UI hook methods expect callback and optional component"),
        };
        let Some(command) = self.resolve_ui_hook_command(method, has_component) else {
            bail!("unsupported UI hook method {method}");
        };
        if let Expression::CallbackLiteral(callback) = callback_expr {
            self.emit_callback_payload(callback, Some(&command))?;
        } else {
            self.emit_expr(callback_expr)?;
        }
        if let Some(component_expr) = component_expr {
            self.emit_expr(component_expr)?;
        }
        self.emit_instruction(&command, operand);
        Ok(ValueKind::Void)
    }

    fn resolve_ui_hook_command(&self, method: &str, has_component: bool) -> Option<String> {
        let prefix = if has_component { "if_" } else { "cc_" };
        let mut candidates = Vec::new();
        for command in &self.ctx.opcode_commands {
            if !command.starts_with(prefix) {
                continue;
            }
            let Some(suffix) = ui_command_suffix(command) else {
                continue;
            };
            if suffix.contains("seton") && ui_method_from_suffix(suffix) == method {
                candidates.push(command.clone());
            }
        }
        candidates.sort();
        candidates.into_iter().next()
    }

    fn emit_ui_list_addentry(
        &mut self,
        arguments: &[Expression],
        operand: Operand,
    ) -> Result<ValueKind> {
        let [id, text, target] = arguments else {
            bail!(
                "UI.ListAddentry expects 3 arguments, got {}",
                arguments.len()
            );
        };
        let command = if is_component_constant(target) && self.ctx.has_command("if_list_addentry") {
            "if_list_addentry"
        } else if self.ctx.has_command("cc_list_addentry") {
            "cc_list_addentry"
        } else if self.ctx.has_command("if_list_addentry") {
            "if_list_addentry"
        } else {
            bail!("unsupported UI method ListAddentry");
        };
        self.emit_expr(id)?;
        self.emit_expr(text)?;
        self.emit_expr(target)?;
        self.emit_instruction(command, operand);
        Ok(command_result_kind(command, arguments, self.ctx))
    }

    fn emit_ui_scriptqueue_add(
        &mut self,
        arguments: &[Expression],
        operand: Operand,
    ) -> Result<ValueKind> {
        let (delay_expr, callback_expr, component_expr, command) = match arguments {
            [delay, callback] => (delay, callback, None, "cc_scriptqueue_add"),
            [delay, callback, component] => {
                (delay, callback, Some(component), "if_scriptqueue_add")
            }
            _ => bail!("UI.ScriptqueueAdd expects delay, callback, and optional component"),
        };
        if !self.ctx.has_command(command) {
            bail!("unsupported UI method ScriptqueueAdd");
        }
        let Expression::CallbackLiteral(callback) = callback_expr else {
            bail!("UI.ScriptqueueAdd second argument must be callback literal");
        };
        self.emit_expr(delay_expr)?;
        self.emit_callback_payload(callback, None)?;
        if let Some(component_expr) = component_expr {
            self.emit_expr(component_expr)?;
        }
        self.emit_instruction(command, operand);
        Ok(ValueKind::Long)
    }

    fn emit_callback_payload(
        &mut self,
        callback: &CallbackLiteral,
        hook_command: Option<&str>,
    ) -> Result<()> {
        if let Some(raw_id) = self.static_callback_target(callback)? {
            self.emit_int_constant(raw_id);
        } else if callback.script == "pop()" {
            // Dynamic target already sits on int stack.
        } else if matches!(
            self.local_types.get(&callback.script),
            Some(ValueKind::Int | ValueKind::Unknown)
        ) {
            self.emit_identifier(&callback.script)?;
        } else {
            bail!("unknown callback target {}", callback.script);
        }
        for argument in &callback.arguments {
            self.emit_expr(argument)?;
        }
        if !callback.watchers.is_empty() {
            for watcher in &callback.watchers {
                self.emit_callback_watcher(watcher, hook_command)?;
            }
            self.emit_int_constant(callback.watchers.len() as i32);
        }
        self.emit_instruction(
            "push_constant_string",
            Operand::Str(callback.raw_descriptor.clone()),
        );
        Ok(())
    }

    fn static_callback_target(&self, callback: &CallbackLiteral) -> Result<Option<i32>> {
        let raw_id = if let Some(script_id) = callback.script_id {
            script_id
        } else if let Some(metadata) = self
            .ctx
            .script_catalog
            .resolve_export_name(&callback.script)
        {
            metadata.group_id.0
        } else if let Some(value) = self.ctx.enum_values_by_name.get(&callback.script) {
            *value
        } else if let Some(id) = callback.script.strip_prefix("script") {
            id.parse::<i32>()?
        } else {
            return Ok(None);
        };
        Ok(Some(raw_id))
    }

    fn emit_callback_watcher(&mut self, watcher: &str, hook_command: Option<&str>) -> Result<()> {
        if matches!(
            hook_command,
            Some("cc_setoninvtransmit" | "if_setoninvtransmit")
        ) && self.ctx.var_refs_by_name.contains_key(watcher)
        {
            self.emit_identifier(watcher)?;
            return Ok(());
        }

        // Watcher trigger ids are int constants, encoded the same typed-constant
        // way as every other int constant in the corpus (see emit_int_constant).
        if let Some(value) = self.ctx.enum_values_by_name.get(watcher).copied() {
            self.emit_int_constant(value);
            return Ok(());
        }
        if let Some(value) = self.ctx.component_ids_by_name.get(watcher).copied() {
            self.emit_int_constant(value);
            return Ok(());
        }
        if let Some(var_ref) = self.ctx.var_refs_by_name.get(watcher) {
            let id = i32::from(var_ref.id);
            self.emit_int_constant(id);
            return Ok(());
        }
        if let Some(varbit_ref) = self.ctx.varbit_refs_by_name.get(watcher) {
            let id = i32::from(varbit_ref.id);
            self.emit_int_constant(id);
            return Ok(());
        }
        for prefix in [
            "inv_",
            "stat_",
            "varc_",
            "varcstr_",
            "varplayer_",
            "varplayerint_",
            "varplayerbit_",
        ] {
            if let Some(id) = watcher.strip_prefix(prefix) {
                self.emit_int_constant(id.parse::<i32>()?);
                return Ok(());
            }
        }
        if self.local_types.contains_key(watcher) {
            let kind = self.emit_identifier(watcher)?;
            if kind == ValueKind::Int {
                return Ok(());
            }
        }
        bail!("unsupported callback watcher {watcher}")
    }

    fn emit_binary_expr(&mut self, binary: &super::BinaryOperation) -> Result<ValueKind> {
        match binary.op {
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::And
            | BinaryOp::Or => {
                self.emit_expr(&binary.left)?;
                self.emit_expr(&binary.right)?;
                self.emit_binary_arithmetic_command(binary.op)
            }
            _ => bail!(
                "comparison/logical expressions are only supported in control-flow conditions"
            ),
        }
    }

    fn emit_binary_arithmetic_command(&mut self, op: BinaryOp) -> Result<ValueKind> {
        let command = match op {
            BinaryOp::Add => "add",
            BinaryOp::Sub if self.ctx.has_command("sub") => "sub",
            BinaryOp::Sub if self.ctx.has_command("quickchat_dynamic_command_add") => {
                "quickchat_dynamic_command_add"
            }
            BinaryOp::Sub => "sub",
            BinaryOp::Mul => "multiply",
            BinaryOp::Div => "divide",
            BinaryOp::Mod => "modulo",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            _ => bail!("comparison/logical expressions are only supported in value expressions"),
        };
        self.emit_instruction(command, Operand::Byte(0));
        Ok(ValueKind::Int)
    }

    fn emit_unary_expr(&mut self, unary: &super::UnaryOperation) -> Result<ValueKind> {
        match unary.op {
            UnaryOp::Neg => match &*unary.operand {
                Expression::NumberLiteral(value) => {
                    self.emit_int_constant(-value.value);
                    Ok(ValueKind::Int)
                }
                Expression::BigIntLiteral(value) => {
                    self.emit_instruction("push_long_constant", Operand::Long(-value.value));
                    Ok(ValueKind::Long)
                }
                _ => bail!("non-literal negation is not supported"),
            },
            UnaryOp::Not => bail!("logical not is only supported in control-flow conditions"),
        }
    }

    fn emit_loop_break(&mut self) -> Result<()> {
        let break_label = self
            .loop_labels
            .last()
            .map(|labels| labels.break_label.clone());
        let Some(break_label) = break_label else {
            bail!("break outside loop");
        };
        self.emit_branch_to("branch", &break_label);
        Ok(())
    }

    fn emit_loop_continue(&mut self) -> Result<()> {
        let continue_label = self
            .loop_labels
            .last()
            .map(|labels| labels.continue_label.clone());
        let Some(continue_label) = continue_label else {
            bail!("continue outside loop");
        };
        self.emit_branch_to("branch", &continue_label);
        Ok(())
    }

    fn emit_instruction(&mut self, command: &str, operand: Operand) {
        self.instructions.push(Instruction {
            opcode: 0,
            command: command.to_string(),
            operand,
        });
    }

    fn emit_branch_to(&mut self, command: &str, label: &str) {
        let index = self.instructions.len();
        self.instructions.push(Instruction {
            opcode: 0,
            command: command.to_string(),
            operand: Operand::Branch(0),
        });
        self.branch_fixups.push((index, label.to_string()));
    }

    fn mark_label(&mut self, label: &str) {
        self.labels
            .insert(label.to_string(), self.instructions.len());
    }

    fn new_label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_label);
        self.next_label += 1;
        label
    }
}

/// Validate a `gosub_with_params` call against the callee's signature. A wrong
/// argument count (or per-type shape) silently corrupts the tri-typed stack at
/// runtime, so reject it at compile time. The per-type check only runs when
/// every argument has a concrete kind; an `Unknown`/`Void` argument falls back
/// to the total-count check to avoid false positives.
fn check_call_arity(
    callee: &str,
    signature: &ScriptSignature,
    arg_kinds: &[ValueKind],
) -> Result<()> {
    if arg_kinds.len() != signature.total_args() {
        bail!(
            "call to `{callee}` expects {} argument(s), got {}",
            signature.total_args(),
            arg_kinds.len()
        );
    }
    if arg_kinds
        .iter()
        .all(|kind| matches!(kind, ValueKind::Int | ValueKind::Long | ValueKind::Object))
    {
        let mut got_int = 0_usize;
        let mut got_obj = 0_usize;
        let mut got_long = 0_usize;
        for kind in arg_kinds {
            match kind {
                ValueKind::Int => got_int += 1,
                ValueKind::Object => got_obj += 1,
                ValueKind::Long => got_long += 1,
                _ => {}
            }
        }
        if got_int != signature.arg_count_int as usize
            || got_obj != signature.arg_count_obj as usize
            || got_long != signature.arg_count_long as usize
        {
            bail!(
                "call to `{callee}` expects (int={}, obj={}, long={}) arguments, got (int={got_int}, obj={got_obj}, long={got_long})",
                signature.arg_count_int,
                signature.arg_count_obj,
                signature.arg_count_long
            );
        }
    }
    Ok(())
}

fn value_kind_for_type(annotation: TypeAnnotation) -> ValueKind {
    match annotation {
        TypeAnnotation::Number | TypeAnnotation::Boolean | TypeAnnotation::Unknown => {
            ValueKind::Int
        }
        TypeAnnotation::BigInt => ValueKind::Long,
        TypeAnnotation::String => ValueKind::Object,
    }
}

fn branch_command_for_binary(
    op: BinaryOp,
    left_kind: ValueKind,
    right_kind: ValueKind,
) -> Result<&'static str> {
    let is_long = matches!(left_kind, ValueKind::Long) || matches!(right_kind, ValueKind::Long);
    let command = match (op, is_long) {
        (BinaryOp::Eq, false) => "branch_equals",
        (BinaryOp::Eq, true) => "long_branch_equals",
        (BinaryOp::Ne, false) => "branch_not",
        (BinaryOp::Ne, true) => "long_branch_not",
        (BinaryOp::Lt, false) => "branch_less_than",
        (BinaryOp::Lt, true) => "long_branch_less_than",
        (BinaryOp::Le, false) => "branch_less_than_or_equals",
        (BinaryOp::Le, true) => "long_branch_less_than_or_equals",
        (BinaryOp::Gt, false) => "branch_greater_than",
        (BinaryOp::Gt, true) => "long_branch_greater_than",
        (BinaryOp::Ge, false) => "branch_greater_than_or_equals",
        (BinaryOp::Ge, true) => "long_branch_greater_than_or_equals",
        _ => bail!("unsupported condition operator"),
    };
    Ok(command)
}

fn kind_from_return_type(value: &str) -> ValueKind {
    if return_type_contains(value, "void") && !value.contains('|') {
        return ValueKind::Void;
    }
    if return_type_contains(value, "string") {
        return ValueKind::Object;
    }
    if return_type_contains(value, "bigint") {
        return ValueKind::Long;
    }
    if return_type_contains(value, "number") || return_type_contains(value, "boolean") {
        return ValueKind::Int;
    }
    match parse_type_annotation(value) {
        TypeAnnotation::Number | TypeAnnotation::Boolean | TypeAnnotation::Unknown => {
            if value == "void" {
                ValueKind::Void
            } else {
                ValueKind::Int
            }
        }
        TypeAnnotation::BigInt => ValueKind::Long,
        TypeAnnotation::String => ValueKind::Object,
    }
}

fn return_type_contains(value: &str, ty: &str) -> bool {
    value.split('|').any(|part| part.trim() == ty)
}

fn count_locals(locals: &[super::LocalVariable], ty: TypeAnnotation) -> u16 {
    locals
        .iter()
        .filter(|local| local.type_annotation == ty)
        .count() as u16
}

fn count_args(args: &[super::ArgumentVariable], ty: TypeAnnotation) -> u16 {
    args.iter()
        .filter(|argument| argument.type_annotation == ty)
        .count() as u16
}

fn parse_numeric_suffix(name: &str) -> Result<usize> {
    let Some((_, suffix)) = name.rsplit_once('_') else {
        bail!("missing numeric suffix for {name}");
    };
    suffix.parse::<usize>().map_err(Into::into)
}

fn assignment_parts(stmt: &StructuredStmt) -> Option<(&AssignmentTarget, &Expression)> {
    if let StructuredStmt::Assignment { target, value } = stmt {
        Some((target, value))
    } else {
        None
    }
}

struct MultiResultAccess {
    command: String,
    arguments: Vec<Expression>,
    field: String,
}

struct ScriptMultiResultAccess {
    script_id: super::ScriptId,
    group_id: i32,
    arguments: Vec<Expression>,
    index: usize,
    return_count: usize,
}

impl ScriptMultiResultAccess {
    fn from_expr(expr: &Expression, ctx: &ReverseCompileContext) -> Option<Self> {
        let Expression::ArrayAccess(access) = expr else {
            return None;
        };
        let Expression::Call(call) = &*access.array else {
            return None;
        };
        let Expression::Identifier(identifier) = &*call.callee else {
            return None;
        };
        let Expression::NumberLiteral(index) = &*access.index else {
            return None;
        };
        let index = usize::try_from(index.value).ok()?;
        let metadata = ctx.script_catalog.resolve_export_name(&identifier.name)?;
        let signature = ctx
            .script_signatures
            .get(&metadata.packed_id)
            .unwrap_or(&metadata.signature);
        let return_count = signature.total_returns();
        (return_count > 1 && index < return_count).then_some(Self {
            script_id: metadata.packed_id,
            group_id: metadata.group_id.0,
            arguments: call.arguments.clone(),
            index,
            return_count,
        })
    }
}

impl MultiResultAccess {
    fn from_expr(expr: &Expression, ctx: &ReverseCompileContext) -> Option<Self> {
        match expr {
            Expression::PropertyAccess(access) => {
                let Expression::Call(call) = &*access.object else {
                    return None;
                };
                let (command, arguments) = call_command_parts(call, ctx)?;
                Some(Self {
                    command,
                    arguments,
                    field: access.property.clone(),
                })
            }
            Expression::ArrayAccess(access) => {
                let Expression::Call(call) = &*access.array else {
                    return None;
                };
                let Expression::NumberLiteral(index) = &*access.index else {
                    return None;
                };
                let (command, arguments) = call_command_parts(call, ctx)?;
                Some(Self {
                    command,
                    arguments,
                    field: index.value.to_string(),
                })
            }
            _ => None,
        }
    }
}

fn call_command_parts(
    call: &super::CallExpr,
    ctx: &ReverseCompileContext,
) -> Option<(String, Vec<Expression>)> {
    let Expression::Identifier(identifier) = &*call.callee else {
        return None;
    };
    let command = ctx.resolve_command(&identifier.name)?.to_string();
    Some((command, call.arguments.clone()))
}

fn multi_result_field_order(command: &str) -> Option<Vec<String>> {
    let fields: &[&str] = match command {
        "get_mousebuttons" => &["primary", "middle", "secondary"],
        "get_active_minimenu_entry" | "get_second_minimenu_entry" => {
            &["entityType", "op", "opBase", "questIconSuffix"]
        }
        "worldlist_start" | "worldlist_next" => &[
            "id",
            "flags",
            "activity",
            "countryId",
            "countryName",
            "players",
            "ping",
            "host",
        ],
        "worldlist_specific" => &[
            "flags",
            "activity",
            "countryId",
            "countryName",
            "players",
            "ping",
            "host",
        ],
        "pushCanvasSize" | "viewport_geteffectivesize" | "fullscreen_getmode" => {
            &["width", "height"]
        }
        "viewport_getzoom" => &["min", "max"],
        "viewport_getfov" => &["max", "min"],
        _ => {
            let count = match command {
                "get_minimenu_length" => 2,
                "get_minimenu_target" => 3,
                "unknown_command_28" => 2,
                "unknown_command_29" => 3,
                "store_lookup" => 13,
                "cc_getcharposatindex" | "if_getcharposatindex" => 2,
                "pushZeroInsets" | "window_getinsets" => 4,
                "pushFontMetrics" => 5,
                _ => return None,
            };
            return Some((0..count).map(|index| index.to_string()).collect());
        }
    };
    Some(fields.iter().map(|field| (*field).to_string()).collect())
}

fn multi_result_pop_order(command: &str) -> Option<Vec<String>> {
    let mut fields = multi_result_field_order(command)?;
    fields.reverse();
    Some(fields)
}

fn ui_command_suffix(command: &str) -> Option<&str> {
    command
        .strip_prefix("cc_")
        .or_else(|| command.strip_prefix("if_"))
}

fn ui_method_from_suffix(suffix: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for c in suffix.chars() {
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

fn ui_command_arg_count(command: &str) -> Option<usize> {
    let suffix = ui_command_suffix(command)?;
    let component_arg = usize::from(command.starts_with("if_"));
    match suffix {
        "set_gamescreen_enabled" => Some(1),
        "find_parent" | "get_gamescreen" => Some(0),
        "setobject_highres" => Some(component_arg + 1),
        suffix if suffix.starts_with("setobject") => Some(component_arg + 2),
        "setparam" | "setparam_int" | "setparam_string" => Some(component_arg + 2),
        "getmodelangle_x" | "getmodelangle_y" | "getmodelangle_z" => Some(component_arg),
        "resume_pausebutton" | "scriptqueue_clear" => Some(component_arg),
        "scriptqueue_clear_script" | "button_setcantoggle" | "button_settoggled" => {
            Some(component_arg + 1)
        }
        "button_setlinkobjoptions" => Some(component_arg + 2),
        "grid_setlayoutparams" => Some(component_arg + 3),
        "button_settextareasizeoffsets" => Some(component_arg + 4),
        _ => super::expr_recovery::opcode_stack_effect(command).map(|effect| effect.total_pops()),
    }
}

fn is_pop_call(value: &Expression) -> bool {
    match value {
        Expression::Call(call) if call.arguments.is_empty() => {
            matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "pop")
        }
        Expression::PopOperation(_) => true,
        _ => false,
    }
}

fn is_stackpush_then_call(value: &Expression) -> bool {
    matches!(
        value,
        Expression::Call(call)
            if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
    )
}

fn literal_i32(value: &Expression) -> Result<i32> {
    match value {
        Expression::NumberLiteral(number) => Ok(number.value),
        Expression::PropertyAccess(property) => {
            let Some(suffix) = property.property.strip_prefix("KEY_") else {
                bail!("expected numeric literal");
            };
            Ok(suffix.parse()?)
        }
        _ => bail!("expected numeric literal"),
    }
}

fn is_component_constant(value: &Expression) -> bool {
    match value {
        Expression::NumberLiteral(number) => number.value >= 65_536,
        Expression::PropertyAccess(access) => {
            matches!(&*access.object, Expression::Identifier(identifier) if identifier.name == "ComponentId")
        }
        _ => false,
    }
}

fn literal_u8(value: &Expression) -> Result<u8> {
    Ok(u8::try_from(literal_i32(value)?)?)
}

fn literal_i64(value: &Expression) -> Result<i64> {
    match value {
        Expression::BigIntLiteral(bigint) => Ok(bigint.value),
        Expression::NumberLiteral(number) => Ok(i64::from(number.value)),
        Expression::UnaryOperation(unary) if unary.op == UnaryOp::Neg => {
            let value = literal_i64(&unary.operand)?;
            let Some(negated) = value.checked_neg() else {
                bail!("bigint literal out of range");
            };
            Ok(negated)
        }
        _ => bail!("expected bigint literal"),
    }
}

fn direct_goto_switch_case_labels(cases: &[super::SwitchCaseStmt]) -> Option<Vec<(i32, String)>> {
    let mut labels = Vec::with_capacity(cases.len());
    for case in cases {
        if case.fallthrough {
            return None;
        }
        let [StructuredStmt::Goto { target }] = case.body.as_slice() else {
            return None;
        };
        labels.push((case.value, block_label(*target)));
    }
    Some(labels)
}

#[cfg(test)]
mod tests {
    use super::{ReverseCompileContext, ValueKind, kind_from_return_type, lower_structured_script};
    use crate::script::{Operand, VarBitRef, VarRef};
    use crate::transpile::ast::{
        ArrayAccess, BigIntLiteral, BinaryOp, BinaryOperation, CallExpr, CallbackLiteral,
        Expression, Identifier, LocalVariable, NumberLiteral, PropertyAccess, ScriptId,
        StringLiteral, TypeAnnotation,
    };
    use crate::transpile::reversible_format::ReversibleMetadata;
    use crate::transpile::structured::{
        AssignmentTarget, StructuredScript, StructuredStmt, SwitchCaseStmt,
    };
    use crate::transpile::{
        ScriptCatalog, ScriptGroupId, ScriptKind, ScriptMetadata, ScriptSignature,
    };
    use crate::vars::VarDomain;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn return_type_kind_uses_value_type_inside_void_union() {
        assert_eq!(kind_from_return_type("string | void"), ValueKind::Object);
        assert_eq!(kind_from_return_type("bigint | void"), ValueKind::Long);
        assert_eq!(kind_from_return_type("number | void"), ValueKind::Int);
    }

    #[test]
    fn concat_call_lowers_to_join_string_with_count_operand() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_obj_0".to_string(),
                type_annotation: TypeAnnotation::String,
            }],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_obj_0".to_string()),
                value: call("concat", vec![string("a"), string("b")]),
            }],
        );
        let ctx = context(
            947,
            &["push_constant_string", "join_string", "pop_string_local"],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("concat should lower to join_string");

        assert_eq!(compiled.code[2].command, "join_string");
        assert!(matches!(compiled.code[2].operand, Operand::Count(2)));
    }

    #[test]
    fn intconst_call_lowers_to_legacy_constant_opcode() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Return {
                value: Some(call("intconst", vec![number(16)])),
            }],
        );
        let ctx = context(947, &["push_constant_int", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("intconst should lower to push_constant_int");

        assert_eq!(compiled.code[0].command, "push_constant_int");
        assert!(matches!(compiled.code[0].operand, Operand::Int(16)));
    }

    #[test]
    fn longconst_call_lowers_to_typed_long_constant_opcode() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Return {
                value: Some(call(
                    "longconst",
                    vec![Expression::BigIntLiteral(
                        crate::transpile::ast::BigIntLiteral { value: 0 },
                    )],
                )),
            }],
        );
        let ctx = context(947, &["push_constant_string", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("longconst should lower to push_constant_string long operand");

        assert_eq!(compiled.code[0].command, "push_constant_string");
        assert!(matches!(compiled.code[0].operand, Operand::Long(0)));
    }

    #[test]
    fn stackassign_call_lowers_values_before_local_stores() {
        let script = script_with_body(
            vec![
                LocalVariable {
                    index: 2,
                    name: "local_int_2".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
                LocalVariable {
                    index: 3,
                    name: "local_int_3".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
            ],
            vec![StructuredStmt::Expr {
                expr: call(
                    "stackassign_2",
                    vec![
                        string("local_int_3"),
                        string("local_int_2"),
                        number(20),
                        number(40),
                    ],
                ),
            }],
        );
        let ctx = context(947, &["push_constant_string", "pop_int_local"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackassign should preserve push-then-pop layout");

        assert_eq!(compiled.code[0].command, "push_constant_string");
        assert!(matches!(compiled.code[0].operand, Operand::Int(20)));
        assert_eq!(compiled.code[1].command, "push_constant_string");
        assert!(matches!(compiled.code[1].operand, Operand::Int(40)));
        assert_eq!(compiled.code[2].command, "pop_int_local");
        assert!(matches!(compiled.code[2].operand, Operand::Local(3)));
        assert_eq!(compiled.code[3].command, "pop_int_local");
        assert!(matches!(compiled.code[3].operand, Operand::Local(2)));
    }

    #[test]
    fn stackassign_call_lowers_values_before_varbit_stores() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call(
                    "stackassign_2",
                    vec![
                        string("varplayerbit_7058"),
                        string("varplayerbit_7057"),
                        number(20),
                        number(40),
                    ],
                ),
            }],
        );
        let mut ctx = context(947, &["push_constant_string", "pop_varbit"]);
        ctx.varbit_refs_by_name.insert(
            "varplayerbit_7058".to_string(),
            VarBitRef {
                id: 7058,
                transmog: false,
            },
        );
        ctx.varbit_refs_by_name.insert(
            "varplayerbit_7057".to_string(),
            VarBitRef {
                id: 7057,
                transmog: false,
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackassign should preserve push-then-varbit-pop layout");

        assert_eq!(compiled.code[0].command, "push_constant_string");
        assert!(matches!(compiled.code[0].operand, Operand::Int(20)));
        assert_eq!(compiled.code[1].command, "push_constant_string");
        assert!(matches!(compiled.code[1].operand, Operand::Int(40)));
        assert_eq!(compiled.code[2].command, "pop_varbit");
        assert!(matches!(
            &compiled.code[2].operand,
            Operand::VarBitRef(varbit) if varbit.id == 7058
        ));
        assert_eq!(compiled.code[3].command, "pop_varbit");
        assert!(matches!(
            &compiled.code[3].operand,
            Operand::VarBitRef(varbit) if varbit.id == 7057
        ));
    }

    #[test]
    fn stackassign_call_lowers_values_before_var_stores() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call("stackassign_1", vec![string("varclient_6908"), number(-1)]),
            }],
        );
        let mut ctx = context(947, &["push_constant_string", "pop_var"]);
        ctx.var_refs_by_name.insert(
            "varclient_6908".to_string(),
            VarRef {
                domain: VarDomain::Client,
                id: 6908,
                transmog: false,
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackassign should preserve push-then-var-pop layout");

        assert_eq!(compiled.code[0].command, "push_constant_string");
        assert!(matches!(compiled.code[0].operand, Operand::Int(-1)));
        assert_eq!(compiled.code[1].command, "pop_var");
        assert!(matches!(
            &compiled.code[1].operand,
            Operand::VarRef(var_ref)
                if var_ref.domain == VarDomain::Client && var_ref.id == 6908
        ));
    }

    #[test]
    fn define_array_call_lowers_size_then_array_operand() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call("define_array_65536", vec![number(4)]),
            }],
        );
        let ctx = context(947, &["push_constant_string", "define_array"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("define_array_N should lower with array operand");

        assert_eq!(compiled.code[1].command, "define_array");
        assert!(matches!(compiled.code[1].operand, Operand::Array(65536)));
    }

    #[test]
    fn array_sort_generic_call_lowers_to_opcode() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call("arraysort", vec![number(4), number(0), number(1)]),
            }],
        );
        let ctx = context(947, &["push_constant_string", "array_sort"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("arraysort should resolve array_sort opcode");

        assert_eq!(compiled.code[3].command, "array_sort");
    }

    #[test]
    fn generic_opcode_with_mode_lowers_trailing_argument_to_operand() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Return {
                value: Some(call(
                    "invtotalWithMode",
                    vec![number(93), identifier("local_int_0"), number(1)],
                )),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "push_int_local",
                "inv_total",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("generic WithMode suffix should lower trailing argument to opcode operand");

        assert_eq!(compiled.code[2].command, "inv_total");
        assert!(matches!(compiled.code[2].operand, Operand::Byte(1)));
    }

    #[test]
    fn direct_goto_switch_lowers_cases_to_original_targets() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::Switch {
                    expr: number(2),
                    cases: vec![
                        SwitchCaseStmt {
                            value: 0,
                            body: vec![StructuredStmt::Goto { target: 3 }],
                            fallthrough: false,
                            break_after: true,
                        },
                        SwitchCaseStmt {
                            value: 1,
                            body: vec![StructuredStmt::Goto { target: 10 }],
                            fallthrough: false,
                            break_after: true,
                        },
                    ],
                    default_body: None,
                },
                StructuredStmt::Goto { target: 20 },
                StructuredStmt::Label { target: 3 },
                StructuredStmt::Return { value: None },
                StructuredStmt::Label { target: 10 },
                StructuredStmt::Return { value: None },
                StructuredStmt::Label { target: 20 },
                StructuredStmt::Return { value: None },
            ],
        );
        let ctx = context(947, &["push_constant_string", "switch", "branch", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("direct-goto switch should lower without trampoline case bodies");

        let Operand::Switch(cases) = &compiled.code[1].operand else {
            panic!("expected switch operand");
        };
        assert_eq!(cases[0].target, 3);
        assert_eq!(cases[1].target, 4);
        assert_eq!(compiled.code[2].command, "branch");
    }

    #[test]
    fn switch_case_terminating_body_omits_dead_break_branch() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Switch {
                expr: number(1),
                cases: vec![SwitchCaseStmt {
                    value: 7,
                    body: vec![StructuredStmt::Return { value: None }],
                    fallthrough: false,
                    break_after: false,
                }],
                default_body: None,
            }],
        );
        let ctx = context(947, &["push_constant_string", "switch", "branch", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("terminating switch case should not emit a dead branch");

        assert_eq!(compiled.code.len(), 4);
        assert_eq!(compiled.code[3].command, "return");
    }

    #[test]
    fn switch_case_explicit_break_preserves_dead_branch_after_return() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Switch {
                expr: number(1),
                cases: vec![SwitchCaseStmt {
                    value: 7,
                    body: vec![StructuredStmt::Return { value: None }],
                    fallthrough: false,
                    break_after: true,
                }],
                default_body: None,
            }],
        );
        let ctx = context(947, &["push_constant_string", "switch", "branch", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("explicit switch break should preserve dead branch");

        assert_eq!(compiled.code.len(), 5);
        assert_eq!(compiled.code[3].command, "return");
        assert_eq!(compiled.code[4].command, "branch");
    }

    #[test]
    fn empty_switch_case_falls_through_to_next_case_body() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Switch {
                expr: number(1),
                cases: vec![
                    SwitchCaseStmt {
                        value: 0,
                        body: Vec::new(),
                        fallthrough: true,
                        break_after: false,
                    },
                    SwitchCaseStmt {
                        value: 1,
                        body: vec![StructuredStmt::Return {
                            value: Some(number(7)),
                        }],
                        fallthrough: false,
                        break_after: false,
                    },
                ],
                default_body: None,
            }],
        );
        let ctx = context(947, &["push_constant_string", "switch", "branch", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("empty switch case should fall through to next case body");

        let Operand::Switch(cases) = &compiled.code[1].operand else {
            panic!("expected switch operand");
        };
        assert_eq!(cases[0].target, cases[1].target);
        assert_eq!(compiled.code[4].command, "return");
    }

    #[test]
    fn switch_final_case_omits_dead_break_branch() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![
                StructuredStmt::Switch {
                    expr: number(1),
                    cases: vec![SwitchCaseStmt {
                        value: 7,
                        body: vec![StructuredStmt::Assignment {
                            target: AssignmentTarget::Identifier("local_int_0".to_string()),
                            value: number(42),
                        }],
                        fallthrough: false,
                        break_after: false,
                    }],
                    default_body: None,
                },
                StructuredStmt::Return { value: None },
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "switch",
                "branch",
                "pop_int_local",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("final switch case should fall through to switch end");

        assert_eq!(compiled.code.len(), 6);
        assert_eq!(compiled.code[4].command, "pop_int_local");
        assert_eq!(compiled.code[5].command, "return");
    }

    #[test]
    fn switch_default_lowers_before_case_bodies() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![
                StructuredStmt::Switch {
                    expr: number(1),
                    cases: vec![SwitchCaseStmt {
                        value: 7,
                        body: vec![StructuredStmt::Assignment {
                            target: AssignmentTarget::Identifier("local_int_0".to_string()),
                            value: number(42),
                        }],
                        fallthrough: false,
                        break_after: false,
                    }],
                    default_body: Some(vec![StructuredStmt::Expr {
                        expr: call("camreset", Vec::new()),
                    }]),
                },
                StructuredStmt::Return { value: None },
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "switch",
                "cam_reset",
                "branch",
                "pop_int_local",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("switch default should lower to fallthrough body after switch");

        assert_eq!(compiled.code[2].command, "cam_reset");
        assert_eq!(compiled.code[3].command, "branch");
        assert_eq!(compiled.code[5].command, "pop_int_local");
        assert_eq!(compiled.code[6].command, "return");
    }

    #[test]
    fn while_terminating_body_omits_dead_backedge() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::While {
                body: vec![StructuredStmt::If {
                    condition: number(1),
                    then_body: vec![StructuredStmt::Continue],
                    else_body: Some(vec![StructuredStmt::Break]),
                }],
            }],
        );
        let ctx = context(947, &["push_constant_string", "branch_if_true", "branch"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("terminating while body should lower without dead backedge");

        assert_eq!(
            compiled
                .code
                .iter()
                .filter(|instruction| instruction.command == "branch")
                .count(),
            1
        );
        assert_eq!(
            compiled
                .code
                .iter()
                .filter(|instruction| instruction.command == "branch_if_true")
                .count(),
            1
        );
    }

    #[test]
    fn if_else_direct_goto_lowers_false_branch_to_target_label() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::If {
                    condition: number(1),
                    then_body: vec![StructuredStmt::Return { value: None }],
                    else_body: Some(vec![StructuredStmt::Goto { target: 10 }]),
                },
                StructuredStmt::Label { target: 10 },
                StructuredStmt::Return { value: None },
            ],
        );
        let ctx = context(
            947,
            &["push_constant_string", "branch_if_true", "branch", "return"],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("direct goto else arm should lower to the target label");

        assert!(matches!(compiled.code[2].operand, Operand::Branch(4)));
    }

    #[test]
    fn not_equals_condition_lowers_to_branch_not() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::If {
                condition: Expression::BinaryOperation(BinaryOperation {
                    op: BinaryOp::Ne,
                    left: Box::new(Expression::Identifier(Identifier {
                        name: "local_int_0".to_string(),
                    })),
                    right: Box::new(number(-1)),
                }),
                then_body: vec![StructuredStmt::Return { value: None }],
                else_body: Some(vec![StructuredStmt::Return { value: None }]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_int_local",
                "push_constant_string",
                "branch_not",
                "branch",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("not-equals condition should lower to branch_not");

        assert_eq!(compiled.code[2].command, "branch_not");
    }

    #[test]
    fn bigint_condition_lowers_to_long_branch() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_long_0".to_string(),
                type_annotation: TypeAnnotation::BigInt,
            }],
            vec![StructuredStmt::If {
                condition: Expression::BinaryOperation(BinaryOperation {
                    op: BinaryOp::Eq,
                    left: Box::new(Expression::Identifier(Identifier {
                        name: "local_long_0".to_string(),
                    })),
                    right: Box::new(call("longconst", vec![bigint(1)])),
                }),
                then_body: vec![StructuredStmt::Return { value: None }],
                else_body: Some(vec![StructuredStmt::Return { value: None }]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_long_local",
                "push_constant_string",
                "long_branch_equals",
                "branch",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("bigint equality should lower to long_branch_equals");

        assert_eq!(compiled.code[2].command, "long_branch_equals");
    }

    #[test]
    fn stack_return_lowers_all_values_before_return() {
        let script = script_with_body(
            vec![
                LocalVariable {
                    index: 1,
                    name: "local_int_1".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
                LocalVariable {
                    index: 2,
                    name: "local_int_2".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
            ],
            vec![StructuredStmt::Return {
                value: Some(call(
                    "stack",
                    vec![identifier("local_int_1"), identifier("local_int_2")],
                )),
            }],
        );
        let ctx = context(947, &["push_int_local", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stack return should lower all values in order");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(commands, vec!["push_int_local", "push_int_local", "return"]);
        assert!(matches!(compiled.code[0].operand, Operand::Local(1)));
        assert!(matches!(compiled.code[1].operand, Operand::Local(2)));
    }

    #[test]
    fn stack_return_coalesces_multi_result_prefix() {
        let size = call("viewportgeteffectivesize", Vec::new());
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Return {
                value: Some(call(
                    "stack",
                    vec![
                        property_access(size.clone(), "width"),
                        property_access(size, "height"),
                        number(820),
                    ],
                )),
            }],
        );
        let ctx = context(
            947,
            &[
                "viewport_geteffectivesize",
                "push_constant_string",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stack return should coalesce multi-result prefix");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "viewport_geteffectivesize",
                "push_constant_string",
                "return"
            ]
        );
    }

    #[test]
    fn pop_assignment_lowers_to_local_store_after_multi_return_call() {
        let script = script_with_body(
            vec![
                LocalVariable {
                    index: 0,
                    name: "local_int_0".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
                LocalVariable {
                    index: 1,
                    name: "local_int_1".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
            ],
            vec![
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_int_1".to_string()),
                    value: call("worldmapgetdisplaycoord", vec![number(123)]),
                },
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_int_0".to_string()),
                    value: call("pop", Vec::new()),
                },
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "worldmap_getdisplaycoord",
                "pop_int_local",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("pop() assignment should lower to a local store");

        assert_eq!(compiled.code[1].command, "worldmap_getdisplaycoord");
        assert_eq!(compiled.code[2].command, "pop_int_local");
        assert_eq!(compiled.code[3].command, "pop_int_local");
    }

    #[test]
    fn pop_argument_lowers_as_existing_stack_value() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call(
                    "worldlistsort",
                    vec![number(1), call("pop", Vec::new()), number(2), number(3)],
                ),
            }],
        );
        let ctx = context(947, &["push_constant_string", "worldlist_sort"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("pop() call argument should lower as existing stack value");

        assert_eq!(compiled.code.len(), 4);
        assert_eq!(compiled.code[3].command, "worldlist_sort");
    }

    #[test]
    fn script_call_argument_call_result_skips_concrete_stack_type_check() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call("callee", vec![call("producer", Vec::new())]),
            }],
        );
        let mut ctx = context(947, &["gosub_with_params"]);
        insert_script(
            &mut ctx,
            100,
            "callee",
            ScriptSignature {
                arg_count_int: 0,
                arg_count_obj: 1,
                arg_count_long: 0,
                return_count_int: 0,
                return_count_obj: 0,
                return_count_long: 0,
                return_type: "void".to_string(),
            },
        );
        insert_script(
            &mut ctx,
            200,
            "producer",
            ScriptSignature {
                arg_count_int: 0,
                arg_count_obj: 0,
                arg_count_long: 0,
                return_count_int: 1,
                return_count_obj: 0,
                return_count_long: 0,
                return_type: "number".to_string(),
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("nested call result type should stay ambiguous for script arity");

        assert_eq!(compiled.code[0].command, "gosub_with_params");
        assert_eq!(compiled.code[1].command, "gosub_with_params");
    }

    #[test]
    fn multi_result_script_call_arguments_lower_single_gosub() {
        let producer = call("producer", vec![identifier("local_int_0")]);
        let script = script_with_body(
            vec![
                LocalVariable {
                    index: 0,
                    name: "local_int_0".to_string(),
                    type_annotation: TypeAnnotation::Number,
                },
                LocalVariable {
                    index: 0,
                    name: "local_obj_0".to_string(),
                    type_annotation: TypeAnnotation::String,
                },
            ],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_obj_0".to_string()),
                value: call("consumer", vec![producer, number(1), number(0)]),
            }],
        );
        let mut ctx = context(
            947,
            &[
                "push_int_local",
                "push_constant_string",
                "gosub_with_params",
                "pop_string_local",
            ],
        );
        insert_script(
            &mut ctx,
            100,
            "consumer",
            ScriptSignature {
                arg_count_int: 5,
                arg_count_obj: 0,
                arg_count_long: 0,
                return_count_int: 0,
                return_count_obj: 1,
                return_count_long: 0,
                return_type: "string".to_string(),
            },
        );
        insert_script(
            &mut ctx,
            200,
            "producer",
            ScriptSignature {
                arg_count_int: 1,
                arg_count_obj: 0,
                arg_count_long: 0,
                return_count_int: 3,
                return_count_obj: 0,
                return_count_long: 0,
                return_type: "number".to_string(),
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("multi-result script arguments should lower as one producer gosub");
        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            commands,
            vec![
                "push_int_local",
                "gosub_with_params",
                "push_constant_string",
                "push_constant_string",
                "gosub_with_params",
                "pop_string_local",
            ]
        );
        assert!(matches!(
            &compiled.code[1].operand,
            Operand::Script(script) if *script == 200
        ));
        assert!(matches!(
            &compiled.code[4].operand,
            Operand::Script(script) if *script == 100
        ));
    }

    #[test]
    fn script_call_local_argument_skips_concrete_stack_type_check() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Expr {
                expr: call("callee", vec![identifier("local_int_0")]),
            }],
        );
        let mut ctx = context(947, &["push_int_local", "gosub_with_params"]);
        insert_script(
            &mut ctx,
            100,
            "callee",
            ScriptSignature {
                arg_count_int: 0,
                arg_count_obj: 1,
                arg_count_long: 0,
                return_count_int: 0,
                return_count_obj: 0,
                return_count_long: 0,
                return_type: "void".to_string(),
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("local argument type should stay ambiguous for script arity");

        assert_eq!(compiled.code[0].command, "push_int_local");
        assert_eq!(compiled.code[1].command, "gosub_with_params");
    }

    #[test]
    fn subtraction_uses_legacy_910_opcode_when_sub_is_absent() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_int_0".to_string()),
                value: Expression::BinaryOperation(BinaryOperation {
                    op: BinaryOp::Sub,
                    left: Box::new(number(10)),
                    right: Box::new(number(3)),
                }),
            }],
        );
        let ctx = context(
            910,
            &[
                "push_constant_string",
                "quickchat_dynamic_command_add",
                "pop_int_local",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(910), &ctx)
            .expect("910 subtraction should lower to legacy opcode");

        assert_eq!(compiled.code[2].command, "quickchat_dynamic_command_add");
    }

    #[test]
    fn nested_logical_expr_lowers_to_bitwise_opcode() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_int_0".to_string()),
                value: Expression::BinaryOperation(BinaryOperation {
                    op: BinaryOp::And,
                    left: Box::new(number(6)),
                    right: Box::new(number(1)),
                }),
            }],
        );
        let ctx = context(947, &["push_constant_string", "and", "pop_int_local"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("nested logical and should lower as bitwise opcode");

        assert_eq!(compiled.code[2].command, "and");
    }

    #[test]
    fn unimported_command_name_lowers_as_opcode() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call(
                    "openurlraw",
                    vec![string("https://example.invalid"), number(0)],
                ),
            }],
        );
        let ctx = context(947, &["push_constant_string", "openurlraw"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("unimported opcode name should lower as command");

        assert_eq!(compiled.code[2].command, "openurlraw");
    }

    #[test]
    fn indexed_multi_result_assignments_lower_single_command() {
        let locals = (0..4)
            .map(|index| LocalVariable {
                index,
                name: format!("local_int_{index}"),
                type_annotation: TypeAnnotation::Number,
            })
            .collect::<Vec<_>>();
        let body = (0..4)
            .rev()
            .map(|index| StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier(format!("local_int_{index}")),
                value: array_access(call("windowgetinsets", Vec::new()), number(index)),
            })
            .collect::<Vec<_>>();
        let script = script_with_body(locals, body);
        let ctx = context(947, &["window_getinsets", "pop_int_local"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("window_getinsets result group should lower once");

        assert_eq!(compiled.code[0].command, "window_getinsets");
        assert_eq!(
            compiled
                .code
                .iter()
                .filter(|instruction| instruction.command == "window_getinsets")
                .count(),
            1
        );
    }

    #[test]
    fn array_assignment_lowers_index_before_value() {
        let locals = (0..3)
            .map(|index| LocalVariable {
                index,
                name: format!("local_int_{index}"),
                type_annotation: TypeAnnotation::Number,
            })
            .collect::<Vec<_>>();
        let script = script_with_body(
            locals,
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::ArrayAccess {
                    array: "array_0".to_string(),
                    index: identifier("local_int_1"),
                },
                value: array_access(identifier("array_0"), identifier("local_int_2")),
            }],
        );
        let ctx = context(947, &["push_int_local", "push_array_int", "pop_array_int"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("array assignment should lower in CS2 stack order");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_int_local",
                "push_int_local",
                "push_array_int",
                "pop_array_int"
            ]
        );
        assert!(matches!(compiled.code[0].operand, Operand::Local(1)));
        assert!(matches!(compiled.code[1].operand, Operand::Local(2)));
    }

    #[test]
    fn array_self_update_leave_index_pseudo_reuses_index_on_stack() {
        let locals = (0..2)
            .map(|index| LocalVariable {
                index,
                name: format!("local_int_{index}"),
                type_annotation: TypeAnnotation::Number,
            })
            .collect::<Vec<_>>();
        let index = identifier("local_int_0");
        let value = Expression::BinaryOperation(BinaryOperation {
            op: BinaryOp::Add,
            left: Box::new(call("push_array_int_leave_index_on_stack_2", vec![index])),
            right: Box::new(identifier("local_int_1")),
        });
        let script = script_with_body(
            locals,
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::ArrayAccess {
                    array: "array_2".to_string(),
                    index: call("pop", vec![]),
                },
                value,
            }],
        );
        let ctx = context(
            947,
            &[
                "push_int_local",
                "push_array_int_leave_index_on_stack",
                "add",
                "pop_array_int",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("array self update should reuse index left on stack");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_int_local",
                "push_array_int_leave_index_on_stack",
                "push_int_local",
                "add",
                "pop_array_int"
            ]
        );
    }

    #[test]
    fn array_assignment_accepts_multi_result_command_value() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::ArrayAccess {
                    array: "array_0".to_string(),
                    index: number(0),
                },
                value: ui_call("Param", vec![number(4261)]),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_param", "pop_array_int"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("array assignment should allow command values with multi stack effects");

        assert_eq!(compiled.code[2].command, "cc_param");
        assert_eq!(compiled.code[3].command, "pop_array_int");
    }

    #[test]
    fn indexed_minimenu_target_assignments_lower_single_command() {
        let locals = vec![
            LocalVariable {
                index: 3,
                name: "local_int_3".to_string(),
                type_annotation: TypeAnnotation::Number,
            },
            LocalVariable {
                index: 3,
                name: "local_obj_3".to_string(),
                type_annotation: TypeAnnotation::String,
            },
            LocalVariable {
                index: 4,
                name: "local_obj_4".to_string(),
                type_annotation: TypeAnnotation::String,
            },
        ];
        let script = script_with_body(
            locals,
            vec![
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_obj_3".to_string()),
                    value: array_access(call("getminimenutarget", Vec::new()), number(2)),
                },
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_obj_4".to_string()),
                    value: array_access(call("getminimenutarget", Vec::new()), number(1)),
                },
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_int_3".to_string()),
                    value: array_access(call("getminimenutarget", Vec::new()), number(0)),
                },
            ],
        );
        let ctx = context(
            947,
            &["get_minimenu_target", "pop_int_local", "pop_string_local"],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("get_minimenu_target result group should lower once");

        assert_eq!(compiled.code[0].command, "get_minimenu_target");
        assert_eq!(
            compiled
                .code
                .iter()
                .filter(|instruction| instruction.command == "get_minimenu_target")
                .count(),
            1
        );
    }

    #[test]
    fn multi_result_command_arguments_lower_single_producer_command() {
        let locals = vec![LocalVariable {
            index: 1,
            name: "local_int_1".to_string(),
            type_annotation: TypeAnnotation::Number,
        }];
        let producer = call("fullscreengetmode", vec![identifier("local_int_1")]);
        let script = script_with_body(
            locals,
            vec![StructuredStmt::Expr {
                expr: call(
                    "fullscreenenter",
                    vec![
                        property_access(producer.clone(), "width"),
                        property_access(producer, "height"),
                    ],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_int_local",
                "fullscreen_getmode",
                "fullscreen_enter",
                "pop_int_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("multi-result arguments should lower as one producer command");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_int_local",
                "fullscreen_getmode",
                "fullscreen_enter",
                "pop_int_discard"
            ]
        );
    }

    #[test]
    fn named_multi_result_assignments_lower_single_command() {
        let locals = vec![
            LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            },
            LocalVariable {
                index: 1,
                name: "local_int_1".to_string(),
                type_annotation: TypeAnnotation::Number,
            },
            LocalVariable {
                index: 2,
                name: "local_int_2".to_string(),
                type_annotation: TypeAnnotation::Number,
            },
            LocalVariable {
                index: 0,
                name: "local_obj_0".to_string(),
                type_annotation: TypeAnnotation::String,
            },
            LocalVariable {
                index: 1,
                name: "local_obj_1".to_string(),
                type_annotation: TypeAnnotation::String,
            },
            LocalVariable {
                index: 2,
                name: "local_obj_2".to_string(),
                type_annotation: TypeAnnotation::String,
            },
            LocalVariable {
                index: 3,
                name: "local_int_3".to_string(),
                type_annotation: TypeAnnotation::Number,
            },
        ];
        let call_expr = call("worldlistspecific", vec![number(301)]);
        let script = script_with_body(
            locals,
            vec![
                multi_result_assignment("local_obj_2", &call_expr, "host"),
                multi_result_assignment("local_int_3", &call_expr, "ping"),
                multi_result_assignment("local_int_2", &call_expr, "players"),
                multi_result_assignment("local_obj_1", &call_expr, "countryName"),
                multi_result_assignment("local_int_1", &call_expr, "countryId"),
                multi_result_assignment("local_obj_0", &call_expr, "activity"),
                multi_result_assignment("local_int_0", &call_expr, "flags"),
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "worldlist_specific",
                "pop_int_local",
                "pop_string_local",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("worldlist_specific result group should lower once");

        assert_eq!(compiled.code[1].command, "worldlist_specific");
        assert_eq!(
            compiled
                .code
                .iter()
                .filter(|instruction| instruction.command == "worldlist_specific")
                .count(),
            1
        );
    }

    #[test]
    fn generic_ui_method_restores_underscored_opcode_by_arg_count() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call("SetobjectNonum", vec![number(15660), number(0)]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_setobject_nonum",
                "if_setobject_nonum",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("SetobjectNonum should resolve cc form");

        assert_eq!(compiled.code[2].command, "cc_setobject_nonum");
    }

    #[test]
    fn interface_option_method_lowers_full_payload_to_if_opcode() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call("Setop", vec![number(1), string(""), number(83_230_736)]),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_setop", "if_setop"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("Setop with component payload should resolve if form");

        assert_eq!(compiled.code[3].command, "if_setop");
    }

    #[test]
    fn interface_createchild_lowers_by_full_payload_arity() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Createchild",
                    vec![number(10), number(1), number(2), number(3)],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "push_int_local",
                "cc_createchild",
                "if_createchild",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("Createchild with component payload should resolve if form");

        assert_eq!(compiled.code[4].command, "if_createchild");
    }

    #[test]
    fn generic_ui_getter_restores_underscored_opcode() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_int_0".to_string()),
                value: ui_call("GetmodelangleX", Vec::new()),
            }],
        );
        let ctx = context(947, &["cc_getmodelangle_x", "pop_int_local"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("GetmodelangleX should resolve cc getter");

        assert_eq!(compiled.code[0].command, "cc_getmodelangle_x");
    }

    #[test]
    fn ui_find_preserves_nonzero_operand_mode() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::Expr {
                    expr: ui_call("find", vec![number(100), number(7), number(1)]),
                },
                StructuredStmt::Expr {
                    expr: ui_call("findInterface", vec![number(200), number(1)]),
                },
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_find",
                "if_find",
                "pop_int_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("find mode should lower to opcode byte operand");

        assert_eq!(compiled.code[2].command, "cc_find");
        assert!(matches!(compiled.code[2].operand, Operand::Byte(1)));
        assert_eq!(compiled.code[5].command, "if_find");
        assert!(matches!(compiled.code[5].operand, Operand::Byte(1)));
    }

    #[test]
    fn ui_create_preserves_nonzero_operand_mode() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call("create", vec![number(100), number(5), number(7), number(1)]),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_create"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("create mode should lower to opcode byte operand");

        assert_eq!(compiled.code[3].command, "cc_create");
        assert!(matches!(compiled.code[3].operand, Operand::Byte(1)));
    }

    #[test]
    fn ui_with_mode_suffix_lowers_nonzero_operand() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call("setTextWithMode", vec![string("text"), number(1)]),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_settext"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("WithMode suffix should lower trailing mode to opcode operand");

        assert_eq!(compiled.code[1].command, "cc_settext");
        assert!(matches!(compiled.code[1].operand, Operand::Byte(1)));
    }

    #[test]
    fn ui_getter_with_mode_suffix_keeps_current_component_form() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_int_0".to_string()),
                value: ui_call("GetwidthWithMode", vec![number(1)]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_getwidth",
                "if_getwidth",
                "pop_int_local",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("WithMode getter should not treat mode as IF component argument");

        assert_eq!(compiled.code[0].command, "cc_getwidth");
        assert!(matches!(compiled.code[0].operand, Operand::Byte(1)));
    }

    #[test]
    fn ui_getter_resolves_current_component_form_by_arg_count() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_obj_0".to_string(),
                type_annotation: TypeAnnotation::String,
            }],
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_obj_0".to_string()),
                value: ui_call("Getop", vec![number(1)]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_getop",
                "if_getop",
                "pop_string_local",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("getter should choose cc form when arg count matches");

        assert_eq!(compiled.code[1].command, "cc_getop");
    }

    #[test]
    fn varbit_identifier_suffix_lowers_transmog_operand() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: Expression::Identifier(Identifier {
                    name: "varbit_45522_transmog".to_string(),
                }),
            }],
        );
        let mut ctx = context(947, &["push_varbit", "pop_int_discard"]);
        ctx.varbit_refs_by_name.insert(
            "varbit_45522_transmog".to_string(),
            VarBitRef {
                id: 45522,
                transmog: true,
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("transmog varbit identifier should lower");

        assert_eq!(compiled.code[0].command, "push_varbit");
        assert!(matches!(
            &compiled.code[0].operand,
            Operand::VarBitRef(varbit) if varbit.id == 45522 && varbit.transmog
        ));
    }

    #[test]
    fn var_identifier_suffix_lowers_transmog_operand() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: Expression::Identifier(Identifier {
                    name: "varplayer_12655_transmog".to_string(),
                }),
            }],
        );
        let mut ctx = context(947, &["push_var", "pop_int_discard"]);
        ctx.var_refs_by_name.insert(
            "varplayer_12655_transmog".to_string(),
            VarRef {
                domain: VarDomain::Player,
                id: 12655,
                transmog: true,
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("transmog var identifier should lower");

        assert_eq!(compiled.code[0].command, "push_var");
        assert!(matches!(
            &compiled.code[0].operand,
            Operand::VarRef(var_ref)
                if var_ref.id == 12655 && var_ref.domain == VarDomain::Player && var_ref.transmog
        ));
    }

    #[test]
    fn callback_watcher_varplayerint_prefix_lowers_to_constant_id() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setonvartransmit",
                    vec![callback(
                        "script123",
                        Vec::new(),
                        vec!["varplayerint_3814"],
                        "Y",
                    )],
                ),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_setonvartransmit"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("varplayerint watcher should lower to constant id");

        assert!(matches!(compiled.code[1].operand, Operand::Int(3814)));
        assert!(matches!(compiled.code[2].operand, Operand::Int(1)));
    }

    #[test]
    fn setoninvtransmit_varplayer_watcher_lowers_to_var_load() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setoninvtransmit",
                    vec![
                        callback("script20702", Vec::new(), vec!["varplayer_12696"], "Y"),
                        number(12_058_639),
                    ],
                ),
            }],
        );
        let mut ctx = context(
            947,
            &["push_constant_string", "push_var", "if_setoninvtransmit"],
        );
        ctx.var_refs_by_name.insert(
            "varplayer_12696".to_string(),
            VarRef {
                domain: VarDomain::Player,
                id: 12696,
                transmog: false,
            },
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("setoninvtransmit varplayer watcher should lower to push_var");

        assert_eq!(compiled.code[1].command, "push_var");
        assert!(matches!(
            &compiled.code[1].operand,
            Operand::VarRef(var_ref)
                if var_ref.domain == VarDomain::Player && var_ref.id == 12696
        ));
    }

    #[test]
    fn callback_watcher_local_identifier_lowers_to_local_load() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setonvartransmit",
                    vec![callback("script123", Vec::new(), vec!["local_int_0"], "Y")],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "push_int_local",
                "cc_setonvartransmit",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("local watcher should lower to local load");

        assert_eq!(compiled.code[1].command, "push_int_local");
        assert!(matches!(compiled.code[1].operand, Operand::Local(0)));
    }

    #[test]
    fn callback_target_enum_constant_lowers_to_script_id() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setontimer",
                    vec![callback("Enum_1.SCRIPT_TARGET", Vec::new(), Vec::new(), "")],
                ),
            }],
        );
        let mut ctx = context(947, &["push_constant_string", "cc_setontimer"]);
        ctx.enum_values_by_name
            .insert("Enum_1.SCRIPT_TARGET".to_string(), 4938);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("enum callback target should lower to script id");

        assert!(matches!(compiled.code[0].operand, Operand::Int(4938)));
    }

    #[test]
    fn callback_target_local_identifier_lowers_to_local_load() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 2,
                name: "local_int_2".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setonmouserepeat",
                    vec![callback("local_int_2", vec![number(1)], Vec::new(), "i")],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_int_local",
                "push_constant_string",
                "cc_setonmouserepeat",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("local callback target should lower as dynamic script id");

        assert_eq!(compiled.code[0].command, "push_int_local");
        assert!(matches!(compiled.code[0].operand, Operand::Local(2)));
    }

    #[test]
    fn callback_target_pop_uses_existing_stack_value() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setonclick",
                    vec![callback("pop()", Vec::new(), Vec::new(), "")],
                ),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_setonclick"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("pop callback target should use existing stack value");

        assert_eq!(compiled.code[0].command, "push_constant_string");
        assert_eq!(compiled.code[1].command, "cc_setonclick");
    }

    #[test]
    fn raw_ui_hook_descriptor_lowers_without_callback_literal() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 0,
                name: "local_int_0".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "Setonvarcstrtransmit",
                    vec![string("Y"), identifier("local_int_0")],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "push_int_local",
                "if_setonvarcstrtransmit",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("raw hook descriptor should lower as fallback payload");

        assert_eq!(compiled.code[0].command, "push_constant_string");
        assert_eq!(compiled.code[1].command, "push_int_local");
        assert_eq!(compiled.code[2].command, "if_setonvarcstrtransmit");
    }

    #[test]
    fn generic_ui_value_command_lowers_statement_discard() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call("ListAddentry", vec![number(5), string("entry"), number(-1)]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_list_addentry",
                "pop_int_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("value-producing UI command should lower with discard");

        assert_eq!(compiled.code[3].command, "cc_list_addentry");
        assert_eq!(compiled.code[4].command, "pop_int_discard");
    }

    #[test]
    fn ui_list_addentry_component_constant_lowers_to_if_form() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "ListAddentry",
                    vec![number(5), string("entry"), number(56_492_035)],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_list_addentry",
                "if_list_addentry",
                "pop_int_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("component constant should select interface form");

        assert_eq!(compiled.code[3].command, "if_list_addentry");
        assert_eq!(compiled.code[4].command, "pop_int_discard");
    }

    #[test]
    fn typed_discard_call_lowers_without_extra_discard() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: call("popintdiscard", Vec::new()),
            }],
        );
        let ctx = context(947, &["pop_int_discard"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("typed discard should lower to discard opcode");

        assert_eq!(compiled.code.len(), 1);
        assert_eq!(compiled.code[0].command, "pop_int_discard");
    }

    #[test]
    fn param_getter_statement_discards_by_param_type() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::Expr {
                    expr: call("structparam", vec![number(123), number(3508)]),
                },
                StructuredStmt::Expr {
                    expr: call("structparam", vec![number(123), number(4085)]),
                },
            ],
        );
        let mut ctx = context(
            947,
            &[
                "push_constant_string",
                "struct_param",
                "pop_int_discard",
                "pop_string_discard",
            ],
        );
        ctx.string_param_ids.insert(4085);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("param getter statement should lower typed discard");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_constant_string",
                "push_constant_string",
                "struct_param",
                "pop_int_discard",
                "push_constant_string",
                "push_constant_string",
                "struct_param",
                "pop_string_discard",
            ]
        );
    }

    #[test]
    fn stackpush_then_lowers_values_before_statement() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::Expr {
                    expr: call(
                        "stackpush_then",
                        vec![number(0), call("autosetupsetultra", Vec::new())],
                    ),
                },
                StructuredStmt::Expr {
                    expr: call("popintdiscard", Vec::new()),
                },
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "autosetup_setultra",
                "pop_int_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackpush_then should preserve value before statement");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_constant_string",
                "autosetup_setultra",
                "pop_int_discard"
            ]
        );
    }

    #[test]
    fn stackpush_then_statement_leaves_value_for_followup_pop() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::Expr {
                    expr: call(
                        "stackpush_then",
                        vec![number(0), call("unknowncommand20", Vec::new())],
                    ),
                },
                StructuredStmt::Expr {
                    expr: call("popintdiscard", Vec::new()),
                },
            ],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "unknown_command_20",
                "pop_int_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackpush_then should leave the produced value on the VM stack");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_constant_string",
                "unknown_command_20",
                "pop_int_discard"
            ]
        );
    }

    #[test]
    fn stack_goto_lowers_values_before_branch() {
        let script = script_with_body(
            Vec::new(),
            vec![
                StructuredStmt::StackGoto {
                    target: 42,
                    values: vec![number(18), number(1)],
                },
                StructuredStmt::Label { target: 42 },
                StructuredStmt::Return { value: None },
            ],
        );
        let ctx = context(947, &["push_constant_string", "branch", "return"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stack goto should preserve pushes before branch");

        let commands = compiled
            .code
            .iter()
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_constant_string",
                "push_constant_string",
                "branch",
                "return"
            ]
        );
        assert!(matches!(compiled.code[2].operand, Operand::Branch(3)));
    }

    #[test]
    fn stackpush_then_groups_multi_result_values_across_stackassign() {
        let script = script_with_body(
            vec![LocalVariable {
                index: 3,
                name: "local_int_3".to_string(),
                type_annotation: TypeAnnotation::Number,
            }],
            vec![StructuredStmt::Expr {
                expr: call(
                    "stackpush_then",
                    vec![
                        array_access(call("windowgetinsets", Vec::new()), number(0)),
                        array_access(call("windowgetinsets", Vec::new()), number(1)),
                        array_access(call("windowgetinsets", Vec::new()), number(2)),
                        call(
                            "stackassign_1",
                            vec![
                                string("local_int_3"),
                                array_access(call("windowgetinsets", Vec::new()), number(3)),
                            ],
                        ),
                    ],
                ),
            }],
        );
        let ctx = context(947, &["window_getinsets", "pop_int_local"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackpush_then should group multi-result assignment values");

        assert_eq!(
            compiled
                .code
                .iter()
                .filter(|instruction| instruction.command == "window_getinsets")
                .count(),
            1
        );
        assert_eq!(compiled.code[1].command, "pop_int_local");
    }

    #[test]
    fn stackpush_then_value_lowers_values_before_condition() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::If {
                condition: Expression::BinaryOperation(BinaryOperation {
                    op: BinaryOp::Eq,
                    left: Box::new(call(
                        "stackpush_then",
                        vec![number(0), call("unknowncommand20", Vec::new())],
                    )),
                    right: Box::new(number(1)),
                }),
                then_body: vec![StructuredStmt::Return { value: None }],
                else_body: Some(vec![StructuredStmt::Return { value: None }]),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "unknown_command_20",
                "branch_equals",
                "branch",
                "return",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("stackpush_then should preserve delayed value before condition");

        let commands = compiled
            .code
            .iter()
            .take(4)
            .map(|instruction| instruction.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            vec![
                "push_constant_string",
                "unknown_command_20",
                "push_constant_string",
                "branch_equals",
            ]
        );
    }

    #[test]
    fn scriptqueue_add_lowers_callback_payload_and_discard() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "ScriptqueueAdd",
                    vec![
                        number(50),
                        callback(
                            "script19746",
                            vec![number(-2_147_483_645), number(-2_147_483_643)],
                            Vec::new(),
                            "ii",
                        ),
                    ],
                ),
            }],
        );
        let ctx = context(
            947,
            &[
                "push_constant_string",
                "cc_scriptqueue_add",
                "pop_long_discard",
            ],
        );

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("scriptqueue add should lower callback payload");

        assert_eq!(compiled.code[5].command, "cc_scriptqueue_add");
        assert_eq!(compiled.code[6].command, "pop_long_discard");
    }

    #[test]
    fn prefixed_seton_hook_lowers_by_sanitized_suffix() {
        let script = script_with_body(
            Vec::new(),
            vec![StructuredStmt::Expr {
                expr: ui_call(
                    "CrmviewSetonupdated",
                    vec![callback(
                        "script18466",
                        vec![number(-2_147_483_645), number(-2_147_483_647)],
                        Vec::new(),
                        "ii",
                    )],
                ),
            }],
        );
        let ctx = context(947, &["push_constant_string", "cc_crmview_setonupdated"]);

        let compiled = lower_structured_script(&script, &metadata(947), &ctx)
            .expect("prefixed seton hook should lower to matching cc command");

        assert_eq!(compiled.code[4].command, "cc_crmview_setonupdated");
    }

    fn script_with_body(locals: Vec<LocalVariable>, body: Vec<StructuredStmt>) -> StructuredScript {
        StructuredScript {
            script_id: ScriptId(0),
            raw_name: None,
            header_comments: Vec::new(),
            imports: Vec::new(),
            function_name: "script0".to_string(),
            arguments: Vec::new(),
            locals,
            arrays: Vec::new(),
            return_type: "void".to_string(),
            body,
        }
    }

    fn metadata(build: u32) -> ReversibleMetadata {
        ReversibleMetadata {
            format_version: 1,
            build,
            subbuild: 0,
            packed_id: 0,
            group_id: 0,
            file_id: 0,
            script_id: 0,
            export_name: "script0".to_string(),
            raw_name: None,
            editable_structured: true,
            structured_digest: String::new(),
            blocking_diagnostics: Vec::new(),
        }
    }

    fn context(build: u32, commands: &[&str]) -> ReverseCompileContext {
        ReverseCompileContext {
            build,
            script_catalog: ScriptCatalog::default(),
            script_signatures: HashMap::<ScriptId, ScriptSignature>::new(),
            var_refs_by_name: HashMap::new(),
            varbit_refs_by_name: HashMap::new(),
            string_param_ids: HashSet::new(),
            enum_values_by_name: HashMap::new(),
            component_ids_by_name: HashMap::new(),
            opcode_commands: commands
                .iter()
                .map(|command| (*command).to_string())
                .collect::<HashSet<_>>(),
        }
    }

    fn insert_script(
        ctx: &mut ReverseCompileContext,
        id: i32,
        export_name: &str,
        signature: ScriptSignature,
    ) {
        ctx.script_catalog.insert(ScriptMetadata {
            packed_id: ScriptId(id),
            group_id: ScriptGroupId(id),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: export_name.to_string(),
            export_name: export_name.to_string(),
            module_name: export_name.to_string(),
            signature: signature.clone(),
        });
        ctx.script_signatures.insert(ScriptId(id), signature);
    }

    fn call(name: &str, arguments: Vec<Expression>) -> Expression {
        Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: name.to_string(),
            })),
            arguments,
        })
    }

    fn ui_call(method: &str, arguments: Vec<Expression>) -> Expression {
        Expression::Call(CallExpr {
            callee: Box::new(Expression::PropertyAccess(PropertyAccess {
                object: Box::new(Expression::Identifier(Identifier {
                    name: "UI".to_string(),
                })),
                property: method.to_string(),
            })),
            arguments,
        })
    }

    fn identifier(name: &str) -> Expression {
        Expression::Identifier(Identifier {
            name: name.to_string(),
        })
    }

    fn array_access(array: Expression, index: Expression) -> Expression {
        Expression::ArrayAccess(ArrayAccess {
            array: Box::new(array),
            index: Box::new(index),
        })
    }

    fn property_access(object: Expression, property: &str) -> Expression {
        Expression::PropertyAccess(PropertyAccess {
            object: Box::new(object),
            property: property.to_string(),
        })
    }

    fn multi_result_assignment(
        target: &str,
        call_expr: &Expression,
        property: &str,
    ) -> StructuredStmt {
        StructuredStmt::Assignment {
            target: AssignmentTarget::Identifier(target.to_string()),
            value: property_access(call_expr.clone(), property),
        }
    }

    fn callback(
        script: &str,
        arguments: Vec<Expression>,
        watchers: Vec<&str>,
        raw_descriptor: &str,
    ) -> Expression {
        Expression::CallbackLiteral(CallbackLiteral {
            script: script.to_string(),
            script_id: None,
            raw_descriptor: raw_descriptor.to_string(),
            arguments,
            watchers: watchers
                .into_iter()
                .map(std::string::ToString::to_string)
                .collect(),
        })
    }

    fn number(value: i32) -> Expression {
        Expression::NumberLiteral(NumberLiteral { value })
    }

    fn bigint(value: i64) -> Expression {
        Expression::BigIntLiteral(BigIntLiteral { value })
    }

    fn string(value: &str) -> Expression {
        Expression::StringLiteral(StringLiteral {
            value: value.to_string(),
        })
    }
}
