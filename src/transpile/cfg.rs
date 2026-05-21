use super::ast::{Expression, Statement, TypeAnnotation};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Block {
    pub start: usize,
    pub end: usize,
    pub statements: Vec<Statement>,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
    pub is_loop_header: bool,
    pub loop_targets: Vec<usize>,
}

impl Block {
    pub fn new(start: usize) -> Self {
        Self {
            start,
            end: start,
            statements: Vec::new(),
            successors: Vec::new(),
            predecessors: Vec::new(),
            is_loop_header: false,
            loop_targets: Vec::new(),
        }
    }
}

pub struct CfgBuilder {
    instructions: Vec<super::ast::InstructionNode>,
}

impl CfgBuilder {
    pub fn new(instructions: Vec<super::ast::InstructionNode>) -> Self {
        Self { instructions }
    }

    pub fn build(self) -> Vec<Block> {
        if self.instructions.is_empty() {
            return vec![];
        }

        let leaders = self.compute_leaders();
        let mut blocks = self.create_blocks(&leaders);
        self.compute_edges(&mut blocks);
        self.detect_loops(&mut blocks);

        blocks
    }

    fn compute_leaders(&self) -> Vec<usize> {
        let mut leaders = HashSet::new();
        leaders.insert(0);

        for (i, instr) in self.instructions.iter().enumerate() {
            if let Some(targets) = self.extract_branch_targets(instr) {
                for &target in &targets {
                    if target < self.instructions.len() {
                        leaders.insert(target);
                    }
                }
            }

            if matches!(
                instr.command.as_str(),
                "branch"
                    | "branch_not"
                    | "branch_if_true"
                    | "branch_if_false"
                    | "branch_equals"
                    | "gosub_with_params"
                    | "return"
            ) {
                let next = i + 1;
                if next < self.instructions.len() {
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
            "branch" | "branch_not" | "branch_if_true" | "branch_if_false" | "branch_equals" => {
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

        for &start in leaders {
            let end = leaders
                .iter()
                .copied()
                .find(|&x| x > start)
                .unwrap_or(self.instructions.len());
            let mut block = Block::new(start);
            block.end = end;
            blocks.push(block);
        }

        for block in &mut blocks {
            let stmts: Vec<Statement> = (block.start..block.end)
                .filter_map(|i| self.instruction_to_statement(&self.instructions[i]))
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
            let last_instr = self.instructions.iter().find(|i| i.index == block.start);

            if let Some(instr) = last_instr
                && let Some(targets) = self.extract_branch_targets(instr)
            {
                for &target in &targets {
                    if let Some(target_bi) = blocks.iter().position(|b| b.start == target) {
                        succ_map.entry(bi).or_default().push(target_bi);
                        pred_map.entry(target_bi).or_default().push(bi);
                    }
                }
            }

            let has_branch_end = last_instr
                .map(|i| {
                    matches!(
                        i.command.as_str(),
                        "branch" | "return" | "gosub_with_params"
                    )
                })
                .unwrap_or(false);

            if !has_branch_end && bi + 1 < block_count {
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
    }

    fn detect_loops(&self, blocks: &mut [Block]) {
        for (bi, block) in blocks.iter_mut().enumerate() {
            for &succ in &block.successors {
                if succ <= bi {
                    block.is_loop_header = true;
                    if !block.loop_targets.contains(&succ) {
                        block.loop_targets.push(succ);
                    }
                }
            }
        }
    }

    fn instruction_to_statement(&self, instr: &super::ast::InstructionNode) -> Option<Statement> {
        use super::ast::{
            CallExpr, ExpressionStatement, GotoStatement, Identifier, NumberLiteral,
            ReturnStatement, StringLiteral, VariableDeclaration,
        };

        match instr.command.as_str() {
            "push_constant_int" => {
                if let super::ast::OperandNode::Int(v) = instr.operand {
                    return Some(Statement::ExpressionStatement(ExpressionStatement {
                        expr: Expression::NumberLiteral(NumberLiteral { value: v }),
                        semicolon: true,
                    }));
                }
            }
            "push_constant_string" => {
                if let super::ast::OperandNode::String(s) = &instr.operand {
                    return Some(Statement::ExpressionStatement(ExpressionStatement {
                        expr: Expression::StringLiteral(StringLiteral { value: s.clone() }),
                        semicolon: true,
                    }));
                }
            }
            "push_int_local" | "push_string_local" | "push_long_local" => {
                if let super::ast::OperandNode::Local(idx) = instr.operand {
                    let (prefix, _) = self.local_type_info(&instr.command);
                    return Some(Statement::ExpressionStatement(ExpressionStatement {
                        expr: Expression::Identifier(Identifier {
                            name: format!("{prefix}_{idx}"),
                        }),
                        semicolon: true,
                    }));
                }
            }
            "pop_int_local" | "pop_string_local" | "pop_long_local" => {
                if let super::ast::OperandNode::Local(idx) = instr.operand {
                    let (prefix, ts_type) = self.local_type_info(&instr.command);
                    return Some(Statement::VariableDeclaration(VariableDeclaration {
                        name: format!("{prefix}_{idx}"),
                        type_hint: ts_type,
                        initializer: Some(Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })),
                    }));
                }
            }
            "branch" => {
                if let super::ast::OperandNode::Branch(target) = instr.operand {
                    return Some(Statement::GotoStatement(GotoStatement { target }));
                }
            }
            "return" => {
                return Some(Statement::ReturnStatement(ReturnStatement { value: None }));
            }
            "switch" => {
                return Some(Statement::Comment("switch(pop()) { ... }".to_string()));
            }
            _ => {}
        }

        None
    }

    fn local_type_info(&self, cmd: &str) -> (&'static str, TypeAnnotation) {
        if cmd.contains("long") {
            ("local_long", TypeAnnotation::BigInt)
        } else if cmd.contains("string") {
            ("local_obj", TypeAnnotation::String)
        } else {
            ("local_int", TypeAnnotation::Number)
        }
    }
}

pub fn build_cfg(instructions: Vec<super::ast::InstructionNode>) -> Vec<Block> {
    CfgBuilder::new(instructions).build()
}

#[derive(Debug)]
pub enum StructuredStatement {
    While {
        condition: String,
        body: Vec<Self>,
    },
    If {
        condition: String,
        then_case: Vec<Self>,
        else_case: Option<Vec<Self>>,
    },
    Switch {
        expression: String,
        cases: Vec<SwitchCase>,
        default: Option<Vec<Self>>,
    },
    Assignment {
        target: String,
        value: String,
    },
    Expression {
        expr: String,
    },
    Goto {
        target: usize,
    },
    Return {
        value: Option<String>,
    },
    Comment(String),
}

#[derive(Debug)]
pub struct SwitchCase {
    pub value: i32,
    pub body: Vec<StructuredStatement>,
}

pub struct StructuredCfgGen {
    blocks: Vec<Block>,
}

impl StructuredCfgGen {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }

    pub fn generate(self) -> Vec<StructuredStatement> {
        if self.blocks.is_empty() {
            return vec![];
        }

        let mut result = Vec::new();
        let mut visited = vec![false; self.blocks.len()];
        self.emit_block(0, &mut visited, &mut result);
        result
    }

    fn emit_block(&self, bi: usize, visited: &mut Vec<bool>, out: &mut Vec<StructuredStatement>) {
        if bi >= self.blocks.len() || visited[bi] {
            return;
        }
        visited[bi] = true;

        let block = &self.blocks[bi];

        if block.is_loop_header && block.loop_targets.first() == Some(&bi) {
            self.emit_loop_header(bi, visited, out);
            return;
        }

        for stmt in &block.statements {
            out.push(self.stmt_to_structured(stmt));
        }

        if let Some(&next) = block.successors.first()
            && next > bi
        {
            self.emit_block(next, visited, out);
        }
    }

    fn emit_loop_header(
        &self,
        bi: usize,
        _visited: &mut Vec<bool>,
        out: &mut Vec<StructuredStatement>,
    ) {
        let block = &self.blocks[bi];

        if let Some(&_loop_target) = block.loop_targets.first() {
            let body_stmts = self.collect_loop_body();

            out.push(StructuredStatement::While {
                condition: "true".to_string(),
                body: body_stmts,
            });
        }
    }

    fn collect_loop_body(&self) -> Vec<StructuredStatement> {
        let mut stmts = Vec::new();
        for block in &self.blocks {
            if block.is_loop_header {
                for stmt in &block.statements {
                    match self.stmt_to_structured(stmt) {
                        StructuredStatement::Goto { target } => {
                            if target != block.start {
                                stmts.push(StructuredStatement::Comment(format!(
                                    "goto block_{target}"
                                )));
                            }
                        }
                        s => stmts.push(s),
                    }
                }
            }
        }
        stmts
    }

    fn stmt_to_structured(&self, stmt: &Statement) -> StructuredStatement {
        match stmt {
            Statement::Comment(text) => StructuredStatement::Comment(text.clone()),
            Statement::ExpressionStatement(es) => StructuredStatement::Expression {
                expr: self.expr_to_string(&es.expr),
            },
            Statement::VariableDeclaration(vd) => {
                let value = match &vd.initializer {
                    Some(expr) => self.expr_to_string(expr),
                    None => "pop()".to_string(),
                };
                StructuredStatement::Assignment {
                    target: vd.name.clone(),
                    value,
                }
            }
            Statement::GotoStatement(gs) => StructuredStatement::Goto { target: gs.target },
            Statement::ReturnStatement(rs) => StructuredStatement::Return {
                value: rs.value.as_ref().map(|e| self.expr_to_string(e)),
            },
            _ => StructuredStatement::Comment(format!("{stmt:?}")),
        }
    }

    #[allow(clippy::self_only_used_in_recursion)]
    fn expr_to_string(&self, expr: &Expression) -> String {
        match expr {
            Expression::NumberLiteral(n) => n.value.to_string(),
            Expression::Identifier(id) => id.name.clone(),
            Expression::StringLiteral(s) => format!("\"{}\"", s.value),
            Expression::Call(c) => {
                let callee = self.expr_to_string(&c.callee);
                let args: Vec<String> =
                    c.arguments.iter().map(|e| self.expr_to_string(e)).collect();
                format!("{callee}({})", args.join(", "))
            }
            _ => "pop()".to_string(),
        }
    }
}

pub fn generate_structured(blocks: Vec<Block>) -> Vec<StructuredStatement> {
    let generator = StructuredCfgGen::new(blocks);
    generator.generate()
}
