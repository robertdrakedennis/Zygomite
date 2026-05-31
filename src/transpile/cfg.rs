use super::ast::{BinaryOperation, CallExpr, Expression, Identifier, UnaryOp, UnaryOperation};
use super::expr_recovery::{ExprRecovery, RecoveredStmt};
use super::structured::{AssignmentTarget, StructuredStmt, stmt_terminates};
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

#[derive(Debug, Clone)]
pub struct Block {
    pub index: usize,
    pub start: usize,
    pub end: usize,
    pub statements: Vec<RecoveredStmt>,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
    pub is_loop_header: bool,
    pub loop_target: Option<usize>,
    pub is_conditional_branch: bool,
    pub branch_condition: Option<Expression>,
}

impl Block {
    pub fn new(index: usize, start: usize) -> Self {
        Self {
            index,
            start,
            end: start,
            statements: Vec::new(),
            successors: Vec::new(),
            predecessors: Vec::new(),
            is_loop_header: false,
            loop_target: None,
            is_conditional_branch: false,
            branch_condition: None,
        }
    }
}

pub struct CfgBuilder<'a> {
    instructions: &'a [super::ast::InstructionNode],
    recovered: Vec<Option<RecoveredStmt>>,
}

impl<'a> CfgBuilder<'a> {
    pub fn new<VarHasher, ComponentHasher, EnumHasher, SignatureHasher>(
        instructions: &'a [super::ast::InstructionNode],
        var_names: &HashMap<(crate::vars::VarDomain, u16), String, VarHasher>,
        component_names: &HashMap<u32, String, ComponentHasher>,
        enum_value_names: &HashMap<i32, String, EnumHasher>,
        script_catalog: &super::ScriptCatalog,
        script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
    ) -> Self
    where
        VarHasher: BuildHasher,
        ComponentHasher: BuildHasher,
        EnumHasher: BuildHasher,
        SignatureHasher: BuildHasher,
    {
        Self::new_for_build(
            instructions,
            var_names,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
            0,
        )
    }

    pub fn new_for_build<VarHasher, ComponentHasher, EnumHasher, SignatureHasher>(
        instructions: &'a [super::ast::InstructionNode],
        var_names: &HashMap<(crate::vars::VarDomain, u16), String, VarHasher>,
        component_names: &HashMap<u32, String, ComponentHasher>,
        enum_value_names: &HashMap<i32, String, EnumHasher>,
        script_catalog: &super::ScriptCatalog,
        script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
        build: u32,
    ) -> Self
    where
        VarHasher: BuildHasher,
        ComponentHasher: BuildHasher,
        EnumHasher: BuildHasher,
        SignatureHasher: BuildHasher,
    {
        let recovered = ExprRecovery::new_for_build(
            instructions,
            var_names,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
            build,
        )
        .recover();
        Self {
            instructions,
            recovered,
        }
    }

    pub fn build(self) -> Vec<Block> {
        if self.instructions.is_empty() {
            return vec![];
        }

        let leaders = self.compute_leaders();
        let mut blocks = self.create_blocks(&leaders);
        self.compute_edges(&mut blocks);
        self.analyze_branches(&mut blocks);

        blocks
    }

    /// Merge consecutive blocks where block N ends with a branch that
    fn compute_leaders(&self) -> Vec<usize> {
        let mut leaders = HashSet::new();
        leaders.insert(0);

        for (i, instr) in self.instructions.iter().enumerate() {
            let next = i + 1;
            let targets = self.extract_branch_targets(instr);

            if let Some(ref targets) = targets {
                for &target in targets {
                    // Don't make target a leader if it's just the fallthrough
                    if target < self.instructions.len() && target != next {
                        leaders.insert(target);
                    }
                }
            }

            let split_switch_fallthrough = instr.command == "switch"
                && self
                    .instructions
                    .get(next)
                    .is_some_and(|next_instr| next_instr.command != "branch");
            let is_branch = split_switch_fallthrough
                || matches!(
                    instr.command.as_str(),
                    "branch"
                        | "branch_not"
                        | "branch_if_true"
                        | "branch_if_false"
                        | "branch_equals"
                        | "branch_less_than"
                        | "branch_greater_than"
                        | "branch_less_than_or_equals"
                        | "branch_greater_than_or_equals"
                        | "long_branch_not"
                        | "long_branch_equals"
                        | "long_branch_less_than"
                        | "long_branch_greater_than"
                        | "long_branch_less_than_or_equals"
                        | "long_branch_greater_than_or_equals"
                        | "return"
                );

            if is_branch && next < self.instructions.len() {
                // Don't split if all branch targets == next (both paths same place)
                let all_same = targets
                    .as_ref()
                    .is_some_and(|t| !t.is_empty() && t.iter().all(|&x| x == next));
                if split_switch_fallthrough || !all_same {
                    leaders.insert(next);
                }
            }
        }

        let mut leaders: Vec<usize> = leaders.into_iter().collect();
        leaders.sort_unstable();
        leaders
    }

    fn extract_branch_targets(&self, instr: &super::ast::InstructionNode) -> Option<Vec<usize>> {
        match instr.command.as_str() {
            "branch" => {
                if let super::ast::OperandNode::Branch(target) = instr.operand {
                    Some(vec![target])
                } else {
                    None
                }
            }
            "branch_not"
            | "branch_if_true"
            | "branch_if_false"
            | "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals"
            | "long_branch_not"
            | "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals" => {
                if let super::ast::OperandNode::Branch(target) = instr.operand {
                    Some(vec![target, instr.index + 1])
                } else {
                    None
                }
            }
            "switch" => {
                if let super::ast::OperandNode::Switch(cases) = &instr.operand {
                    let mut targets: Vec<usize> = cases.iter().map(|c| c.target).collect();
                    targets.push(instr.index + 1);
                    Some(targets)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn create_blocks(&self, leaders: &[usize]) -> Vec<Block> {
        let mut blocks = Vec::new();

        for (i, &start) in leaders.iter().enumerate() {
            let end = leaders
                .iter()
                .copied()
                .find(|&x| x > start)
                .unwrap_or(self.instructions.len());
            let mut block = Block::new(i, start);
            block.end = end;
            blocks.push(block);
        }

        for block in &mut *blocks {
            let stmts: Vec<RecoveredStmt> = (block.start..block.end)
                .filter_map(|idx| self.recovered.get(idx).cloned().flatten())
                .collect();
            block.statements = stmts;
        }

        blocks
    }

    fn compute_edges(&self, blocks: &mut [Block]) {
        let block_count = blocks.len();
        let mut succ_map: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut pred_map: HashMap<usize, Vec<usize>> = HashMap::new();

        for (bi, block) in blocks.iter().enumerate() {
            // Look at the last few instructions for branch patterns.
            let block_instrs = &self.instructions[block.start..block.end];
            let last_instr = block_instrs.last();
            let prev_instr = if block_instrs.len() >= 2 {
                block_instrs.get(block_instrs.len() - 2)
            } else {
                None
            };

            // Detect: conditional_branch (target == next_instr) followed by `branch`
            let targets_opt: Option<Vec<usize>> = if let (Some(prev), Some(last)) =
                (prev_instr, last_instr)
                && last.command == "branch"
                && is_cond_flag_instr(&prev.command)
                && prev.index + 1 == last.index
                && let super::ast::OperandNode::Branch(true_target) = prev.operand
                && let super::ast::OperandNode::Branch(false_target) = last.operand
            {
                Some(vec![true_target, false_target])
            } else if let Some(instr) = last_instr {
                self.extract_branch_targets(instr)
            } else {
                None
            };

            if let Some(targets) = targets_opt {
                for &target in &targets {
                    if let Some(target_bi) = blocks.iter().position(|b| b.start == target) {
                        succ_map.entry(bi).or_default().push(target_bi);
                        pred_map.entry(target_bi).or_default().push(bi);
                    }
                }
            }

            let has_jump = last_instr
                .is_some_and(|i| matches!(i.command.as_str(), "branch" | "return" | "switch"));

            if !has_jump && bi + 1 < block_count {
                succ_map.entry(bi).or_default().push(bi + 1);
                pred_map.entry(bi + 1).or_default().push(bi);
            }
        }

        for (bi, block) in blocks.iter_mut().enumerate() {
            if let Some(succs) = succ_map.get(&bi) {
                block.successors.clone_from(succs);
            }
            if let Some(preds) = pred_map.get(&bi) {
                block.predecessors.clone_from(preds);
            }
        }

        for (bi, block) in blocks.iter_mut().enumerate() {
            for &succ in &block.successors {
                if succ < bi {
                    block.is_loop_header = true;
                    block.loop_target = Some(succ);
                    break;
                }
            }
        }
    }

    fn analyze_branches(&self, blocks: &mut [Block]) {
        for block in &mut *blocks {
            if block.successors.len() >= 2 {
                block.is_conditional_branch = true;
                // Find the last Branch/BranchBinary in the statements
                // (not just the last statement, which might be a Goto)
                for stmt in block.statements.iter().rev() {
                    let cond = branch_condition_expr(stmt);
                    if let Some(cond) = cond {
                        block.branch_condition = Some(cond);
                        break;
                    }
                }
            }
        }
    }
}

pub(crate) fn branch_condition_expr(stmt: &RecoveredStmt) -> Option<Expression> {
    match stmt {
        RecoveredStmt::Branch {
            condition,
            negated: false,
            ..
        } => Some(condition.clone()),
        RecoveredStmt::Branch {
            condition,
            negated: true,
            ..
        } => Some(Expression::UnaryOperation(UnaryOperation {
            op: UnaryOp::Not,
            operand: Box::new(condition.clone()),
        })),
        RecoveredStmt::BranchBinary {
            op, left, right, ..
        } => Some(Expression::BinaryOperation(BinaryOperation {
            op: *op,
            left: Box::new(left.clone()),
            right: Box::new(right.clone()),
        })),
        _ => None,
    }
}

pub(crate) fn assignment_target_from_recovered(target: &str) -> AssignmentTarget {
    if let Some((array, index)) = target.split_once('[')
        && let Some(index) = index.strip_suffix(']')
    {
        if let Some(index_expr) = simple_target_index_expr(index) {
            return AssignmentTarget::ArrayAccess {
                array: array.to_string(),
                index: index_expr,
            };
        }
        return AssignmentTarget::Opaque(target.to_string());
    }
    if is_identifier_like(target) {
        AssignmentTarget::Identifier(target.to_string())
    } else {
        AssignmentTarget::Opaque(target.to_string())
    }
}

fn is_identifier_like(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn simple_target_index_expr(value: &str) -> Option<Expression> {
    if value == "pop()" {
        return Some(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "pop".to_string(),
            })),
            arguments: Vec::new(),
        }));
    }
    if let Ok(number) = value.parse::<i32>() {
        return Some(Expression::NumberLiteral(super::ast::NumberLiteral {
            value: number,
        }));
    }
    if is_identifier_like(value) {
        return Some(Expression::Identifier(super::ast::Identifier {
            name: value.to_string(),
        }));
    }
    None
}

fn is_cond_flag_instr(cmd: &str) -> bool {
    matches!(
        cmd,
        "branch_not"
            | "branch_if_true"
            | "branch_if_false"
            | "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals"
            | "long_branch_not"
            | "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals"
    )
}

pub fn build_cfg<VarHasher, ComponentHasher, EnumHasher, SignatureHasher>(
    instructions: &[super::ast::InstructionNode],
    var_names: &HashMap<(crate::vars::VarDomain, u16), String, VarHasher>,
    component_names: &HashMap<u32, String, ComponentHasher>,
    enum_value_names: &HashMap<i32, String, EnumHasher>,
    script_catalog: &super::ScriptCatalog,
    script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
) -> Vec<Block>
where
    VarHasher: BuildHasher,
    ComponentHasher: BuildHasher,
    EnumHasher: BuildHasher,
    SignatureHasher: BuildHasher,
{
    CfgBuilder::new(
        instructions,
        var_names,
        component_names,
        enum_value_names,
        script_catalog,
        script_signatures,
    )
    .build()
}

pub fn build_cfg_for_build<VarHasher, ComponentHasher, EnumHasher, SignatureHasher>(
    instructions: &[super::ast::InstructionNode],
    var_names: &HashMap<(crate::vars::VarDomain, u16), String, VarHasher>,
    component_names: &HashMap<u32, String, ComponentHasher>,
    enum_value_names: &HashMap<i32, String, EnumHasher>,
    script_catalog: &super::ScriptCatalog,
    script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
    build: u32,
) -> Vec<Block>
where
    VarHasher: BuildHasher,
    ComponentHasher: BuildHasher,
    EnumHasher: BuildHasher,
    SignatureHasher: BuildHasher,
{
    CfgBuilder::new_for_build(
        instructions,
        var_names,
        component_names,
        enum_value_names,
        script_catalog,
        script_signatures,
        build,
    )
    .build()
}

pub fn emit_structured(blocks: &[Block]) -> Vec<StructuredStmt> {
    super::structurer::structure(blocks)
}

pub fn emit_linear_structured(blocks: &[Block]) -> Vec<StructuredStmt> {
    super::structurer::structure_linear(blocks)
}

/// Scan structured statements for `Return` nodes to determine the
/// function return type.
pub fn detect_return_type(stmts: &[StructuredStmt]) -> &'static str {
    detect_return_type_inner::<std::collections::hash_map::RandomState>(stmts, None, None)
}

pub fn detect_return_type_with_signatures<SignatureHasher>(
    stmts: &[StructuredStmt],
    script_catalog: &super::ScriptCatalog,
    script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
) -> &'static str
where
    SignatureHasher: BuildHasher,
{
    detect_return_type_inner(stmts, Some(script_catalog), Some(script_signatures))
}

fn detect_return_type_inner<SignatureHasher>(
    stmts: &[StructuredStmt],
    script_catalog: Option<&super::ScriptCatalog>,
    script_signatures: Option<&HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>>,
) -> &'static str
where
    SignatureHasher: BuildHasher,
{
    let context = ReturnTypeContext {
        script_catalog,
        script_signatures,
    };
    let mut value_kind = None;
    let mut has_mixed_value_returns = false;
    let mut has_void_return = false;
    // In the linear (goto) form every block is reachable via its label, so code
    // after a terminator is NOT dead and the scan must not stop early. Only the
    // nested form has a genuinely-unreachable tail (the dead default-return
    // epilogue) to skip.
    let stop_at_terminator = !contains_label(stmts);
    scan_for_returns(
        stmts,
        &context,
        &mut value_kind,
        &mut has_mixed_value_returns,
        &mut has_void_return,
        stop_at_terminator,
    );
    let value_kind = if has_mixed_value_returns {
        Some(ReturnValueKind::Int)
    } else {
        value_kind
    };
    match (value_kind, has_void_return) {
        (Some(ReturnValueKind::Int), false) => "number",
        (Some(ReturnValueKind::Int), true) => "number | void",
        (Some(ReturnValueKind::Object), false) => "string",
        (Some(ReturnValueKind::Object), true) => "string | void",
        (Some(ReturnValueKind::Long), false) => "bigint",
        (Some(ReturnValueKind::Long), true) => "bigint | void",
        (None, true) => "void",
        (None, false) => "void",
    }
}

#[derive(Debug, Clone, Copy)]
struct ReturnTypeContext<'a, SignatureHasher> {
    script_catalog: Option<&'a super::ScriptCatalog>,
    script_signatures:
        Option<&'a HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnValueKind {
    Int,
    Object,
    Long,
}

fn record_return_value_kind(
    value_kind: &mut Option<ReturnValueKind>,
    has_mixed_value_returns: &mut bool,
    next: ReturnValueKind,
) {
    match value_kind {
        Some(existing) if *existing != next => *has_mixed_value_returns = true,
        Some(_) => {}
        None => *value_kind = Some(next),
    }
}

fn infer_return_value_kind<SignatureHasher>(
    expr: &Expression,
    context: &ReturnTypeContext<'_, SignatureHasher>,
) -> ReturnValueKind
where
    SignatureHasher: BuildHasher,
{
    match expr {
        Expression::BigIntLiteral(_) => ReturnValueKind::Long,
        Expression::StringLiteral(_) => ReturnValueKind::Object,
        Expression::Identifier(identifier) => kind_from_identifier(&identifier.name),
        Expression::Call(call) => infer_call_return_value_kind(call, context),
        _ => ReturnValueKind::Int,
    }
}

fn kind_from_identifier(name: &str) -> ReturnValueKind {
    if name.starts_with("local_obj_") || name.starts_with("arg_obj_") {
        ReturnValueKind::Object
    } else if name.starts_with("local_long_") || name.starts_with("arg_long_") {
        ReturnValueKind::Long
    } else {
        ReturnValueKind::Int
    }
}

fn infer_call_return_value_kind(
    call: &super::ast::CallExpr,
    context: &ReturnTypeContext<'_, impl BuildHasher>,
) -> ReturnValueKind {
    let Expression::Identifier(identifier) = &*call.callee else {
        return ReturnValueKind::Int;
    };
    if identifier.name == "stack" {
        return homogeneous_stack_return_kind(&call.arguments, context)
            .unwrap_or(ReturnValueKind::Int);
    }
    if let Some(kind) = script_call_return_value_kind(&identifier.name, context) {
        return kind;
    }
    match identifier.name.as_str() {
        "append" | "concat" | "ocname" | "tostring" | "tostringlong" => ReturnValueKind::Object,
        _ => ReturnValueKind::Int,
    }
}

fn script_call_return_value_kind(
    export_name: &str,
    context: &ReturnTypeContext<'_, impl BuildHasher>,
) -> Option<ReturnValueKind> {
    let catalog = context.script_catalog?;
    let signatures = context.script_signatures?;
    let metadata = catalog.resolve_export_name(export_name)?;
    let signature = signatures
        .get(&metadata.packed_id)
        .unwrap_or(&metadata.signature);
    kind_from_return_type_name(&signature.return_type)
}

fn kind_from_return_type_name(value: &str) -> Option<ReturnValueKind> {
    if return_type_contains(value, "string") {
        Some(ReturnValueKind::Object)
    } else if return_type_contains(value, "bigint") {
        Some(ReturnValueKind::Long)
    } else if return_type_contains(value, "number") || return_type_contains(value, "boolean") {
        Some(ReturnValueKind::Int)
    } else {
        None
    }
}

fn return_type_contains(value: &str, ty: &str) -> bool {
    value.split('|').any(|part| part.trim() == ty)
}

fn homogeneous_stack_return_kind(
    values: &[Expression],
    context: &ReturnTypeContext<'_, impl BuildHasher>,
) -> Option<ReturnValueKind> {
    let mut value_kinds = values
        .iter()
        .filter(|value| !is_void_script_call(value, context))
        .map(|value| infer_return_value_kind(value, context));
    let first = value_kinds.next()?;
    value_kinds.all(|kind| kind == first).then_some(first)
}

fn is_void_script_call(
    expr: &Expression,
    context: &ReturnTypeContext<'_, impl BuildHasher>,
) -> bool {
    let Expression::Call(call) = expr else {
        return false;
    };
    let Expression::Identifier(identifier) = &*call.callee else {
        return false;
    };
    let Some(catalog) = context.script_catalog else {
        return false;
    };
    let Some(signatures) = context.script_signatures else {
        return false;
    };
    let Some(metadata) = catalog.resolve_export_name(&identifier.name) else {
        return false;
    };
    let signature = signatures
        .get(&metadata.packed_id)
        .unwrap_or(&metadata.signature);
    signature.return_type.trim() == "void"
}

fn contains_label(stmts: &[StructuredStmt]) -> bool {
    stmts
        .iter()
        .any(|s| matches!(s, StructuredStmt::Label { .. }))
}

fn scan_for_returns(
    stmts: &[StructuredStmt],
    context: &ReturnTypeContext<'_, impl BuildHasher>,
    value_kind: &mut Option<ReturnValueKind>,
    has_mixed_value_returns: &mut bool,
    has_void: &mut bool,
    stop_at_terminator: bool,
) {
    for stmt in stmts {
        match stmt {
            StructuredStmt::Return { value } => {
                if let Some(value) = value {
                    record_return_value_kind(
                        value_kind,
                        has_mixed_value_returns,
                        infer_return_value_kind(value, context),
                    );
                } else {
                    *has_void = true;
                }
            }
            StructuredStmt::While { body } => {
                scan_for_returns(
                    body,
                    context,
                    value_kind,
                    has_mixed_value_returns,
                    has_void,
                    stop_at_terminator,
                );
            }
            StructuredStmt::If {
                then_body,
                else_body,
                ..
            } => {
                scan_for_returns(
                    then_body,
                    context,
                    value_kind,
                    has_mixed_value_returns,
                    has_void,
                    stop_at_terminator,
                );
                if let Some(else_b) = else_body {
                    scan_for_returns(
                        else_b,
                        context,
                        value_kind,
                        has_mixed_value_returns,
                        has_void,
                        stop_at_terminator,
                    );
                }
            }
            StructuredStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    scan_for_returns(
                        &case.body,
                        context,
                        value_kind,
                        has_mixed_value_returns,
                        has_void,
                        stop_at_terminator,
                    );
                }
                if let Some(default_body) = default_body {
                    scan_for_returns(
                        default_body,
                        context,
                        value_kind,
                        has_mixed_value_returns,
                        has_void,
                        stop_at_terminator,
                    );
                }
            }
            _ => {}
        }
        // In the nested form, statements after an unconditional terminator are
        // unreachable — notably the dead default-return epilogue the structurer
        // emits for byte fidelity — so don't let them influence the inferred
        // return type. The linear form keeps everything reachable via labels.
        if stop_at_terminator && stmt_terminates(stmt) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_cfg;
    use crate::transpile::ast::{
        CallExpr, Expression, Identifier, InstructionNode, OperandNode, StringLiteral, SwitchCase,
    };
    use crate::transpile::structured::StructuredStmt;
    use crate::transpile::{
        ScriptCatalog, ScriptGroupId, ScriptId, ScriptKind, ScriptMetadata, ScriptSignature,
    };
    use std::collections::HashMap;

    #[test]
    fn unconditional_branch_has_no_fallthrough_successor() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "branch".to_string(),
                operand: OperandNode::Branch(2),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
        ];

        let blocks = build_cfg(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        );
        let successor_starts = blocks[0]
            .successors
            .iter()
            .map(|&successor| blocks[successor].start)
            .collect::<Vec<_>>();

        assert_eq!(successor_starts, vec![2]);
    }

    #[test]
    fn switch_fallthrough_has_own_block() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "switch".to_string(),
                operand: OperandNode::Switch(vec![SwitchCase {
                    value: 1,
                    target: 4,
                }]),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "cam_reset".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "branch".to_string(),
                operand: OperandNode::Branch(5),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
        ];

        let blocks = build_cfg(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        );
        let block_starts = blocks.iter().map(|block| block.start).collect::<Vec<_>>();
        let successor_starts = blocks[0]
            .successors
            .iter()
            .map(|&successor| blocks[successor].start)
            .collect::<Vec<_>>();

        assert_eq!(block_starts, vec![0, 2, 4, 5]);
        assert_eq!(successor_starts, vec![4, 2]);
    }

    #[test]
    fn branch_to_return_block_uses_linear_form() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "branch".to_string(),
                operand: OperandNode::Branch(2),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
        ];

        let blocks = build_cfg(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        );
        let structured = super::emit_structured(&blocks);

        assert!(matches!(
            structured.first(),
            Some(StructuredStmt::Goto { target: 2 })
        ));
        assert!(
            structured
                .iter()
                .any(|stmt| matches!(stmt, StructuredStmt::Label { target: 2 }))
        );
    }

    #[test]
    fn detect_return_type_uses_string_return_values() {
        let structured = vec![StructuredStmt::Return {
            value: Some(Expression::StringLiteral(StringLiteral {
                value: "ok".to_string(),
            })),
        }];

        assert_eq!(super::detect_return_type(&structured), "string");
    }

    #[test]
    fn detect_return_type_uses_homogeneous_stack_value_type() {
        let structured = vec![StructuredStmt::Return {
            value: Some(Expression::Call(CallExpr {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "stack".to_string(),
                })),
                arguments: vec![
                    Expression::Identifier(Identifier {
                        name: "local_obj_0".to_string(),
                    }),
                    Expression::StringLiteral(StringLiteral {
                        value: "ok".to_string(),
                    }),
                ],
            })),
        }];

        assert_eq!(super::detect_return_type(&structured), "string");
    }

    #[test]
    fn detect_return_type_ignores_void_calls_inside_stack_return() {
        let script_id = ScriptId(1);
        let mut catalog = ScriptCatalog::default();
        let signature = ScriptSignature {
            arg_count_int: 1,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 0,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "void".to_string(),
        };
        catalog.insert(ScriptMetadata {
            packed_id: script_id,
            group_id: ScriptGroupId(1),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "void_helper".to_string(),
            export_name: "void_helper".to_string(),
            module_name: "void_helper".to_string(),
            signature: signature.clone(),
        });
        let signatures = HashMap::from([(script_id, signature)]);
        let structured = vec![StructuredStmt::Return {
            value: Some(Expression::Call(CallExpr {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "stack".to_string(),
                })),
                arguments: vec![
                    Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "void_helper".to_string(),
                        })),
                        arguments: Vec::new(),
                    }),
                    Expression::StringLiteral(StringLiteral {
                        value: "ok".to_string(),
                    }),
                ],
            })),
        }];

        assert_eq!(
            super::detect_return_type_with_signatures(&structured, &catalog, &signatures),
            "string"
        );
    }
}
