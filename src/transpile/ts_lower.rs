use super::ast::{BinaryOp, Expression, ScriptId, TypeAnnotation, UnaryOp};
use super::reversible_format::ReversibleMetadata;
use super::structured::{
    AssignmentTarget, StructuredScript, StructuredStmt, parse_type_annotation,
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
    pub enum_values_by_name: HashMap<String, i32>,
    pub component_ids_by_name: HashMap<String, i32>,
    pub opcode_commands: HashSet<String>,
}

impl ReverseCompileContext {
    pub fn has_command(&self, command: &str) -> bool {
        self.opcode_commands.contains(command)
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
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, stmt: &StructuredStmt) -> Result<()> {
        match stmt {
            StructuredStmt::While { body } => self.lower_while(body),
            StructuredStmt::If {
                condition,
                then_body,
                else_body,
            } => self.lower_if(condition, then_body, else_body.as_deref()),
            StructuredStmt::Switch { expr, cases } => self.lower_switch(expr, cases),
            StructuredStmt::Assignment { target, value } => self.lower_assignment(target, value),
            StructuredStmt::Expr { expr } => {
                let kind = self.emit_expr(expr)?;
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
            StructuredStmt::Goto { .. } => {
                bail!("structured recompilation does not support goto statements")
            }
            StructuredStmt::Return { value } => {
                if let Some(value) = value {
                    self.emit_expr(value)?;
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
        self.emit_branch_to("branch", &continue_label);
        self.mark_label(&break_label);
        Ok(())
    }

    fn lower_if(
        &mut self,
        condition: &Expression,
        then_body: &[StructuredStmt],
        else_body: Option<&[StructuredStmt]>,
    ) -> Result<()> {
        let then_label = self.new_label("if_then");
        let end_label = self.new_label("if_end");
        let else_label = else_body
            .map(|_| self.new_label("if_else"))
            .unwrap_or_else(|| end_label.clone());

        self.emit_condition(condition, &then_label, &else_label)?;
        self.mark_label(&then_label);
        self.lower_stmts(then_body)?;
        if let Some(else_body) = else_body {
            self.emit_branch_to("branch", &end_label);
            self.mark_label(&else_label);
            self.lower_stmts(else_body)?;
        }
        self.mark_label(&end_label);
        Ok(())
    }

    fn lower_switch(&mut self, expr: &Expression, cases: &[super::SwitchCaseStmt]) -> Result<()> {
        self.emit_expr(expr)?;
        let case_labels = cases
            .iter()
            .map(|case| (case.value, self.new_label("switch_case")))
            .collect::<Vec<_>>();
        let index = self.instructions.len();
        self.instructions.push(Instruction {
            opcode: 0,
            command: "switch".to_string(),
            operand: Operand::Switch(Vec::new()),
        });
        self.switch_fixups.push((index, case_labels.clone()));
        let end_label = self.new_label("switch_end");
        self.emit_branch_to("branch", &end_label);
        for ((_, label), case) in case_labels.iter().zip(cases) {
            self.mark_label(label);
            self.lower_stmts(&case.body)?;
            self.emit_branch_to("branch", &end_label);
        }
        self.mark_label(&end_label);
        Ok(())
    }

    fn lower_assignment(&mut self, target: &AssignmentTarget, value: &Expression) -> Result<()> {
        let value_kind = self.emit_expr(value)?;
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
            AssignmentTarget::ArrayAccess { array, index } => {
                self.emit_expr(index)?;
                let Some(array_id) = array.strip_prefix("array_") else {
                    bail!("unsupported array target {array}");
                };
                let array_id = array_id.parse::<i32>()?;
                // Only integer arrays exist in the CS2 opcode set; there is no
                // pop_array_string. Reject string-array assignment cleanly rather
                // than emitting a phantom command.
                if value_kind == ValueKind::Object {
                    bail!(
                        "string arrays are not supported (no pop_array_string opcode): {array}[..]"
                    );
                }
                self.emit_instruction("pop_array_int", Operand::Array(array_id));
                Ok(())
            }
            AssignmentTarget::Opaque(target) => {
                bail!("opaque assignment target is not reversible: {target}")
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
                BinaryOp::Eq | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    self.emit_expr(&binary.left)?;
                    self.emit_expr(&binary.right)?;
                    let branch = match binary.op {
                        BinaryOp::Eq => "branch_equals",
                        BinaryOp::Lt => "branch_less_than",
                        BinaryOp::Le => "branch_less_than_or_equals",
                        BinaryOp::Gt => "branch_greater_than",
                        BinaryOp::Ge => "branch_greater_than_or_equals",
                        _ => unreachable!(),
                    };
                    self.emit_branch_to(branch, true_label);
                    self.emit_branch_to("branch", false_label);
                    Ok(())
                }
                BinaryOp::Ne => {
                    let skip_label = self.new_label("if_ne_skip");
                    self.emit_expr(&binary.left)?;
                    self.emit_expr(&binary.right)?;
                    self.emit_branch_to("branch_equals", &skip_label);
                    self.emit_branch_to("branch", true_label);
                    self.mark_label(&skip_label);
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
                // Int constants have two CS2 encodings: push_constant_int and
                // push_constant_string with an int discriminator. The corpus
                // predominantly uses the latter (measured +81 editable by the
                // byte gate), but the stack-effect validator (validate.rs) models
                // push_constant_string as a string push, so emitting it here
                // makes assemble-script's verifier flag a false StackUnderflow —
                // i.e. byte-gate and validator disagree. Stay on
                // push_constant_int until the validator models the int
                // discriminator (a prerequisite of the byte-fidelity work).
                self.emit_instruction("push_constant_int", Operand::Int(value.value));
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
                self.emit_instruction("push_constant_int", Operand::Int(i32::from(value.value)));
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
                self.emit_instruction("push_constant_string", Operand::Int(*value));
                return Ok(ValueKind::Int);
            }
            if object.name == "ComponentId" {
                let key = format!("{}.{}", object.name, access.property);
                let Some(value) = self.ctx.component_ids_by_name.get(&key).copied() else {
                    bail!("unknown component constant {key}");
                };
                self.emit_instruction("push_constant_int", Operand::Int(value));
                return Ok(ValueKind::Int);
            }
        }
        bail!("property access expressions are only supported for enum and component constants")
    }

    fn emit_call(&mut self, call: &super::CallExpr) -> Result<ValueKind> {
        if let Expression::Identifier(identifier) = &*call.callee
            && let Some(script_metadata) = self
                .ctx
                .script_catalog
                .resolve_export_name(&identifier.name)
        {
            let mut arg_kinds = Vec::with_capacity(call.arguments.len());
            for argument in &call.arguments {
                arg_kinds.push(self.emit_expr(argument)?);
            }
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

        bail!("unsupported call expression")
    }

    fn emit_ui_call(&mut self, method: &str, arguments: &[Expression]) -> Result<ValueKind> {
        match method {
            "create" => {
                let [parent, kind, id] = arguments else {
                    bail!("UI.create expects 3 arguments, got {}", arguments.len());
                };
                self.emit_expr(parent)?;
                self.emit_expr(kind)?;
                self.emit_expr(id)?;
                self.emit_instruction("cc_create", Operand::Byte(0));
                Ok(ValueKind::Void)
            }
            "delete" => {
                self.emit_instruction("cc_delete", Operand::Byte(0));
                Ok(ValueKind::Void)
            }
            "deleteAll" => {
                let [target] = arguments else {
                    bail!("UI.deleteAll expects 1 argument, got {}", arguments.len());
                };
                self.emit_expr(target)?;
                self.emit_instruction("cc_deleteall", Operand::Byte(0));
                Ok(ValueKind::Void)
            }
            "find" => {
                for argument in arguments {
                    self.emit_expr(argument)?;
                }
                let command = match arguments.len() {
                    1 => "if_find",
                    2 => "cc_find",
                    _ => bail!("UI.find expects 1 or 2 arguments"),
                };
                self.emit_instruction(command, Operand::Byte(0));
                Ok(ValueKind::Int)
            }
            "getText" => {
                let [component] = arguments else {
                    bail!("UI.getText expects 1 argument, got {}", arguments.len());
                };
                self.emit_expr(component)?;
                self.emit_instruction("if_gettext", Operand::Byte(0));
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
                self.emit_instruction(command, Operand::Byte(0));
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
                self.emit_instruction(command, Operand::Byte(0));
                Ok(ValueKind::Void)
            }
            method if method.starts_with("Seton") => self.emit_ui_hook_call(method, arguments),
            method => {
                // Generic inverse of the decompiler's `UI.<CamelCase(cc-suffix)>`
                // naming: map back to `cc_<lowercase>` when that opcode exists
                // for the build. Covers every single-word set-method (both the
                // decompiler's `Sethide`/`Settext` and hand-written `setHide`)
                // without a hardcoded list. Underscore-bearing opcodes keep
                // explicit mappings since the camelCase form is lossy.
                let generic = format!("cc_{}", method.to_ascii_lowercase());
                let command: String = if self.ctx.has_command(&generic) {
                    generic
                } else {
                    match method {
                        "setParam" => "cc_setparam".to_string(),
                        "setParamInt" => "cc_setparam_int".to_string(),
                        "setParamString" => "cc_setparam_string".to_string(),
                        _ => bail!("unsupported UI method {method}"),
                    }
                };
                for argument in arguments {
                    self.emit_expr(argument)?;
                }
                self.emit_instruction(&command, Operand::Byte(0));
                Ok(ValueKind::Void)
            }
        }
    }

    fn emit_ui_hook_call(&mut self, method: &str, arguments: &[Expression]) -> Result<ValueKind> {
        let (command_cc, command_if) = match method {
            "Setonclick" => ("cc_setonclick", "if_setonclick"),
            "Setonvartransmit" => ("cc_setonvartransmit", "if_setonvartransmit"),
            "Setonstocktransmit" => ("cc_setonstocktransmit", "if_setonstocktransmit"),
            "Setoninvtransmit" => ("cc_setoninvtransmit", "if_setoninvtransmit"),
            _ => bail!("unsupported UI hook method {method}"),
        };
        let (callback_expr, component_expr, command) = match arguments {
            [callback] => (callback, None, command_cc),
            [callback, component] => (callback, Some(component), command_if),
            _ => bail!("UI hook methods expect callback and optional component"),
        };
        let Expression::CallbackLiteral(callback) = callback_expr else {
            bail!("UI hook first argument must be callback literal");
        };

        let raw_id = if let Some(script_id) = callback.script_id {
            script_id
        } else if let Some(metadata) = self
            .ctx
            .script_catalog
            .resolve_export_name(&callback.script)
        {
            metadata.group_id.0
        } else if let Some(id) = callback.script.strip_prefix("script") {
            id.parse::<i32>()?
        } else {
            bail!("unknown callback target {}", callback.script);
        };
        self.emit_instruction("push_constant_int", Operand::Int(raw_id));
        for argument in &callback.arguments {
            self.emit_expr(argument)?;
        }
        if !callback.watchers.is_empty() {
            for watcher in &callback.watchers {
                self.emit_callback_watcher(watcher)?;
            }
            self.emit_instruction(
                "push_constant_int",
                Operand::Int(callback.watchers.len() as i32),
            );
        }
        self.emit_instruction(
            "push_constant_string",
            Operand::Str(callback.raw_descriptor.clone()),
        );
        if let Some(component_expr) = component_expr {
            self.emit_expr(component_expr)?;
        }
        self.emit_instruction(command, Operand::Byte(0));
        Ok(ValueKind::Void)
    }

    fn emit_callback_watcher(&mut self, watcher: &str) -> Result<()> {
        if let Some(var_ref) = self.ctx.var_refs_by_name.get(watcher) {
            self.emit_instruction("push_constant_int", Operand::Int(i32::from(var_ref.id)));
            return Ok(());
        }
        if let Some(varbit_ref) = self.ctx.varbit_refs_by_name.get(watcher) {
            self.emit_instruction("push_constant_int", Operand::Int(i32::from(varbit_ref.id)));
            return Ok(());
        }
        if let Some(id) = watcher.strip_prefix("inv_") {
            self.emit_instruction("push_constant_int", Operand::Int(id.parse::<i32>()?));
            return Ok(());
        }
        if let Some(id) = watcher.strip_prefix("stat_") {
            self.emit_instruction("push_constant_int", Operand::Int(id.parse::<i32>()?));
            return Ok(());
        }
        if let Some(id) = watcher.strip_prefix("varc_") {
            self.emit_instruction("push_constant_int", Operand::Int(id.parse::<i32>()?));
            return Ok(());
        }
        if let Some(id) = watcher.strip_prefix("varcstr_") {
            self.emit_instruction("push_constant_int", Operand::Int(id.parse::<i32>()?));
            return Ok(());
        }
        bail!("unsupported callback watcher {watcher}")
    }

    fn emit_binary_expr(&mut self, binary: &super::BinaryOperation) -> Result<ValueKind> {
        match binary.op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                self.emit_expr(&binary.left)?;
                self.emit_expr(&binary.right)?;
                let command = match binary.op {
                    BinaryOp::Add => "add",
                    BinaryOp::Sub => "sub",
                    BinaryOp::Mul => "multiply",
                    BinaryOp::Div => "divide",
                    BinaryOp::Mod => "modulo",
                    _ => unreachable!(),
                };
                self.emit_instruction(command, Operand::Byte(0));
                Ok(ValueKind::Int)
            }
            _ => bail!(
                "comparison/logical expressions are only supported in control-flow conditions"
            ),
        }
    }

    fn emit_unary_expr(&mut self, unary: &super::UnaryOperation) -> Result<ValueKind> {
        match unary.op {
            UnaryOp::Neg => match &*unary.operand {
                Expression::NumberLiteral(value) => {
                    self.emit_instruction("push_constant_int", Operand::Int(-value.value));
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

fn kind_from_return_type(value: &str) -> ValueKind {
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
