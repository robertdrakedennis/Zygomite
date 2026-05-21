use crate::vars::VarDomain;

#[derive(Debug, Clone)]
pub enum TypeScriptNode {
    Program(Program),
    Statement(Statement),
    Expression(Expression),
    Declaration(Declaration),
}

#[derive(Debug, Clone)]
pub struct Program {
    pub imports: Vec<ImportStatement>,
    pub statements: Vec<Statement>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImportStatement {
    pub module: String,
    pub named_exports: Vec<String>,
    pub is_type_only: bool,
}

#[derive(Debug, Clone)]
pub enum Statement {
    ExpressionStatement(ExpressionStatement),
    VariableDeclaration(VariableDeclaration),
    IfStatement(IfStatement),
    GotoStatement(GotoStatement),
    SwitchStatement(SwitchStatement),
    Label(Label),
    CallStatement(CallStatement),
    ReturnStatement(ReturnStatement),
    Comment(String),
}

#[derive(Debug, Clone)]
pub struct ExpressionStatement {
    pub expr: Expression,
    pub semicolon: bool,
}

#[derive(Debug, Clone)]
pub struct VariableDeclaration {
    pub name: String,
    pub type_hint: TypeAnnotation,
    pub initializer: Option<Expression>,
}

#[derive(Debug, Clone)]
pub struct IfStatement {
    pub condition: Expression,
    pub then_branch: Box<Statement>,
    pub else_branch: Option<Box<Statement>>,
}

#[derive(Debug, Clone)]
pub struct GotoStatement {
    pub target: usize,
}

#[derive(Debug, Clone)]
pub struct SwitchStatement {
    pub discriminant: Expression,
    pub cases: Vec<SwitchCase>,
    pub default_target: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub value: i32,
    pub target: usize,
}

#[derive(Debug, Clone)]
pub struct Label {
    pub index: usize,
}

#[derive(Debug, Clone)]
pub struct CallStatement {
    pub callee: String,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone)]
pub struct ReturnStatement {
    pub value: Option<Expression>,
}

#[derive(Debug, Clone)]
pub enum Expression {
    NumberLiteral(NumberLiteral),
    BigIntLiteral(BigIntLiteral),
    StringLiteral(StringLiteral),
    BooleanLiteral(BooleanLiteral),
    Identifier(Identifier),
    ArrayAccess(ArrayAccess),
    PropertyAccess(PropertyAccess),
    Call(CallExpr),
    BinaryOperation(BinaryOperation),
    UnaryOperation(UnaryOperation),
    PushOperation(PushOperation),
    PopOperation(PopOperation),
    GotoExpr(GotoExpr),
}

#[derive(Debug, Clone)]
pub struct NumberLiteral {
    pub value: i32,
}

#[derive(Debug, Clone)]
pub struct BigIntLiteral {
    pub value: i64,
}

#[derive(Debug, Clone)]
pub struct StringLiteral {
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct BooleanLiteral {
    pub value: bool,
}

#[derive(Debug, Clone)]
pub struct Identifier {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ArrayAccess {
    pub array: Box<Expression>,
    pub index: Box<Expression>,
}

#[derive(Debug, Clone)]
pub struct PropertyAccess {
    pub object: Box<Expression>,
    pub property: String,
}

#[derive(Debug, Clone)]
pub struct CallExpr {
    pub callee: Box<Expression>,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone)]
pub struct BinaryOperation {
    pub op: BinaryOp,
    pub left: Box<Expression>,
    pub right: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnaryOperation {
    pub op: UnaryOp,
    pub operand: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

impl UnaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PushOperation {
    pub value: Box<Expression>,
}

#[derive(Debug, Clone)]
pub struct PopOperation {
    pub target: Option<Box<Expression>>,
}

#[derive(Debug, Clone)]
pub struct GotoExpr {
    pub target: usize,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub script_id: i32,
    pub name: Option<String>,
    pub locals: Vec<LocalVariable>,
    pub arguments: Vec<ArgumentVariable>,
    pub instructions: Vec<InstructionNode>,
}

#[derive(Debug, Clone)]
pub struct LocalVariable {
    pub index: usize,
    pub name: String,
    pub type_annotation: TypeAnnotation,
}

#[derive(Debug, Clone)]
pub struct ArgumentVariable {
    pub index: usize,
    pub name: String,
    pub type_annotation: TypeAnnotation,
}

#[derive(Debug, Clone)]
pub struct InstructionNode {
    pub index: usize,
    pub opcode: u16,
    pub command: String,
    pub operand: OperandNode,
}

#[derive(Debug, Clone)]
pub enum OperandNode {
    Int(i32),
    Long(i64),
    String(String),
    Local(usize),
    VarRef(VarRefNode),
    VarBitRef(VarBitRefNode),
    Branch(usize),
    Switch(Vec<SwitchCase>),
    Script(i32),
    Array(i32),
    Count(usize),
    Byte(u8),
}

#[derive(Debug, Clone)]
pub struct VarRefNode {
    pub domain: VarDomain,
    pub id: u16,
    pub name: Option<String>,
    pub is_transmog: bool,
}

#[derive(Debug, Clone)]
pub struct VarBitRefNode {
    pub id: u16,
    pub name: Option<String>,
    pub is_transmog: bool,
}

#[derive(Debug, Clone)]
pub enum TypeAnnotation {
    Number,
    BigInt,
    String,
    Boolean,
    Unknown,
}

impl TypeAnnotation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::BigInt => "bigint",
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Unknown => "any",
        }
    }
}
