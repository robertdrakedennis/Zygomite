use super::ast::{
    ArgumentVariable, ArrayAccess, BigIntLiteral, BinaryOp, BinaryOperation, BooleanLiteral,
    CallExpr, CallbackLiteral, Expression, Identifier, ImportStatement, LocalVariable,
    NumberLiteral, PropertyAccess, ScriptId, StringLiteral, TypeAnnotation, UnaryOp,
    UnaryOperation,
};
use super::structured::{
    AssignmentTarget, StructuredScript, StructuredStmt, SwitchCaseStmt, parse_type_annotation,
};
use crate::cache_bail as bail;
use crate::error::Result;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    self, AssignmentOperator, BindingPattern, CallExpression, Declaration, ExpressionStatement,
    FormalParameter, Function, ImportDeclaration, ImportDeclarationSpecifier, Statement,
    VariableDeclarationKind,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

pub fn parse_structured_typescript(source: &str) -> Result<StructuredScript> {
    let allocator = Allocator::default();
    let source_type = SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.errors.is_empty() {
        let messages = parsed
            .errors
            .into_iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!("TypeScript parse failed:\n{messages}");
    }

    let mut imports = Vec::new();
    let mut structured: Option<StructuredScript> = None;
    for statement in &parsed.program.body {
        match statement {
            Statement::ImportDeclaration(decl) => imports.push(parse_import(decl)?),
            Statement::ExportNamedDeclaration(decl) => {
                let Some(Declaration::FunctionDeclaration(function)) = &decl.declaration else {
                    bail!("only export function declarations are supported");
                };
                structured = Some(parse_function(function, source, imports.clone())?);
            }
            Statement::FunctionDeclaration(function) => {
                structured = Some(parse_function(function, source, imports.clone())?);
            }
            Statement::EmptyStatement(_) => {}
            other => bail!("unsupported top-level statement: {:?}", other),
        }
    }

    if let Some(structured) = structured {
        Ok(structured)
    } else {
        bail!("missing exported function declaration");
    }
}

fn parse_import(decl: &ImportDeclaration<'_>) -> Result<ImportStatement> {
    let Some(specifiers) = &decl.specifiers else {
        bail!("side-effect imports are not supported");
    };
    let mut named_exports = Vec::new();
    for specifier in specifiers {
        match specifier {
            ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                named_exports.push(specifier.local.name.to_string());
            }
            _ => bail!("only named imports are supported"),
        }
    }
    Ok(ImportStatement {
        module: decl.source.value.to_string(),
        named_exports,
        is_type_only: matches!(decl.import_kind, ast::ImportOrExportKind::Type),
    })
}

fn parse_function(
    function: &Function<'_>,
    source: &str,
    imports: Vec<ImportStatement>,
) -> Result<StructuredScript> {
    let Some(function_id) = function.id.as_ref() else {
        bail!("structured function must be named");
    };
    let function_name = function_id.name.to_string();
    let arguments = function
        .params
        .items
        .iter()
        .enumerate()
        .map(|(index, param)| parse_argument_variable(index, param, source))
        .collect::<Result<Vec<_>>>()?;
    let return_type = function.return_type.as_ref().map_or_else(
        || "void".to_string(),
        |annotation| type_from_span(annotation.span, source),
    );
    let Some(body) = function.body.as_ref() else {
        bail!("structured function body missing");
    };

    let mut locals = Vec::new();
    let mut arrays = Vec::new();
    let mut body_stmts = Vec::new();
    let mut parsing_decls = true;
    for statement in &body.statements {
        if parsing_decls && let Some(parsed) = parse_preamble_declaration(statement, source)? {
            match parsed {
                PreambleDecl::Local(local) => locals.push(local),
                PreambleDecl::Array(id) => arrays.push(id),
            }
            continue;
        }
        parsing_decls = false;
        body_stmts.push(parse_statement(statement, source)?);
    }

    Ok(StructuredScript {
        script_id: ScriptId(0),
        raw_name: None,
        header_comments: Vec::new(),
        imports,
        function_name,
        arguments,
        locals,
        arrays,
        return_type,
        body: body_stmts,
    })
}

enum PreambleDecl {
    Local(LocalVariable),
    Array(u32),
}

fn parse_preamble_declaration(
    statement: &Statement<'_>,
    source: &str,
) -> Result<Option<PreambleDecl>> {
    let Statement::VariableDeclaration(decl) = statement else {
        return Ok(None);
    };
    if decl.kind != VariableDeclarationKind::Let || decl.declarations.len() != 1 {
        bail!("only single let declarations are supported in structured preamble");
    }
    let declarator = &decl.declarations[0];
    let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
        bail!("destructuring locals are not supported");
    };
    let name = identifier.name.to_string();
    if let Some(array_id) = name.strip_prefix("array_")
        && let Some(init) = &declarator.init
        && matches!(init, ast::Expression::ArrayExpression(_))
    {
        return Ok(Some(PreambleDecl::Array(array_id.parse::<u32>()?)));
    }
    if declarator.init.is_some() {
        bail!("local declarations must not have initializers");
    }
    let type_annotation = declarator
        .type_annotation
        .as_ref()
        .map_or(TypeAnnotation::Unknown, |annotation| {
            parse_type_annotation(&type_from_span(annotation.span, source))
        });
    Ok(Some(PreambleDecl::Local(LocalVariable {
        index: parse_local_index(&name)?,
        name,
        type_annotation,
    })))
}

fn parse_argument_variable(
    index: usize,
    param: &FormalParameter<'_>,
    source: &str,
) -> Result<ArgumentVariable> {
    let BindingPattern::BindingIdentifier(identifier) = &param.pattern else {
        bail!("only identifier parameters are supported");
    };
    let type_annotation = param
        .type_annotation
        .as_ref()
        .map_or(TypeAnnotation::Unknown, |annotation| {
            parse_type_annotation(&type_from_span(annotation.span, source))
        });
    Ok(ArgumentVariable {
        index,
        name: identifier.name.to_string(),
        type_annotation,
    })
}

fn parse_statement(statement: &Statement<'_>, source: &str) -> Result<StructuredStmt> {
    match statement {
        Statement::BlockStatement(block) => {
            bail!(
                "unexpected bare block statement with {} statements",
                block.body.len()
            )
        }
        Statement::ExpressionStatement(expr) => parse_expression_statement(expr, source),
        Statement::IfStatement(stmt) => Ok(StructuredStmt::If {
            condition: parse_expression(&stmt.test, source)?,
            then_body: parse_statement_block(&stmt.consequent, source)?,
            else_body: stmt
                .alternate
                .as_ref()
                .map(|stmt| parse_statement_block(stmt, source))
                .transpose()?,
        }),
        Statement::WhileStatement(stmt) => {
            let condition = parse_expression(&stmt.test, source)?;
            if !matches!(
                condition,
                Expression::BooleanLiteral(BooleanLiteral { value: true })
            ) {
                bail!("only while (true) loops are supported");
            }
            Ok(StructuredStmt::While {
                body: parse_statement_block(&stmt.body, source)?,
            })
        }
        Statement::SwitchStatement(stmt) => {
            let mut cases = Vec::new();
            for case in &stmt.cases {
                let Some(test) = &case.test else {
                    bail!("default switch cases are not supported");
                };
                let Expression::NumberLiteral(value) = parse_expression(test, source)? else {
                    bail!("switch case values must be numeric literals");
                };
                let mut body = parse_statement_list(&case.consequent, source)?;
                if matches!(body.last(), Some(StructuredStmt::Break)) {
                    body.pop();
                }
                cases.push(SwitchCaseStmt {
                    value: value.value,
                    body,
                });
            }
            Ok(StructuredStmt::Switch {
                expr: parse_expression(&stmt.discriminant, source)?,
                cases,
            })
        }
        Statement::ReturnStatement(stmt) => Ok(StructuredStmt::Return {
            value: stmt
                .argument
                .as_ref()
                .map(|expr| parse_expression(expr, source))
                .transpose()?,
        }),
        Statement::BreakStatement(_) => Ok(StructuredStmt::Break),
        Statement::ContinueStatement(_) => Ok(StructuredStmt::Continue),
        Statement::EmptyStatement(_) => Ok(StructuredStmt::Comment(String::new())),
        other => bail!("unsupported structured statement: {:?}", other),
    }
}

fn parse_statement_block(statement: &Statement<'_>, source: &str) -> Result<Vec<StructuredStmt>> {
    match statement {
        Statement::BlockStatement(block) => parse_statement_list(&block.body, source),
        other => Ok(vec![parse_statement(other, source)?]),
    }
}

fn parse_statement_list(statements: &[Statement<'_>], source: &str) -> Result<Vec<StructuredStmt>> {
    let mut structured = Vec::with_capacity(statements.len());
    for statement in statements {
        let parsed = parse_statement(statement, source)?;
        if !matches!(&parsed, StructuredStmt::Comment(text) if text.is_empty()) {
            structured.push(parsed);
        }
    }
    Ok(structured)
}

fn parse_expression_statement(
    statement: &ExpressionStatement<'_>,
    source: &str,
) -> Result<StructuredStmt> {
    match &statement.expression {
        ast::Expression::AssignmentExpression(expr) => {
            if expr.operator != AssignmentOperator::Assign {
                bail!("compound assignments are not supported");
            }
            Ok(StructuredStmt::Assignment {
                target: parse_assignment_target(&expr.left, source)?,
                value: parse_expression(&expr.right, source)?,
            })
        }
        expr => Ok(StructuredStmt::Expr {
            expr: parse_expression(expr, source)?,
        }),
    }
}

fn parse_assignment_target(
    target: &ast::AssignmentTarget<'_>,
    source: &str,
) -> Result<AssignmentTarget> {
    match target {
        ast::AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
            Ok(AssignmentTarget::Identifier(identifier.name.to_string()))
        }
        ast::AssignmentTarget::ComputedMemberExpression(expr) => {
            let ast::Expression::Identifier(array) = &expr.object else {
                bail!("only identifier array targets are supported");
            };
            Ok(AssignmentTarget::ArrayAccess {
                array: array.name.to_string(),
                index: parse_expression(&expr.expression, source)?,
            })
        }
        _ => bail!("unsupported assignment target"),
    }
}

fn parse_expression(expression: &ast::Expression<'_>, source: &str) -> Result<Expression> {
    match expression {
        ast::Expression::NumericLiteral(value) => Ok(Expression::NumberLiteral(NumberLiteral {
            value: numeric_literal_to_i32(value, source)?,
        })),
        ast::Expression::BigIntLiteral(value) => Ok(Expression::BigIntLiteral(BigIntLiteral {
            value: slice_span(value.span, source)
                .trim_end_matches('n')
                .parse::<i64>()?,
        })),
        ast::Expression::StringLiteral(value) => Ok(Expression::StringLiteral(StringLiteral {
            value: value.value.to_string(),
        })),
        ast::Expression::BooleanLiteral(value) => Ok(Expression::BooleanLiteral(BooleanLiteral {
            value: value.value,
        })),
        ast::Expression::Identifier(identifier) => Ok(Expression::Identifier(Identifier {
            name: identifier.name.to_string(),
        })),
        ast::Expression::CallExpression(call) => parse_call_expression(call, source),
        ast::Expression::BinaryExpression(binary) => {
            Ok(Expression::BinaryOperation(BinaryOperation {
                op: parse_binary_op(binary.operator)?,
                left: Box::new(parse_expression(&binary.left, source)?),
                right: Box::new(parse_expression(&binary.right, source)?),
            }))
        }
        ast::Expression::LogicalExpression(logical) => {
            Ok(Expression::BinaryOperation(BinaryOperation {
                op: match logical.operator {
                    ast::LogicalOperator::And => BinaryOp::And,
                    ast::LogicalOperator::Or => BinaryOp::Or,
                    ast::LogicalOperator::Coalesce => bail!("unsupported logical operator"),
                },
                left: Box::new(parse_expression(&logical.left, source)?),
                right: Box::new(parse_expression(&logical.right, source)?),
            }))
        }
        ast::Expression::UnaryExpression(unary) => Ok(Expression::UnaryOperation(UnaryOperation {
            op: match unary.operator {
                ast::UnaryOperator::UnaryNegation => UnaryOp::Neg,
                ast::UnaryOperator::LogicalNot => UnaryOp::Not,
                _ => bail!("unsupported unary operator"),
            },
            operand: Box::new(parse_expression(&unary.argument, source)?),
        })),
        ast::Expression::ParenthesizedExpression(expr) => {
            parse_expression(&expr.expression, source)
        }
        ast::Expression::ComputedMemberExpression(expr) => {
            Ok(Expression::ArrayAccess(ArrayAccess {
                array: Box::new(parse_expression(&expr.object, source)?),
                index: Box::new(parse_expression(&expr.expression, source)?),
            }))
        }
        ast::Expression::StaticMemberExpression(expr) => {
            Ok(Expression::PropertyAccess(PropertyAccess {
                object: Box::new(parse_expression(&expr.object, source)?),
                property: expr.property.name.to_string(),
            }))
        }
        ast::Expression::TSAsExpression(expr) => parse_expression(&expr.expression, source),
        ast::Expression::TSNonNullExpression(expr) => parse_expression(&expr.expression, source),
        ast::Expression::TSTypeAssertion(expr) => parse_expression(&expr.expression, source),
        ast::Expression::TSSatisfiesExpression(expr) => parse_expression(&expr.expression, source),
        other => bail!("unsupported structured expression: {:?}", other),
    }
}

fn parse_call_expression(call: &CallExpression<'_>, source: &str) -> Result<Expression> {
    if let ast::Expression::Identifier(identifier) = &call.callee
        && identifier.name == "callback"
    {
        return parse_callback_expression(call, source);
    }

    let callee = parse_expression(&call.callee, source)?;
    let arguments = call
        .arguments
        .iter()
        .map(|argument| parse_argument_expression(argument, source))
        .collect::<Result<Vec<_>>>()?;

    Ok(Expression::Call(CallExpr {
        callee: Box::new(callee),
        arguments,
    }))
}

fn parse_callback_expression(call: &CallExpression<'_>, source: &str) -> Result<Expression> {
    match call.arguments.as_slice() {
        [
            ast::Argument::StringLiteral(script),
            ast::Argument::ArrayExpression(watchers),
        ] => Ok(Expression::CallbackLiteral(CallbackLiteral {
            script: script.value.to_string(),
            script_id: None,
            raw_descriptor: String::new(),
            arguments: Vec::new(),
            watchers: parse_watcher_elements(&watchers.elements, source)?,
        })),
        [
            ast::Argument::StringLiteral(script),
            ast::Argument::ArrayExpression(args),
            ast::Argument::ArrayExpression(watchers),
            ast::Argument::StringLiteral(raw),
        ] => Ok(Expression::CallbackLiteral(CallbackLiteral {
            script: script.value.to_string(),
            script_id: None,
            raw_descriptor: raw.value.to_string(),
            arguments: parse_expression_elements(&args.elements, source)?,
            watchers: parse_watcher_elements(&watchers.elements, source)?,
        })),
        _ => bail!("callback signature must be callback(name, args, watchers, descriptor)"),
    }
}

fn parse_expression_elements(
    elements: &[ast::ArrayExpressionElement<'_>],
    source: &str,
) -> Result<Vec<Expression>> {
    let mut values = Vec::with_capacity(elements.len());
    for element in elements {
        match element {
            ast::ArrayExpressionElement::SpreadElement(_)
            | ast::ArrayExpressionElement::Elision(_) => {
                bail!("spread and sparse callback argument arrays are not supported")
            }
            ast::ArrayExpressionElement::BooleanLiteral(value) => {
                values.push(Expression::BooleanLiteral(BooleanLiteral {
                    value: value.value,
                }));
            }
            ast::ArrayExpressionElement::NumericLiteral(value) => {
                values.push(Expression::NumberLiteral(NumberLiteral {
                    value: numeric_literal_to_i32(value, source)?,
                }));
            }
            ast::ArrayExpressionElement::BigIntLiteral(value) => {
                values.push(Expression::BigIntLiteral(BigIntLiteral {
                    value: slice_span(value.span, source)
                        .trim_end_matches('n')
                        .parse::<i64>()?,
                }));
            }
            ast::ArrayExpressionElement::StringLiteral(value) => {
                values.push(Expression::StringLiteral(StringLiteral {
                    value: value.value.to_string(),
                }));
            }
            ast::ArrayExpressionElement::Identifier(identifier) => {
                values.push(Expression::Identifier(Identifier {
                    name: identifier.name.to_string(),
                }));
            }
            ast::ArrayExpressionElement::CallExpression(value) => {
                values.push(parse_call_expression(value, source)?);
            }
            ast::ArrayExpressionElement::BinaryExpression(value) => {
                values.push(Expression::BinaryOperation(BinaryOperation {
                    op: parse_binary_op(value.operator)?,
                    left: Box::new(parse_expression(&value.left, source)?),
                    right: Box::new(parse_expression(&value.right, source)?),
                }));
            }
            ast::ArrayExpressionElement::UnaryExpression(value) => {
                values.push(Expression::UnaryOperation(UnaryOperation {
                    op: match value.operator {
                        ast::UnaryOperator::UnaryNegation => UnaryOp::Neg,
                        ast::UnaryOperator::LogicalNot => UnaryOp::Not,
                        _ => bail!("unsupported unary operator"),
                    },
                    operand: Box::new(parse_expression(&value.argument, source)?),
                }));
            }
            ast::ArrayExpressionElement::ComputedMemberExpression(value) => {
                values.push(Expression::ArrayAccess(ArrayAccess {
                    array: Box::new(parse_expression(&value.object, source)?),
                    index: Box::new(parse_expression(&value.expression, source)?),
                }));
            }
            ast::ArrayExpressionElement::StaticMemberExpression(value) => {
                values.push(Expression::PropertyAccess(PropertyAccess {
                    object: Box::new(parse_expression(&value.object, source)?),
                    property: value.property.name.to_string(),
                }));
            }
            ast::ArrayExpressionElement::ParenthesizedExpression(value) => {
                values.push(parse_expression(&value.expression, source)?);
            }
            ast::ArrayExpressionElement::TSAsExpression(value) => {
                values.push(parse_expression(&value.expression, source)?);
            }
            ast::ArrayExpressionElement::TSNonNullExpression(value) => {
                values.push(parse_expression(&value.expression, source)?);
            }
            ast::ArrayExpressionElement::TSTypeAssertion(value) => {
                values.push(parse_expression(&value.expression, source)?);
            }
            ast::ArrayExpressionElement::TSSatisfiesExpression(value) => {
                values.push(parse_expression(&value.expression, source)?);
            }
            other => bail!("unsupported callback argument expression: {:?}", other),
        }
    }
    Ok(values)
}

fn parse_watcher_elements(
    elements: &[ast::ArrayExpressionElement<'_>],
    source: &str,
) -> Result<Vec<String>> {
    let mut watchers = Vec::with_capacity(elements.len());
    for element in elements {
        let watcher = match element {
            ast::ArrayExpressionElement::Identifier(identifier) => identifier.name.to_string(),
            ast::ArrayExpressionElement::StaticMemberExpression(value) => format!(
                "{}.{}",
                slice_span(value.object.span(), source),
                value.property.name
            ),
            ast::ArrayExpressionElement::ComputedMemberExpression(value) => format!(
                "{}[{}]",
                slice_span(value.object.span(), source),
                slice_span(value.expression.span(), source)
            ),
            other => bail!("unsupported callback watcher expression: {:?}", other),
        };
        watchers.push(watcher);
    }
    Ok(watchers)
}

fn parse_argument_expression(argument: &ast::Argument<'_>, source: &str) -> Result<Expression> {
    match argument {
        ast::Argument::SpreadElement(_) => bail!("spread arguments are not supported"),
        ast::Argument::BooleanLiteral(value) => Ok(Expression::BooleanLiteral(BooleanLiteral {
            value: value.value,
        })),
        ast::Argument::NumericLiteral(value) => Ok(Expression::NumberLiteral(NumberLiteral {
            value: numeric_literal_to_i32(value, source)?,
        })),
        ast::Argument::BigIntLiteral(value) => Ok(Expression::BigIntLiteral(BigIntLiteral {
            value: slice_span(value.span, source)
                .trim_end_matches('n')
                .parse::<i64>()?,
        })),
        ast::Argument::StringLiteral(value) => Ok(Expression::StringLiteral(StringLiteral {
            value: value.value.to_string(),
        })),
        ast::Argument::Identifier(identifier) => Ok(Expression::Identifier(Identifier {
            name: identifier.name.to_string(),
        })),
        ast::Argument::CallExpression(call) => parse_call_expression(call, source),
        ast::Argument::BinaryExpression(binary) => {
            Ok(Expression::BinaryOperation(BinaryOperation {
                op: parse_binary_op(binary.operator)?,
                left: Box::new(parse_expression(&binary.left, source)?),
                right: Box::new(parse_expression(&binary.right, source)?),
            }))
        }
        ast::Argument::LogicalExpression(logical) => {
            Ok(Expression::BinaryOperation(BinaryOperation {
                op: match logical.operator {
                    ast::LogicalOperator::And => BinaryOp::And,
                    ast::LogicalOperator::Or => BinaryOp::Or,
                    ast::LogicalOperator::Coalesce => bail!("unsupported logical operator"),
                },
                left: Box::new(parse_expression(&logical.left, source)?),
                right: Box::new(parse_expression(&logical.right, source)?),
            }))
        }
        ast::Argument::UnaryExpression(unary) => Ok(Expression::UnaryOperation(UnaryOperation {
            op: match unary.operator {
                ast::UnaryOperator::UnaryNegation => UnaryOp::Neg,
                ast::UnaryOperator::LogicalNot => UnaryOp::Not,
                _ => bail!("unsupported unary operator"),
            },
            operand: Box::new(parse_expression(&unary.argument, source)?),
        })),
        ast::Argument::ParenthesizedExpression(expr) => parse_expression(&expr.expression, source),
        ast::Argument::ComputedMemberExpression(expr) => Ok(Expression::ArrayAccess(ArrayAccess {
            array: Box::new(parse_expression(&expr.object, source)?),
            index: Box::new(parse_expression(&expr.expression, source)?),
        })),
        ast::Argument::StaticMemberExpression(expr) => {
            Ok(Expression::PropertyAccess(PropertyAccess {
                object: Box::new(parse_expression(&expr.object, source)?),
                property: expr.property.name.to_string(),
            }))
        }
        ast::Argument::TSAsExpression(expr) => parse_expression(&expr.expression, source),
        ast::Argument::TSNonNullExpression(expr) => parse_expression(&expr.expression, source),
        ast::Argument::TSTypeAssertion(expr) => parse_expression(&expr.expression, source),
        ast::Argument::TSSatisfiesExpression(expr) => parse_expression(&expr.expression, source),
        other => bail!("unsupported call argument: {:?}", other),
    }
}

fn parse_binary_op(op: ast::BinaryOperator) -> Result<BinaryOp> {
    Ok(match op {
        ast::BinaryOperator::Addition => BinaryOp::Add,
        ast::BinaryOperator::Subtraction => BinaryOp::Sub,
        ast::BinaryOperator::Multiplication => BinaryOp::Mul,
        ast::BinaryOperator::Division => BinaryOp::Div,
        ast::BinaryOperator::Remainder => BinaryOp::Mod,
        ast::BinaryOperator::Equality => BinaryOp::Eq,
        ast::BinaryOperator::Inequality => BinaryOp::Ne,
        ast::BinaryOperator::LessThan => BinaryOp::Lt,
        ast::BinaryOperator::LessEqualThan => BinaryOp::Le,
        ast::BinaryOperator::GreaterThan => BinaryOp::Gt,
        ast::BinaryOperator::GreaterEqualThan => BinaryOp::Ge,
        _ => bail!("unsupported binary operator"),
    })
}

fn parse_local_index(name: &str) -> Result<usize> {
    let Some((_, suffix)) = name.rsplit_once('_') else {
        bail!("local name missing numeric suffix: {name}");
    };
    suffix.parse::<usize>().map_err(Into::into)
}

fn type_from_span(span: oxc_span::Span, source: &str) -> String {
    slice_span(span, source)
        .trim()
        .trim_start_matches(':')
        .trim()
        .to_string()
}

fn slice_span(span: oxc_span::Span, source: &str) -> &str {
    &source[span.start as usize..span.end as usize]
}

/// Convert an oxc-parsed numeric literal to the i32 a CS2 int operand holds.
/// oxc has already parsed the literal value (handling hex/binary/octal/`_`
/// separators), so we work from that rather than re-parsing the raw span with
/// `i32::parse` (which rejects `0xFFFFFF` colours and full-u32 bitmasks). The
/// full unsigned range `0..=u32::MAX` is accepted and reinterpreted as i32.
fn numeric_literal_to_i32(literal: &ast::NumericLiteral<'_>, source: &str) -> Result<i32> {
    let value = literal.value;
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(u32::MAX) {
        bail!(
            "unsupported numeric literal `{}` (expected an integer in 0..=4294967295)",
            slice_span(literal.span, source)
        );
    }
    Ok(value as u32 as i32)
}
