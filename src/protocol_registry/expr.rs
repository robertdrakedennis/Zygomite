//! The payload size-expression mini-language (DSL v2 §2.2): tokenizer,
//! recursive-descent parser, validator, and canonical TS emitter.

use crate::error::Result;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// DSL v2 expression grammar (Stage 8 §2.2)
//
//   expr := term (op term)*           op := '&' | '|' | '<<' | '>>' | '+' | '-'
//   term := ident | ident '[' INT ']' | INT | HEX | '(' expr ')'
//
// String-ops only (no regex, no new deps). The parser validates an arg against
// the grammar, records identifiers + array accesses for bounds checking, and
// re-emits a canonical TS expression with `!` appended to every array access
// (strict-tsc non-null assertion). v1 plain param/literal args are the
// degenerate single-term case.
// ---------------------------------------------------------------------------

/// One token of an in-grammar expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprTok {
    Ident(String),
    /// Integer literal, kept as written (decimal or `0x` hex).
    Int(String),
    Op(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
}

/// A parsed expression AST node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Ident(String),
    Int(String),
    /// `ident[index]` array access.
    Index(String, u32),
    /// `(inner)` explicit grouping.
    Paren(Box<Self>),
    /// `lhs op rhs`, left-associated as written (no precedence folding — the
    /// source parenthesizes explicitly and we reproduce that grouping).
    Bin(Box<Self>, String, Box<Self>),
}

/// Tokenize an expression. Returns an error for any character outside the
/// grammar's alphabet (ternaries, `===`, `*`, `/`, etc.), which is how those
/// packets stay hand-written.
fn tokenize_expr(text: &str) -> Result<Vec<ExprTok>> {
    let bytes: Vec<char> = text.chars().collect();
    let mut toks: Vec<ExprTok> = Vec::new();
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        let c = bytes[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push(ExprTok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(ExprTok::RParen);
                i += 1;
            }
            '[' => {
                toks.push(ExprTok::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(ExprTok::RBracket);
                i += 1;
                // Consume a TS non-null assertion `!` after `]` (no-op).
                if i < n && bytes[i] == '!' {
                    i += 1;
                }
            }
            '<' | '>' => {
                if i + 1 < n && bytes[i + 1] == c {
                    toks.push(ExprTok::Op(format!("{c}{c}")));
                    i += 2;
                } else {
                    return Err(crate::error::CacheError::message(format!(
                        "expression: stray `{c}` (only `<<`/`>>` admitted)"
                    )));
                }
            }
            '&' | '|' | '+' | '-' => {
                toks.push(ExprTok::Op(c.to_string()));
                i += 1;
            }
            '0'..='9' => {
                let start = i;
                if c == '0' && i + 1 < n && (bytes[i + 1] == 'x' || bytes[i + 1] == 'X') {
                    i += 2;
                    let hstart = i;
                    while i < n && bytes[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    if i == hstart {
                        return Err(crate::error::CacheError::message(
                            "expression: malformed hex literal".to_owned(),
                        ));
                    }
                } else {
                    while i < n && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                toks.push(ExprTok::Int(bytes[start..i].iter().collect()));
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                toks.push(ExprTok::Ident(bytes[start..i].iter().collect()));
            }
            _ => {
                return Err(crate::error::CacheError::message(format!(
                    "expression: illegal character {c:?}"
                )));
            }
        }
    }
    Ok(toks)
}

/// Recursive-descent parser over the token list. Mirrors the survey's parser.
struct ExprParser {
    toks: Vec<ExprTok>,
    pos: usize,
}

impl ExprParser {
    fn new(toks: Vec<ExprTok>) -> Self {
        Self { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&ExprTok> {
        self.toks.get(self.pos)
    }

    fn parse(&mut self) -> Result<Expr> {
        let e = self.parse_expr()?;
        if self.pos != self.toks.len() {
            return Err(crate::error::CacheError::message(format!(
                "expression: trailing tokens at {}",
                self.pos
            )));
        }
        Ok(e)
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        let mut left = self.parse_term()?;
        while let Some(ExprTok::Op(op)) = self.peek() {
            let op = op.clone();
            self.pos += 1;
            let right = self.parse_term()?;
            left = Expr::Bin(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr> {
        match self.peek().cloned() {
            Some(ExprTok::LParen) => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                if self.peek() != Some(&ExprTok::RParen) {
                    return Err(crate::error::CacheError::message(
                        "expression: missing `)`".to_owned(),
                    ));
                }
                self.pos += 1;
                Ok(Expr::Paren(Box::new(inner)))
            }
            Some(ExprTok::Int(v)) => {
                self.pos += 1;
                Ok(Expr::Int(v))
            }
            Some(ExprTok::Ident(name)) => {
                self.pos += 1;
                if self.peek() == Some(&ExprTok::LBracket) {
                    self.pos += 1;
                    let idx = match self.peek().cloned() {
                        Some(ExprTok::Int(v)) if !v.starts_with("0x") && !v.starts_with("0X") => {
                            self.pos += 1;
                            v.parse::<u32>().map_err(|_| {
                                crate::error::CacheError::message(
                                    "expression: array index out of range".to_owned(),
                                )
                            })?
                        }
                        _ => {
                            return Err(crate::error::CacheError::message(
                                "expression: array index must be a decimal literal".to_owned(),
                            ));
                        }
                    };
                    if self.peek() != Some(&ExprTok::RBracket) {
                        return Err(crate::error::CacheError::message(
                            "expression: missing `]`".to_owned(),
                        ));
                    }
                    self.pos += 1;
                    Ok(Expr::Index(name, idx))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            other => Err(crate::error::CacheError::message(format!(
                "expression: unexpected token {other:?}"
            ))),
        }
    }
}

/// Parse an in-grammar expression string into an AST.
pub fn parse_expr(text: &str) -> Result<Expr> {
    let toks = tokenize_expr(text)?;
    if toks.is_empty() {
        return Err(crate::error::CacheError::message(
            "expression: empty".to_owned(),
        ));
    }
    ExprParser::new(toks).parse()
}

/// Validate every identifier and array access in an expression against the
/// declared params. `scalars` are non-array param names; `arrays` maps an
/// array param name to its declared element count (from the default).
pub fn validate_expr(
    expr: &Expr,
    scalars: &std::collections::HashSet<&str>,
    arrays: &BTreeMap<&str, usize>,
    ctx: &str,
) -> Result<()> {
    match expr {
        Expr::Int(_) => Ok(()),
        Expr::Ident(name) => {
            if scalars.contains(name.as_str()) {
                Ok(())
            } else if arrays.contains_key(name.as_str()) {
                Err(crate::error::CacheError::message(format!(
                    "{ctx}: array param `{name}` used without an index"
                )))
            } else {
                Err(crate::error::CacheError::message(format!(
                    "{ctx}: `{name}` is not a declared param"
                )))
            }
        }
        Expr::Index(name, idx) => {
            let Some(&len) = arrays.get(name.as_str()) else {
                return Err(crate::error::CacheError::message(format!(
                    "{ctx}: `{name}[{idx}]` indexes a non-array param"
                )));
            };
            if (*idx as usize) >= len {
                return Err(crate::error::CacheError::message(format!(
                    "{ctx}: `{name}[{idx}]` index out of bounds (length {len})"
                )));
            }
            Ok(())
        }
        Expr::Paren(inner) => validate_expr(inner, scalars, arrays, ctx),
        Expr::Bin(l, _, r) => {
            validate_expr(l, scalars, arrays, ctx)?;
            validate_expr(r, scalars, arrays, ctx)
        }
    }
}

/// Emit an expression as canonical TS: operators spaced, parens reproduced,
/// `!` appended to every array access for strict tsc.
pub fn emit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Ident(name) => name.clone(),
        Expr::Int(v) => v.clone(),
        Expr::Index(name, idx) => format!("{name}[{idx}]!"),
        Expr::Paren(inner) => format!("({})", emit_expr(inner)),
        Expr::Bin(l, op, r) => format!("{} {op} {}", emit_expr(l), emit_expr(r)),
    }
}

/// Emit an expression in the survey's canonical schema form: identical to
/// [`emit_expr`] except array accesses carry **no** trailing `!`
/// (`name[idx]`, not `name[idx]!`).
///
/// This mirrors the Python survey's `_ExprParser` re-emission, which is what the
/// `payloads.json` `arg` field stores. The strict-tsc `!` is a generator-only
/// concern ([`emit_expr`]) and must not leak into the survey output.
pub fn emit_schema(expr: &Expr) -> String {
    match expr {
        Expr::Ident(name) => name.clone(),
        Expr::Int(v) => v.clone(),
        Expr::Index(name, idx) => format!("{name}[{idx}]"),
        Expr::Paren(inner) => format!("({})", emit_schema(inner)),
        Expr::Bin(l, op, r) => format!("{} {op} {}", emit_schema(l), emit_schema(r)),
    }
}
