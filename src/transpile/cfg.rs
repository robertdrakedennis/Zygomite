use super::ast::{BinaryOperation, Expression, UnaryOp, UnaryOperation};
use super::expr_recovery::{ExprRecovery, RecoveredStmt};
use super::structured::{AssignmentTarget, StructuredStmt, SwitchCaseStmt};
use std::collections::{HashMap, HashSet};

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
    pub fn new<S: std::hash::BuildHasher>(
        instructions: &'a [super::ast::InstructionNode],
        var_names: &std::collections::HashMap<(crate::vars::VarDomain, u16), String>,
        component_names: &std::collections::HashMap<u32, String, S>,
        enum_value_names: &std::collections::HashMap<i32, String, S>,
        script_catalog: &super::ScriptCatalog,
        script_signatures: &std::collections::HashMap<super::ScriptId, super::ScriptSignature>,
    ) -> Self {
        let recovered = ExprRecovery::new(
            instructions,
            var_names,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
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

            let is_branch = matches!(
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
                if !all_same {
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
                    // branch is flag-conditional: two successors (target and fallthrough)
                    Some(vec![target, instr.index + 1])
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
                && let super::ast::OperandNode::Branch(target) = last.operand
            {
                let true_target = target;
                let false_target = last.index + 1;
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

            let has_jump =
                last_instr.is_some_and(|i| matches!(i.command.as_str(), "branch" | "return"));

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

#[expect(
    clippy::implicit_hasher,
    reason = "transpile APIs use default HashMap aliases across module boundaries"
)]
pub fn build_cfg<S: std::hash::BuildHasher>(
    instructions: &[super::ast::InstructionNode],
    var_names: &std::collections::HashMap<(crate::vars::VarDomain, u16), String>,
    component_names: &std::collections::HashMap<u32, String, S>,
    enum_value_names: &std::collections::HashMap<i32, String, S>,
    script_catalog: &super::ScriptCatalog,
    script_signatures: &std::collections::HashMap<super::ScriptId, super::ScriptSignature>,
) -> Vec<Block> {
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

pub struct StructuredEmitter {
    blocks: Vec<Block>,
    visited: Vec<bool>,
    result: Vec<StructuredStmt>,
    loop_target_to_header: HashMap<usize, usize>,
    /// Maps instruction index → block index
    instr_to_block: HashMap<usize, usize>,
}

impl StructuredEmitter {
    pub fn new(blocks: Vec<Block>) -> Self {
        let len = blocks.len();
        let mut loop_target_to_header = HashMap::new();
        let mut instr_to_block = HashMap::new();
        for (i, block) in blocks.iter().enumerate() {
            if let Some(target) = block.loop_target {
                loop_target_to_header.insert(target, i);
            }
            for idx in block.start..block.end {
                instr_to_block.insert(idx, i);
            }
        }
        Self {
            blocks,
            visited: vec![false; len],
            result: Vec::new(),
            loop_target_to_header,
            instr_to_block,
        }
    }

    pub fn emit(mut self) -> Vec<StructuredStmt> {
        if !self.blocks.is_empty() {
            self.emit_block(0);
        }
        self.result
    }

    fn emit_block(&mut self, bi: usize) {
        if bi >= self.blocks.len() || self.visited[bi] {
            return;
        }

        // If this block is the target of a loop, emit the loop first
        if let Some(&header) = self.loop_target_to_header.get(&bi) {
            self.emit_loop(header);
            return;
        }

        self.visited[bi] = true;

        let block = self.blocks[bi].clone();

        self.emit_block_statements(&block);

        if block.is_conditional_branch && block.successors.len() >= 2 {
            let Some(condition) = block.branch_condition.clone() else {
                if let Some(&next) = block.successors.first()
                    && next > bi
                {
                    self.emit_block(next);
                }
                return;
            };

            let true_target = block.successors[0];
            let false_target = if block.successors.len() > 1 {
                block.successors[1]
            } else {
                bi + 1
            };

            // Deduplicate: if both targets are the same, skip the if
            if true_target == false_target {
                self.emit_block(true_target);
                return;
            }
            let th0 = true_target > bi;
            let th1 = true_target <= bi;
            let fh0 = false_target > bi;
            let fh1 = false_target <= bi;

            let then_body = if th0 {
                self.collect_branch_body(true_target, bi)
            } else if th1 {
                vec![StructuredStmt::Continue]
            } else {
                vec![]
            };

            let else_body = if fh0 {
                Some(self.collect_branch_body(false_target, bi))
            } else if fh1 {
                Some(vec![])
            } else {
                None
            };

            self.result.push(StructuredStmt::If {
                condition,
                then_body,
                else_body,
            });
            return;
        }

        if let Some(&next) = block.successors.first()
            && next > bi
        {
            self.emit_block(next);
        }
    }

    fn emit_loop(&mut self, bi: usize) {
        let block = self.blocks[bi].clone();
        let Some(loop_target) = block.loop_target else {
            return;
        };
        let successors = block.successors;

        let body_blocks = self.collect_loop_blocks(bi, loop_target);
        let body_blocks_set: HashSet<usize> = body_blocks.iter().copied().collect();

        // Save result, emit loop body, then wrap in While
        let saved = std::mem::take(&mut self.result);
        // Start loop body emission from the loop target (not the header)
        self.emit_block_loop(loop_target, loop_target, &body_blocks_set);
        let body_stmts = std::mem::take(&mut self.result);
        self.result = saved;

        self.result.push(StructuredStmt::While { body: body_stmts });

        // Emit exit blocks
        for &succ in &successors {
            if succ != loop_target && !body_blocks_set.contains(&succ) {
                self.emit_block(succ);
            }
        }
    }

    /// Emit statements for blocks within a loop body, handling nested
    /// if/else and converting gotos to break/continue.
    fn emit_block_loop(&mut self, bi: usize, loop_target: usize, body_blocks: &HashSet<usize>) {
        if bi >= self.blocks.len() || self.visited[bi] {
            return;
        }
        self.visited[bi] = true;

        let block = self.blocks[bi].clone();

        // Emit block statements (skip the loop-back goto at the end)
        let block_is_cond = block.is_conditional_branch
            && block.successors.len() >= 2
            && block.branch_condition.is_some();

        for stmt in &block.statements {
            if is_loop_back_stmt(stmt, loop_target) {
                continue;
            }
            // Skip Branch/Goto statements whose target is handled by the if/else
            if block_is_cond {
                match stmt {
                    RecoveredStmt::Branch { .. } | RecoveredStmt::BranchBinary { .. } => continue,
                    RecoveredStmt::Goto(_) => continue,
                    _ => {}
                }
            }
            // Convert gotos/branches within the loop
            match stmt {
                RecoveredStmt::Goto(target) => {
                    let target_bi = self.instr_to_block.get(target).copied();
                    if target_bi == Some(loop_target) {
                        self.result.push(StructuredStmt::Continue);
                    } else if target_bi.is_some_and(|b| body_blocks.contains(&b)) {
                        self.result.push(StructuredStmt::Goto { target: *target });
                    } else {
                        self.result.push(StructuredStmt::Break);
                    }
                }
                _ => self.result.push(self.stmt_to_structured(stmt)),
            }
        }

        // Handle successors (if/else detection)
        if block_is_cond {
            let Some(condition) = block.branch_condition.clone() else {
                return;
            };
            let true_target = block.successors[0];
            let false_target = block.successors[1];

            let then_body = if body_blocks.contains(&true_target) {
                if self.blocks[true_target].is_loop_header {
                    vec![StructuredStmt::Continue]
                } else {
                    let saved = std::mem::take(&mut self.result);
                    self.emit_block_loop(true_target, loop_target, body_blocks);
                    let body = std::mem::take(&mut self.result);
                    self.result = saved;
                    body
                }
            } else {
                vec![StructuredStmt::Break]
            };

            let else_body = if body_blocks.contains(&false_target) {
                if self.blocks[false_target].is_loop_header {
                    Some(vec![StructuredStmt::Continue])
                } else {
                    let saved = std::mem::take(&mut self.result);
                    self.emit_block_loop(false_target, loop_target, body_blocks);
                    let body = std::mem::take(&mut self.result);
                    self.result = saved;
                    Some(body)
                }
            } else {
                None
            };

            self.result.push(StructuredStmt::If {
                condition,
                then_body,
                else_body,
            });
            return;
        }

        // Linear successor: follow forward within loop body
        if let Some(&next) = block.successors.first()
            && body_blocks.contains(&next)
            && next > bi
        {
            self.emit_block_loop(next, loop_target, body_blocks);
        }
    }

    fn stmt_to_structured(&self, stmt: &RecoveredStmt) -> StructuredStmt {
        match stmt {
            RecoveredStmt::Expression(expr) => StructuredStmt::Expr { expr: expr.clone() },
            RecoveredStmt::Assignment { target, value, .. } => StructuredStmt::Assignment {
                target: assignment_target_from_recovered(target),
                value: value.clone(),
            },
            RecoveredStmt::Goto(target) => StructuredStmt::Goto { target: *target },
            RecoveredStmt::Branch {
                condition,
                target,
                negated,
            } => {
                let condition = if *negated {
                    Expression::UnaryOperation(UnaryOperation {
                        op: UnaryOp::Not,
                        operand: Box::new(condition.clone()),
                    })
                } else {
                    condition.clone()
                };
                StructuredStmt::Comment(format!(
                    "if ({}) goto {target};",
                    super::structured::format_expression(&condition)
                ))
            }
            RecoveredStmt::BranchBinary {
                op,
                left,
                right,
                target,
            } => {
                let condition = Expression::BinaryOperation(BinaryOperation {
                    op: *op,
                    left: Box::new(left.clone()),
                    right: Box::new(right.clone()),
                });
                StructuredStmt::Comment(format!(
                    "if ({}) goto {target};",
                    super::structured::format_expression(&condition)
                ))
            }
            RecoveredStmt::Switch {
                discriminant,
                cases,
            } => {
                let cases = cases
                    .iter()
                    .map(|(v, _)| SwitchCaseStmt {
                        value: *v,
                        body: Vec::new(),
                    })
                    .collect();
                StructuredStmt::Switch {
                    expr: discriminant.clone(),
                    cases,
                }
            }
            RecoveredStmt::Return(val) => StructuredStmt::Return { value: val.clone() },
            RecoveredStmt::Comment(text) => StructuredStmt::Comment(text.clone()),
        }
    }

    fn collect_loop_blocks(&self, header: usize, loop_target: usize) -> Vec<usize> {
        let mut blocks = Vec::new();
        let mut queue = vec![loop_target];
        let mut visited = HashSet::new();

        if visited.insert(loop_target) {
            blocks.push(loop_target);
        }

        // Find exit blocks: header's successors that aren't the loop_target
        let exit_set: HashSet<usize> = self.blocks[header]
            .successors
            .iter()
            .copied()
            .filter(|&s| s != loop_target)
            .collect();

        while let Some(bi) = queue.pop() {
            // Don't recurse into the header itself or exit blocks
            if bi == header || exit_set.contains(&bi) {
                continue;
            }
            let block = &self.blocks[bi];
            for &succ in &block.successors {
                if succ != header && !exit_set.contains(&succ) && visited.insert(succ) {
                    blocks.push(succ);
                    queue.push(succ);
                }
            }
        }

        // Always include the header
        if visited.insert(header) {
            blocks.push(header);
        }

        blocks.sort_unstable();
        blocks
    }

    fn collect_branch_body(&mut self, target: usize, _fallthrough: usize) -> Vec<StructuredStmt> {
        // Walk from target until we hit a merge point or loop header.
        // Each block's statements are converted and the walk follows
        // the first successor forward.
        let mut stmts = Vec::new();
        let mut bi = target;

        loop {
            if bi >= self.blocks.len() || self.visited[bi] {
                break;
            }

            let block = &self.blocks[bi];
            self.visited[bi] = true;

            for stmt in &block.statements {
                stmts.push(self.stmt_to_structured(stmt));
            }

            if block.is_loop_header {
                break;
            }

            // Follow linear forward path, skipping blocks that have
            // multiple predecessors (merge points)
            if let Some(&next) = block.successors.first() {
                if next > bi && block.predecessors.len() <= 1 {
                    bi = next;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        stmts
    }

    fn emit_block_statements(&mut self, block: &Block) {
        for stmt in &block.statements {
            self.result.push(self.stmt_to_structured(stmt));
        }
    }
}

fn is_loop_back_stmt(stmt: &RecoveredStmt, loop_target: usize) -> bool {
    match stmt {
        RecoveredStmt::Goto(target) => *target == loop_target,
        RecoveredStmt::Branch { target, .. } | RecoveredStmt::BranchBinary { target, .. } => {
            *target == loop_target
        }
        _ => false,
    }
}

pub fn emit_structured(blocks: &[Block]) -> Vec<StructuredStmt> {
    super::structurer::structure(blocks)
}

/// Scan structured statements for `Return` nodes to determine the
/// function return type.
pub fn detect_return_type(stmts: &[StructuredStmt]) -> &'static str {
    let mut has_value_return = false;
    let mut has_void_return = false;
    scan_for_returns(stmts, &mut has_value_return, &mut has_void_return);
    match (has_value_return, has_void_return) {
        (true, false) => "number",
        (false, true) => "void",
        (true, true) => "number | void",
        (false, false) => "void",
    }
}

fn scan_for_returns(stmts: &[StructuredStmt], has_val: &mut bool, has_void: &mut bool) {
    for stmt in stmts {
        match stmt {
            StructuredStmt::Return { value } => {
                if value.is_some() {
                    *has_val = true;
                } else {
                    *has_void = true;
                }
            }
            StructuredStmt::While { body } => scan_for_returns(body, has_val, has_void),
            StructuredStmt::If {
                then_body,
                else_body,
                ..
            } => {
                scan_for_returns(then_body, has_val, has_void);
                if let Some(else_b) = else_body {
                    scan_for_returns(else_b, has_val, has_void);
                }
            }
            StructuredStmt::Switch { cases, .. } => {
                for case in cases {
                    scan_for_returns(&case.body, has_val, has_void);
                }
            }
            _ => {}
        }
    }
}
