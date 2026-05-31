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
            let mut default_body = None;
            for case in &stmt.cases {
                let mut body = parse_statement_list(&case.consequent, source)?;
                let has_trailing_break = matches!(body.last(), Some(StructuredStmt::Break));
                if has_trailing_break {
                    body.pop();
                }
                if let Some(test) = &case.test {
                    let value = parse_switch_case_value(parse_expression(test, source)?)?;
                    let fallthrough = !has_trailing_break && body.is_empty();
                    cases.push(SwitchCaseStmt {
                        value,
                        body,
                        fallthrough,
                        break_after: has_trailing_break,
                    });
                } else {
                    if default_body.is_some() {
                        bail!("duplicate default switch case");
                    }
                    default_body = Some(body);
                }
            }
            Ok(StructuredStmt::Switch {
                expr: parse_expression(&stmt.discriminant, source)?,
                cases,
                default_body,
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
        // `goto(N)` / `label(N)` are the linear control-flow markers (a jump and
        // its target); parse them back to dedicated statements rather than
        // generic calls so they round-trip and lower to branches/labels.
        ast::Expression::CallExpression(call) => {
            if let Some(target) = control_marker_target(call, "goto") {
                Ok(StructuredStmt::Goto { target })
            } else if let Some(target) = control_marker_target(call, "label") {
                Ok(StructuredStmt::Label { target })
            } else if let Some((target, values)) = stack_goto_marker(call, source)? {
                Ok(StructuredStmt::StackGoto { target, values })
            } else {
                Ok(StructuredStmt::Expr {
                    expr: parse_expression(&statement.expression, source)?,
                })
            }
        }
        expr => Ok(StructuredStmt::Expr {
            expr: parse_expression(expr, source)?,
        }),
    }
}

/// If `call` is `name(<integer>)`, return the integer target; else `None`.
fn control_marker_target(call: &CallExpression<'_>, name: &str) -> Option<usize> {
    let ast::Expression::Identifier(ident) = &call.callee else {
        return None;
    };
    if ident.name.as_str() != name || call.arguments.len() != 1 {
        return None;
    }
    let ast::Argument::NumericLiteral(num) = &call.arguments[0] else {
        return None;
    };
    let value = num.value;
    if value < 0.0 || value.fract() != 0.0 {
        return None;
    }
    Some(value as usize)
}

fn stack_goto_marker(
    call: &CallExpression<'_>,
    source: &str,
) -> Result<Option<(usize, Vec<Expression>)>> {
    let ast::Expression::Identifier(ident) = &call.callee else {
        return Ok(None);
    };
    if ident.name.as_str() != "stackpush_then" {
        return Ok(None);
    }
    let Some((last, values)) = call.arguments.split_last() else {
        return Ok(None);
    };
    let ast::Argument::CallExpression(goto_call) = last else {
        return Ok(None);
    };
    let Some(target) = control_marker_target(goto_call, "goto") else {
        return Ok(None);
    };
    let values = values
        .iter()
        .map(|argument| parse_argument_expression(argument, source))
        .collect::<Result<Vec<_>>>()?;
    Ok(Some((target, values)))
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

fn parse_switch_case_value(expr: Expression) -> Result<i32> {
    match expr {
        Expression::NumberLiteral(value) => Ok(value.value),
        Expression::UnaryOperation(unary) if unary.op == UnaryOp::Neg => {
            let Expression::NumberLiteral(value) = *unary.operand else {
                bail!("switch case values must be numeric literals");
            };
            Ok(-value.value)
        }
        _ => bail!("switch case values must be numeric literals"),
    }
}

fn parse_expression(expression: &ast::Expression<'_>, source: &str) -> Result<Expression> {
    match expression {
        ast::Expression::NumericLiteral(value) => Ok(Expression::NumberLiteral(NumberLiteral {
            value: numeric_literal_to_i32(value, source)?,
        })),
        ast::Expression::BigIntLiteral(value) => Ok(Expression::BigIntLiteral(BigIntLiteral {
            value: bigint_literal_to_i64(value, source)?,
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
        ast::Expression::UnaryExpression(unary) => {
            parse_unary_expression(unary.operator, &unary.argument, source)
        }
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
                    value: bigint_literal_to_i64(value, source)?,
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
                values.push(parse_unary_expression(
                    value.operator,
                    &value.argument,
                    source,
                )?);
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
            value: bigint_literal_to_i64(value, source)?,
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
        ast::Argument::UnaryExpression(unary) => {
            parse_unary_expression(unary.operator, &unary.argument, source)
        }
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

fn parse_unary_expression(
    operator: ast::UnaryOperator,
    argument: &ast::Expression<'_>,
    source: &str,
) -> Result<Expression> {
    let op = parse_unary_op(operator)?;
    if op == UnaryOp::Neg
        && let ast::Expression::BigIntLiteral(literal) = argument
    {
        return Ok(Expression::BigIntLiteral(BigIntLiteral {
            value: negated_bigint_literal_to_i64(literal, source)?,
        }));
    }
    Ok(Expression::UnaryOperation(UnaryOperation {
        op,
        operand: Box::new(parse_expression(argument, source)?),
    }))
}

fn parse_unary_op(operator: ast::UnaryOperator) -> Result<UnaryOp> {
    match operator {
        ast::UnaryOperator::UnaryNegation => Ok(UnaryOp::Neg),
        ast::UnaryOperator::LogicalNot => Ok(UnaryOp::Not),
        _ => bail!("unsupported unary operator"),
    }
}

fn bigint_literal_to_i64(literal: &ast::BigIntLiteral<'_>, source: &str) -> Result<i64> {
    bigint_literal_digits(literal, source)
        .parse::<i64>()
        .map_err(Into::into)
}

fn negated_bigint_literal_to_i64(literal: &ast::BigIntLiteral<'_>, source: &str) -> Result<i64> {
    const I64_MIN_ABS: u64 = 9_223_372_036_854_775_808;

    let magnitude = bigint_literal_digits(literal, source).parse::<u64>()?;
    if magnitude == I64_MIN_ABS {
        Ok(i64::MIN)
    } else {
        Ok(-i64::try_from(magnitude)?)
    }
}

fn bigint_literal_digits<'a>(literal: &ast::BigIntLiteral<'_>, source: &'a str) -> &'a str {
    slice_span(literal.span, source).trim_end_matches('n')
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

#[cfg(test)]
mod tests {
    use super::parse_structured_typescript;
    use crate::transpile::ast::Expression;
    use crate::transpile::structured::StructuredStmt;

    #[test]
    fn parses_negative_switch_case_value() {
        let script = parse_structured_typescript(
            "export function script0(): void {\n    let local_int_0: number;\n    switch (local_int_0) {\n        case -1:\n            return;\n    }\n}\n",
        )
        .expect("negative switch case should parse");

        assert!(matches!(
            &script.body[0],
            StructuredStmt::Switch { cases, .. } if cases[0].value == -1
        ));
    }

    #[test]
    fn parses_switch_default_case() {
        let script = parse_structured_typescript(
            "export function script0(): void {\n    let local_int_0: number;\n    switch (local_int_0) {\n        case 1:\n            return;\n        default:\n            camreset();\n            break;\n    }\n}\n",
        )
        .expect("default switch case should parse");

        assert!(matches!(
            &script.body[0],
            StructuredStmt::Switch {
                default_body: Some(body),
                ..
            } if body.len() == 1
        ));
    }

    #[test]
    fn parses_min_i64_bigint_literal() {
        let script = parse_structured_typescript(
            "export function script0(): bigint {\n    return longconst(-9223372036854775808n);\n}\n",
        )
        .expect("min i64 bigint should parse");

        let StructuredStmt::Return {
            value: Some(Expression::Call(call)),
        } = &script.body[0]
        else {
            panic!("expected return call");
        };
        assert!(matches!(
            call.arguments.as_slice(),
            [Expression::BigIntLiteral(value)] if value.value == i64::MIN
        ));
    }

    #[test]
    fn parses_stackpush_then_goto_as_stack_goto() {
        let script = parse_structured_typescript(
            "export function script0(): void {\n    stackpush_then(18, 1, goto(42));\n}\n",
        )
        .expect("stack goto should parse");

        assert!(matches!(
            &script.body[0],
            StructuredStmt::StackGoto { target: 42, values } if values.len() == 2
        ));
    }
}
