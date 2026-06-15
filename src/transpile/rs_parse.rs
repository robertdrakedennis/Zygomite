//! G1.3 — RuneScript parser.
//!
//! Inverts [`super::runescript::render_runescript`]: RuneScript text → a [`StructuredScript`]
//! that lowers (via `ts_lower::lower_structured_script`) to the same bytecode as the TS surface.
//! The parser only has to invert the *emitter's* grammar (the sole producer of this text), and it
//! does so by **rule** — no registry: `enum_getoutputcount` → strip underscores →
//! `enumgetoutputcount`; `cc_x`/`if_x` → `UI.<sanitize_camel(x)>`; `~name` → `Call(name)`; `$int4`
//! plus the header arg-counts → `local_int_2`. Because both surfaces produce the same IR, the
//! existing byte-fidelity gate validates the round-trip. See
//! `plans/tooling/cs2-runescript-decompiler.md` (G1.3).

use super::ast::{
    ArgumentVariable, ArrayAccess, BigIntLiteral, BinaryOp, BinaryOperation, BooleanLiteral,
    CallExpr, CallbackLiteral, Expression, GotoExpr, Identifier, ImportStatement, LocalVariable,
    NumberLiteral, PropertyAccess, ScriptId, StringLiteral, UnaryOp, UnaryOperation,
};
use super::runescript::RuneScriptContext;
use super::structured::{
    AssignmentTarget, StructuredScript, StructuredStmt, SwitchCaseStmt, parse_type_annotation,
};
use super::types::{BaseVarType, lattice};
use crate::cache_bail as bail;
use crate::error::Result;
use std::collections::{HashMap, HashSet};

/// CS2 local-variable stack domains, mirrored from the emitter. The decoder name token
/// (`int`/`obj`/`long`) and the per-domain index fix a local's bytecode slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Domain {
    Int,
    Obj,
    Long,
}

impl Domain {
    /// The decoder name token for the domain (`local_<token>_<n>`).
    fn token(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Obj => "obj",
            Self::Long => "long",
        }
    }

    /// Map a RuneScript type keyword (`int`/`string`/`long` or a semantic type like `component`)
    /// to its stack domain via the type lattice.
    fn from_type_keyword(keyword: &str) -> Self {
        match lattice().by_name(keyword).base() {
            Some(BaseVarType::Long) => Self::Long,
            Some(BaseVarType::String | BaseVarType::CoordFine) => Self::Obj,
            _ => Self::Int,
        }
    }
}

// ── lexer ──

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(i64),
    Long(i64),
    Str(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Dot,
    Colon,
    Tilde,
    Eq,
    Bang,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
}

fn lex(source: &str) -> Result<Vec<Tok>> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '"' {
            let (value, next) = lex_string(source, i)?;
            out.push(Tok::Str(value));
            i = next;
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                i += 1;
            }
            let digits = &source[start..i];
            if i < bytes.len() && bytes[i] == b'L' {
                i += 1;
                // `i64::MIN` renders as `-9223372036854775808L`; the lexer sees the magnitude
                // `9223372036854775808` (= `i64::MAX + 1`) as an unsigned token (the `-` is a
                // separate `Tok::Minus`), which overflows `i64::from_str`. Fall back to `u64`
                // reinterpreted two's-complement: `9223372036854775808u64 as i64 == i64::MIN`, and
                // `parse_atom`'s `wrapping_neg` then yields `i64::MIN` exactly. Exact for all other
                // values (`i64::from_str` succeeds, the fallback never runs).
                let v = digits
                    .parse::<i64>()
                    .or_else(|_| digits.parse::<u64>().map(|u| u as i64))?;
                out.push(Tok::Long(v));
            } else {
                out.push(Tok::Num(digits.parse()?));
            }
            continue;
        }
        // `%var` reference — only when `%` is immediately followed by an identifier char; a bare
        // `%` is the modulo operator.
        if c == '%' && i + 1 < bytes.len() && is_ident_continue(bytes[i + 1] as char) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i] as char) {
                i += 1;
            }
            out.push(Tok::Ident(source[start..i].to_string()));
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i] as char) {
                i += 1;
            }
            out.push(Tok::Ident(source[start..i].to_string()));
            continue;
        }
        // operators / punctuation
        let two = if i + 1 < bytes.len() {
            &source[i..i + 2]
        } else {
            ""
        };
        match two {
            "<=" => {
                out.push(Tok::Le);
                i += 2;
                continue;
            }
            ">=" => {
                out.push(Tok::Ge);
                i += 2;
                continue;
            }
            _ => {}
        }
        let tok = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            ',' => Tok::Comma,
            ';' => Tok::Semi,
            '.' => Tok::Dot,
            ':' => Tok::Colon,
            '~' => Tok::Tilde,
            '=' => Tok::Eq,
            '!' => Tok::Bang,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '&' => Tok::Amp,
            '|' => Tok::Pipe,
            other => bail!("rs_parse: unexpected character {other:?}"),
        };
        out.push(tok);
        i += 1;
    }
    Ok(out)
}

fn lex_string(source: &str, start: usize) -> Result<(String, usize)> {
    // Char-aware so multi-byte UTF-8 in string contents (e.g. a non-breaking space) survives.
    let rest = &source[start + 1..]; // skip opening quote
    let mut value = String::new();
    let mut chars = rest.char_indices();
    while let Some((offset, c)) = chars.next() {
        if c == '"' {
            // absolute byte index just past the closing quote
            return Ok((value, start + 1 + offset + 1));
        }
        if c == '\\'
            && let Some((_, esc)) = chars.next()
        {
            value.push(match esc {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            continue;
        }
        value.push(c);
    }
    bail!("rs_parse: unterminated string literal")
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// ── parser ──

/// Parse RuneScript source (as produced by `render_runescript`) into a [`StructuredScript`]. The
/// `ctx` (the same the emitter used) supplies the canonical opcode set so command names un-strip
/// correctly.
pub fn parse_runescript(source: &str, ctx: &RuneScriptContext) -> Result<StructuredScript> {
    let toks = lex(source)?;
    let mut parser = Parser {
        ctx,
        toks,
        pos: 0,
        arg_counts: HashMap::new(),
        locals: Vec::new(),
        local_names: HashMap::new(),
        arrays: Vec::new(),
        gosub_names: HashSet::new(),
    };
    parser.parse_script()
}

struct Parser<'a> {
    ctx: &'a RuneScriptContext,
    toks: Vec<Tok>,
    pos: usize,
    arg_counts: HashMap<Domain, usize>,
    locals: Vec<LocalVariable>,
    /// decoder local name → its position in `locals` (dedupe def_s, first wins).
    local_names: HashMap<String, usize>,
    arrays: Vec<u32>,
    /// `~name` gosub callees — reconstructed into `imports` so `ts_lower` resolves a script call
    /// whose name collides with a command opcode (`error`, `fromdate`, …) as a gosub, not the command.
    gosub_names: HashSet<String>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let tok = self.toks.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, expected: &Tok) -> Result<()> {
        match self.next() {
            Some(ref tok) if tok == expected => Ok(()),
            other => bail!("rs_parse: expected {expected:?}, found {other:?}"),
        }
    }

    fn eat_ident(&mut self) -> Result<String> {
        match self.next() {
            Some(Tok::Ident(name)) => Ok(name),
            other => bail!("rs_parse: expected identifier, found {other:?}"),
        }
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Ident(name)) => Some(name.as_str()),
            _ => None,
        }
    }

    fn parse_script(&mut self) -> Result<StructuredScript> {
        let function_name = self.eat_ident()?;

        // Up to two paren groups: `(params)` then `(returns)`. One group is always params; a
        // returns-only script renders `name()(ret)` (empty params group first).
        let mut groups: Vec<Vec<(String, Option<String>)>> = Vec::new();
        while matches!(self.peek(), Some(Tok::LParen)) {
            groups.push(self.parse_paren_decl_group()?);
            if groups.len() == 2 {
                break;
            }
        }

        let (param_group, return_group) = match groups.len() {
            0 => (Vec::new(), Vec::new()),
            1 => (groups.remove(0), Vec::new()),
            _ => {
                let returns = groups.remove(1);
                (groups.remove(0), returns)
            }
        };

        // Header params define the arguments. Index per domain in declaration order; the arg name
        // is reconstructed from (domain, index) — the rendered `$name` is redundant with it.
        let mut arguments = Vec::new();
        let mut per_domain: HashMap<Domain, usize> = HashMap::new();
        for (ty, _name) in &param_group {
            let domain = Domain::from_type_keyword(ty);
            let index = *per_domain.entry(domain).or_default();
            *per_domain.entry(domain).or_default() += 1;
            arguments.push(ArgumentVariable {
                index,
                name: format!("arg_{}_{index}", domain.token()),
                type_annotation: parse_type_annotation(ty),
            });
        }
        self.arg_counts = per_domain;

        let return_type = build_return_type(&return_group);

        let body = self.parse_stmts_until_eof()?;

        // Reconstruct the gosub callee imports (collected while parsing `~name` calls). `ts_lower`
        // treats an imported identifier as a script call; without this a gosub to a script whose name
        // is also a command opcode lowers as that command. Sorted for determinism.
        let mut gosub_names: Vec<String> = self.gosub_names.iter().cloned().collect();
        gosub_names.sort();
        let imports = gosub_names
            .into_iter()
            .map(|name| ImportStatement {
                module: format!("./{name}"),
                named_exports: vec![name],
                is_type_only: false,
            })
            .collect();

        Ok(StructuredScript {
            script_id: ScriptId(0),
            raw_name: None,
            header_comments: Vec::new(),
            imports,
            function_name,
            arguments,
            locals: std::mem::take(&mut self.locals),
            arrays: std::mem::take(&mut self.arrays),
            return_type,
            body,
        })
    }

    /// Parse `(type $name, …)` (params) or `(type, …)` (returns). Each entry is (type, opt-name).
    fn parse_paren_decl_group(&mut self) -> Result<Vec<(String, Option<String>)>> {
        self.eat(&Tok::LParen)?;
        let mut entries = Vec::new();
        while !matches!(self.peek(), Some(Tok::RParen)) {
            let mut ty = self.eat_ident()?;
            // A union return type (`number | void`) is rendered verbatim by the emitter — the
            // decompiler models scripts whose paths mix `return;` and `return <value>;` this way,
            // and `return_types` never splits on `|`. Capture the whole union as one type token so
            // `build_return_type` passes it through unchanged and the emitter re-renders it
            // identically. Returns don't affect bytecode; params never contain `|`.
            while matches!(self.peek(), Some(Tok::Pipe)) {
                self.pos += 1;
                ty.push_str(" | ");
                ty.push_str(&self.eat_ident()?);
            }
            let name = if let Some(Tok::Ident(n)) = self.peek() {
                let n = n.clone();
                self.pos += 1;
                Some(n)
            } else {
                None
            };
            entries.push((ty, name));
            if matches!(self.peek(), Some(Tok::Comma)) {
                self.pos += 1;
            }
        }
        self.eat(&Tok::RParen)?;
        Ok(entries)
    }

    fn parse_stmts_until_eof(&mut self) -> Result<Vec<StructuredStmt>> {
        let mut stmts = Vec::new();
        while self.peek().is_some() {
            if let Some(stmt) = self.parse_stmt()? {
                stmts.push(stmt);
            }
        }
        Ok(stmts)
    }

    fn parse_block(&mut self) -> Result<Vec<StructuredStmt>> {
        self.eat(&Tok::LBrace)?;
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Some(Tok::RBrace)) {
            if self.peek().is_none() {
                bail!("rs_parse: unterminated block");
            }
            if let Some(stmt) = self.parse_stmt()? {
                stmts.push(stmt);
            }
        }
        self.eat(&Tok::RBrace)?;
        Ok(stmts)
    }

    /// Parse one statement. Returns `None` for a bare local declaration (`def_<ty> $name;`), which
    /// registers a local but contributes no body statement.
    fn parse_stmt(&mut self) -> Result<Option<StructuredStmt>> {
        let stmt = match self.peek_ident() {
            Some("if") => self.parse_if()?,
            Some("while") => self.parse_while()?,
            Some("switch_int") => self.parse_switch()?,
            Some("goto") => self.parse_goto()?,
            Some("label") => self.parse_label()?,
            Some("return") => self.parse_return()?,
            Some("break") => {
                self.pos += 1;
                self.eat(&Tok::Semi)?;
                StructuredStmt::Break
            }
            Some("continue") => {
                self.pos += 1;
                self.eat(&Tok::Semi)?;
                StructuredStmt::Continue
            }
            Some("stackpush_then") => self.parse_stackpush_then_stmt()?,
            Some(kw) if kw.starts_with("def_") => return self.parse_def(),
            _ => self.parse_assignment_or_expr()?,
        };
        Ok(Some(stmt))
    }

    fn parse_if(&mut self) -> Result<StructuredStmt> {
        self.pos += 1; // if
        self.eat(&Tok::LParen)?;
        let condition = self.parse_expr(0)?;
        self.eat(&Tok::RParen)?;
        let then_body = self.parse_block()?;
        let else_body = if self.peek_ident() == Some("else") {
            self.pos += 1;
            Some(self.parse_block()?)
        } else {
            None
        };
        Ok(StructuredStmt::If {
            condition,
            then_body,
            else_body,
        })
    }

    fn parse_while(&mut self) -> Result<StructuredStmt> {
        self.pos += 1; // while
        self.eat(&Tok::LParen)?;
        // condition is always `true` (our structurer emits while(true) + breaks)
        let _ = self.parse_expr(0)?;
        self.eat(&Tok::RParen)?;
        let body = self.parse_block()?;
        Ok(StructuredStmt::While { body })
    }

    fn parse_switch(&mut self) -> Result<StructuredStmt> {
        self.pos += 1; // switch_int
        self.eat(&Tok::LParen)?;
        let expr = self.parse_expr(0)?;
        self.eat(&Tok::RParen)?;
        self.eat(&Tok::LBrace)?;
        let mut cases = Vec::new();
        let mut default_body = None;
        while !matches!(self.peek(), Some(Tok::RBrace)) {
            if self.peek_ident() != Some("case") {
                bail!("rs_parse: expected `case` in switch");
            }
            self.pos += 1; // case
            if self.peek_ident() == Some("default") {
                self.pos += 1;
                self.eat(&Tok::Colon)?;
                default_body = Some(self.parse_case_body()?);
            } else {
                let value = self.parse_signed_int()?;
                self.eat(&Tok::Colon)?;
                let body = self.parse_case_body()?;
                // An empty case body is a fallthrough group (`case A: case B: <body>`): `ts_lower`
                // shares the next case's jump target only when `fallthrough && body.is_empty()`.
                // Without this, each value gets its own target → the switch operand (and every
                // downstream branch offset) diverges.
                let fallthrough = body.is_empty();
                cases.push(SwitchCaseStmt {
                    value,
                    body,
                    fallthrough,
                    break_after: false,
                });
            }
        }
        self.eat(&Tok::RBrace)?;
        Ok(StructuredStmt::Switch {
            expr,
            cases,
            default_body,
        })
    }

    /// A switch case body runs until the next `case`/`default` or the closing `}`.
    fn parse_case_body(&mut self) -> Result<Vec<StructuredStmt>> {
        let mut stmts = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RBrace) => break,
                Some(Tok::Ident(kw)) if kw == "case" => break,
                None => bail!("rs_parse: unterminated switch case"),
                _ => {
                    if let Some(stmt) = self.parse_stmt()? {
                        stmts.push(stmt);
                    }
                }
            }
        }
        Ok(stmts)
    }

    fn parse_goto(&mut self) -> Result<StructuredStmt> {
        self.pos += 1;
        self.eat(&Tok::LParen)?;
        let target = self.parse_usize()?;
        self.eat(&Tok::RParen)?;
        self.eat(&Tok::Semi)?;
        Ok(StructuredStmt::Goto { target })
    }

    fn parse_label(&mut self) -> Result<StructuredStmt> {
        self.pos += 1;
        self.eat(&Tok::LParen)?;
        let target = self.parse_usize()?;
        self.eat(&Tok::RParen)?;
        self.eat(&Tok::Semi)?;
        Ok(StructuredStmt::Label { target })
    }

    fn parse_return(&mut self) -> Result<StructuredStmt> {
        self.pos += 1;
        let value = if matches!(self.peek(), Some(Tok::LParen)) {
            self.eat(&Tok::LParen)?;
            let expr = self.parse_expr(0)?;
            self.eat(&Tok::RParen)?;
            Some(expr)
        } else {
            None
        };
        self.eat(&Tok::Semi)?;
        Ok(StructuredStmt::Return { value })
    }

    /// `stackpush_then(…)` at statement level is either a `StackGoto` (the last argument is
    /// `goto(N)`) or a plain stack-pseudo call statement.
    fn parse_stackpush_then_stmt(&mut self) -> Result<StructuredStmt> {
        let call = self.parse_primary()?; // stackpush_then(args) via the command-call path
        self.eat(&Tok::Semi)?;
        if let Expression::Call(c) = &call
            && let Some(Expression::GotoExpr(g)) = c.arguments.last()
        {
            let target = g.target;
            let values = c.arguments[..c.arguments.len() - 1].to_vec();
            return Ok(StructuredStmt::StackGoto { target, values });
        }
        Ok(StructuredStmt::Expr { expr: call })
    }

    fn parse_def(&mut self) -> Result<Option<StructuredStmt>> {
        let keyword = self.eat_ident()?; // def_int / def_string / def_long / def_<semantic>
        let Some(ty) = keyword.strip_prefix("def_") else {
            bail!("rs_parse: bad def keyword: {keyword}");
        };
        let ty = ty.to_string();
        let name = self.eat_ident()?; // $arr0 or $int4
        let Some(body) = name.strip_prefix('$') else {
            bail!("rs_parse: def target not a local: {name}");
        };

        if let Some(arr_str) = body.strip_prefix("arr") {
            // array declaration: `def_<ty> $arr<index>(size)`, or `$arr<index>_<typelow>` when the
            // define_array operand carries non-zero element-type bits (the access uses the bare index
            // but the operand is the full `(index << 16) | typelow`).
            let (index, type_low): (u32, u32) = match arr_str.split_once('_') {
                Some((i, t)) => (i.parse()?, t.parse()?),
                None => (arr_str.parse()?, 0),
            };
            let raw = (index << 16) | type_low;
            self.eat(&Tok::LParen)?;
            let size = self.parse_expr(0)?;
            self.eat(&Tok::RParen)?;
            self.eat(&Tok::Semi)?;
            if !self.arrays.contains(&raw) {
                self.arrays.push(raw);
            }
            return Ok(Some(StructuredStmt::Expr {
                expr: Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: format!("define_array_{raw}"),
                    })),
                    arguments: vec![size],
                }),
            }));
        }

        let decoder = self.local_decoder_name(&name)?;
        self.register_local(&decoder, &ty);

        // bare declaration `def_<ty> $name;` — registers a local but emits no statement.
        if matches!(self.peek(), Some(Tok::Semi)) {
            self.pos += 1;
            return Ok(None);
        }

        // local declaration-assignment: `def_<ty> $<type><idx> = expr`
        self.eat(&Tok::Eq)?;
        let value = self.parse_expr(0)?;
        self.eat(&Tok::Semi)?;
        Ok(Some(StructuredStmt::Assignment {
            target: AssignmentTarget::Identifier(decoder),
            value,
        }))
    }

    fn register_local(&mut self, decoder: &str, ty: &str) {
        if self.local_names.contains_key(decoder) {
            return;
        }
        let index = decoder
            .rsplit('_')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let slot = self.locals.len();
        self.local_names.insert(decoder.to_string(), slot);
        self.locals.push(LocalVariable {
            index,
            name: decoder.to_string(),
            type_annotation: parse_type_annotation(ty),
        });
    }

    fn parse_assignment_or_expr(&mut self) -> Result<StructuredStmt> {
        let lhs = self.parse_primary()?;
        if matches!(self.peek(), Some(Tok::Eq)) {
            self.pos += 1;
            let value = self.parse_expr(0)?;
            self.eat(&Tok::Semi)?;
            let target = expr_to_target(lhs)?;
            return Ok(StructuredStmt::Assignment { target, value });
        }
        self.eat(&Tok::Semi)?;
        Ok(StructuredStmt::Expr { expr: lhs })
    }

    // ── expressions (precedence climber; calc/paren are transparent) ──

    fn parse_expr(&mut self, min_prec: i32) -> Result<Expression> {
        let mut left = self.parse_primary()?;
        while let Some(op) = self.peek_binary_op() {
            let prec = op_prec(op);
            if prec < min_prec {
                break;
            }
            self.consume_binary_op();
            let right = self.parse_expr(prec + 1)?;
            left = Expression::BinaryOperation(BinaryOperation {
                op,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    /// A primary is an atom followed by any number of `.property` postfixes.
    fn parse_primary(&mut self) -> Result<Expression> {
        let atom = self.parse_atom()?;
        self.parse_postfix(atom)
    }

    fn parse_postfix(&mut self, mut base: Expression) -> Result<Expression> {
        // `.property` and trailing `(args)` postfixes. The latter is the decompiler's multi-return
        // indexing — `window_getinsets()(3)` = the 3rd return value — which `ts_lower` lowers as a
        // call on a call. (Postfix-call was reverted in G1.5 when a getter's `(mode)` flipped cc_/if_;
        // the cc_/if_ casing fixes since make the inner re-emit stable, re-enabled + re-measured here.)
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.eat(&Tok::Dot)?;
                    let property = self.eat_ident()?;
                    base = Expression::PropertyAccess(PropertyAccess {
                        object: Box::new(base),
                        property,
                    });
                }
                Some(Tok::LParen) => {
                    let arguments = self.parse_call_args()?;
                    // CS2 has no first-class functions — a call result is never itself callable —
                    // so `cmd(args)(N)` is the decompiler's multi-return indexing (the N-th return
                    // value). Reconstruct the `ArrayAccess(Call, N)` IR the TS surface emits and that
                    // `MultiResultAccess::from_expr` recognises, so a run of `local = cmd()(k)`
                    // assignments collapses to one call + pops instead of bailing on a `Call(Call)`
                    // callee. Only a single numeric index qualifies; anything else stays a `Call`.
                    let multi_return_index = match (&base, arguments.as_slice()) {
                        (Expression::Call(_), [Expression::NumberLiteral(index)]) => {
                            Some(index.clone())
                        }
                        _ => None,
                    };
                    base = if let Some(index) = multi_return_index {
                        Expression::ArrayAccess(ArrayAccess {
                            array: Box::new(base),
                            index: Box::new(Expression::NumberLiteral(index)),
                        })
                    } else {
                        Expression::Call(CallExpr {
                            callee: Box::new(base),
                            arguments,
                        })
                    };
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn parse_atom(&mut self) -> Result<Expression> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.pos += 1;
                match self.peek() {
                    // `-N` folds to a negative literal (the common case)
                    Some(Tok::Num(v)) => {
                        let v = *v;
                        self.pos += 1;
                        Ok(Expression::NumberLiteral(NumberLiteral {
                            value: i32::try_from(-v).unwrap_or(i32::MIN),
                        }))
                    }
                    Some(Tok::Long(v)) => {
                        let v = *v;
                        self.pos += 1;
                        // `wrapping_neg` so `-9223372036854775808L` (lexed as `i64::MIN` via the
                        // two's-complement `u64` fallback) negates to `i64::MIN`, not a panic.
                        Ok(Expression::BigIntLiteral(BigIntLiteral {
                            value: v.wrapping_neg(),
                        }))
                    }
                    // `-<expr>` (e.g. `--1` = neg of a negative literal, or `-$x`) is a real negation
                    _ => Ok(Expression::UnaryOperation(UnaryOperation {
                        op: UnaryOp::Neg,
                        operand: Box::new(self.parse_atom()?),
                    })),
                }
            }
            Some(Tok::Num(_)) => {
                let Some(Tok::Num(v)) = self.next() else {
                    unreachable!()
                };
                Ok(Expression::NumberLiteral(NumberLiteral {
                    value: i32::try_from(v).unwrap_or(i32::MAX),
                }))
            }
            Some(Tok::Long(_)) => {
                let Some(Tok::Long(v)) = self.next() else {
                    unreachable!()
                };
                Ok(Expression::BigIntLiteral(BigIntLiteral { value: v }))
            }
            Some(Tok::Str(_)) => {
                let Some(Tok::Str(s)) = self.next() else {
                    unreachable!()
                };
                Ok(Expression::StringLiteral(StringLiteral { value: s }))
            }
            Some(Tok::LParen) => {
                // a precedence-grouping paren the emitter added — transparent
                self.eat(&Tok::LParen)?;
                let inner = self.parse_expr(0)?;
                self.eat(&Tok::RParen)?;
                Ok(inner)
            }
            Some(Tok::Tilde) => self.parse_gosub(),
            Some(Tok::Ident(_)) => self.parse_ident_primary(),
            other => bail!("rs_parse: unexpected token in expression: {other:?}"),
        }
    }

    fn parse_gosub(&mut self) -> Result<Expression> {
        self.eat(&Tok::Tilde)?;
        let name = self.eat_ident()?;
        self.gosub_names.insert(name.clone());
        let arguments = if matches!(self.peek(), Some(Tok::LParen)) {
            self.parse_call_args()?
        } else {
            Vec::new()
        };
        // A gosub keeps its script name verbatim (no underscore stripping).
        Ok(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier { name })),
            arguments,
        }))
    }

    fn parse_ident_primary(&mut self) -> Result<Expression> {
        let name = self.eat_ident()?;

        // calc(...) is a transparent wrapper — just yield the inner expression.
        if name == "calc" {
            self.eat(&Tok::LParen)?;
            let inner = self.parse_expr(0)?;
            self.eat(&Tok::RParen)?;
            return Ok(inner);
        }
        if name == "true" || name == "false" {
            return Ok(Expression::BooleanLiteral(BooleanLiteral {
                value: name == "true",
            }));
        }
        if name == "pop" && matches!(self.peek(), Some(Tok::LParen)) {
            self.eat(&Tok::LParen)?;
            self.eat(&Tok::RParen)?;
            return Ok(Expression::Call(CallExpr {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "pop".to_string(),
                })),
                arguments: Vec::new(),
            }));
        }
        if name == "callback" && matches!(self.peek(), Some(Tok::LParen)) {
            return self.parse_callback();
        }
        // `goto(N)` can appear inside a `stackpush_then(…)` argument list as a `GotoExpr`.
        if name == "goto" && matches!(self.peek(), Some(Tok::LParen)) {
            self.eat(&Tok::LParen)?;
            let target = self.parse_usize()?;
            self.eat(&Tok::RParen)?;
            return Ok(Expression::GotoExpr(GotoExpr { target }));
        }

        // `$local` or `$arr<n>(idx)`
        if let Some(body) = name.strip_prefix('$') {
            if let Some(index_str) = body.strip_prefix("arr") {
                let index: u32 = index_str.parse()?;
                self.eat(&Tok::LParen)?;
                let idx = self.parse_expr(0)?;
                self.eat(&Tok::RParen)?;
                return Ok(Expression::ArrayAccess(ArrayAccess {
                    array: Box::new(Expression::Identifier(Identifier {
                        name: format!("array_{index}"),
                    })),
                    index: Box::new(idx),
                }));
            }
            let decoder = self.local_decoder_name(&name)?;
            return Ok(Expression::Identifier(Identifier { name: decoder }));
        }

        // `%var`
        if let Some(var) = name.strip_prefix('%') {
            return Ok(Expression::Identifier(Identifier {
                name: var.to_string(),
            }));
        }

        // (`Enum_X.KEY_Y` property access is handled by the postfix `.` in `parse_primary`.)

        // command call `name(args)` (UI or generic) — or a bare reference.
        if matches!(self.peek(), Some(Tok::LParen)) {
            let arguments = self.parse_call_args()?;
            return Ok(Expression::Call(CallExpr {
                callee: Box::new(command_callee(self.ctx, &name)),
                arguments,
            }));
        }

        Ok(Expression::Identifier(Identifier { name }))
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expression>> {
        self.eat(&Tok::LParen)?;
        let mut args = Vec::new();
        while !matches!(self.peek(), Some(Tok::RParen)) {
            args.push(self.parse_expr(0)?);
            if matches!(self.peek(), Some(Tok::Comma)) {
                self.pos += 1;
            }
        }
        self.eat(&Tok::RParen)?;
        Ok(args)
    }

    /// `callback("script", [args], [watchers], "descriptor")` — the round-trippable hook form.
    fn parse_callback(&mut self) -> Result<Expression> {
        self.eat(&Tok::LParen)?;
        let script = self.eat_string()?;
        self.eat(&Tok::Comma)?;
        let arguments = self.parse_bracket_exprs()?;
        self.eat(&Tok::Comma)?;
        let watchers = self.parse_bracket_idents()?;
        self.eat(&Tok::Comma)?;
        let raw_descriptor = self.eat_string()?;
        self.eat(&Tok::RParen)?;
        Ok(Expression::CallbackLiteral(CallbackLiteral {
            script,
            script_id: None,
            raw_descriptor,
            arguments,
            watchers,
        }))
    }

    fn eat_string(&mut self) -> Result<String> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(s),
            other => bail!("rs_parse: expected string literal, found {other:?}"),
        }
    }

    fn parse_bracket_exprs(&mut self) -> Result<Vec<Expression>> {
        self.eat(&Tok::LBracket)?;
        let mut out = Vec::new();
        while !matches!(self.peek(), Some(Tok::RBracket)) {
            out.push(self.parse_expr(0)?);
            if matches!(self.peek(), Some(Tok::Comma)) {
                self.pos += 1;
            }
        }
        self.eat(&Tok::RBracket)?;
        Ok(out)
    }

    fn parse_bracket_idents(&mut self) -> Result<Vec<String>> {
        self.eat(&Tok::LBracket)?;
        let mut out = Vec::new();
        while !matches!(self.peek(), Some(Tok::RBracket)) {
            // A watcher is a raw decompiler name and may be a dotted constant like
            // `Enum_16086.BOSS_PETS`, not just a bare identifier.
            let mut name = self.eat_ident()?;
            while matches!(self.peek(), Some(Tok::Dot)) {
                self.eat(&Tok::Dot)?;
                name.push('.');
                name.push_str(&self.eat_ident()?);
            }
            out.push(name);
            if matches!(self.peek(), Some(Tok::Comma)) {
                self.pos += 1;
            }
        }
        self.eat(&Tok::RBracket)?;
        Ok(out)
    }

    /// Invert a `$<type><index>` local name to its decoder name (`arg_int_0` / `local_int_2`)
    /// using the header per-domain argument counts.
    fn local_decoder_name(&self, rs_name: &str) -> Result<String> {
        let Some(body) = rs_name.strip_prefix('$') else {
            bail!("rs_parse: local not $-prefixed: {rs_name}");
        };
        let Some(split) = body.find(|c: char| c.is_ascii_digit()) else {
            bail!("rs_parse: local missing index: {rs_name}");
        };
        let type_str = &body[..split];
        let index: usize = body[split..].parse()?;
        let domain = Domain::from_type_keyword(type_str);
        let arg_count = self.arg_counts.get(&domain).copied().unwrap_or(0);
        if index < arg_count {
            Ok(format!("arg_{}_{index}", domain.token()))
        } else {
            Ok(format!("local_{}_{}", domain.token(), index - arg_count))
        }
    }

    fn peek_binary_op(&self) -> Option<BinaryOp> {
        Some(match self.peek()? {
            Tok::Eq => BinaryOp::Eq,
            Tok::Bang => BinaryOp::Ne,
            Tok::Lt => BinaryOp::Lt,
            Tok::Le => BinaryOp::Le,
            Tok::Gt => BinaryOp::Gt,
            Tok::Ge => BinaryOp::Ge,
            Tok::Pipe => BinaryOp::Or,
            Tok::Amp => BinaryOp::And,
            Tok::Plus => BinaryOp::Add,
            Tok::Minus => BinaryOp::Sub,
            Tok::Star => BinaryOp::Mul,
            Tok::Slash => BinaryOp::Div,
            Tok::Percent => BinaryOp::Mod,
            _ => return None,
        })
    }

    fn consume_binary_op(&mut self) {
        self.pos += 1;
    }

    fn parse_usize(&mut self) -> Result<usize> {
        match self.next() {
            Some(Tok::Num(v)) if v >= 0 => Ok(v as usize),
            other => bail!("rs_parse: expected non-negative integer, found {other:?}"),
        }
    }

    fn parse_signed_int(&mut self) -> Result<i32> {
        let negative = matches!(self.peek(), Some(Tok::Minus));
        if negative {
            self.pos += 1;
        }
        match self.next() {
            Some(Tok::Num(v)) => {
                let v = if negative { -v } else { v };
                Ok(i32::try_from(v).unwrap_or(i32::MAX))
            }
            other => bail!("rs_parse: expected integer, found {other:?}"),
        }
    }
}

/// Build the TS-form `return_type` string from RuneScript return type keywords.
fn build_return_type(returns: &[(String, Option<String>)]) -> String {
    if returns.is_empty() {
        return "void".to_string();
    }
    let mapped: Vec<String> = returns
        .iter()
        .map(|(ty, _)| runescript_type_to_ts(ty))
        .collect();
    if mapped.len() == 1 {
        mapped.into_iter().next().unwrap_or_else(|| "void".to_string())
    } else {
        format!("[{}]", mapped.join(", "))
    }
}

fn runescript_type_to_ts(ty: &str) -> String {
    match ty {
        "int" => "number",
        "string" => "string",
        "long" => "bigint",
        "boolean" => "boolean",
        other => other,
    }
    .to_string()
}

/// Reconstruct the TS callee identifier for a command name. `cc_x`/`if_x` → `UI.<method>`, where the
/// method's **first-letter casing carries the cc_/if_ distinction** for `ts_lower::resolve_ui_command`
/// (cc_ → lowercase-first, if_ → capital-first). `sanitize_camel` capitalises the first letter, which
/// is right for `if_`; lower it back for `cc_`. (Without this the parser always produced capital-first
/// → `ts_lower` read every UI call as `if_` → the ~800-script cc_→if_ byte-gate flips.)
/// A real non-UI opcode un-strips to its TS spelling; a synthetic (`stackassign_2`, …) passes through.
fn command_callee(ctx: &RuneScriptContext, name: &str) -> Expression {
    // Curated UI methods whose exact camelCase `ts_lower` expects can't be recovered from the
    // underscore'd opcode (`sanitize_camel("setparam_int")` → `setparamInt`, but `ts_lower`'s arm
    // wants `setParamInt` — no word boundary between `set` and `param` in the opcode name).
    match name {
        "cc_setparam" => return ui_property("setParam".to_string()),
        "cc_setparam_int" => return ui_property("setParamInt".to_string()),
        "cc_setparam_string" => return ui_property("setParamString".to_string()),
        // `if_find Byte(mode)` is only ever emitted for `UI.findInterface(component, mode)` (the
        // emitter renders the 1-arg `find` → `if_find Byte(0)` form as `cc_find`). The generic
        // `if_` → `sanitize_camel` path would yield `Find`, which `ts_lower::resolve_ui_command`
        // matches to `cc_find` (2 stack args) — `findInterface`'s mode is a Byte *operand*, not a
        // stack arg, so only the explicit `findInterface` arm emits `if_find`. Name it exactly.
        "if_find" => return ui_property("findInterface".to_string()),
        // The only two cc_/if_-prefixed multi-return getters: the decompiler renders these as BARE
        // command identifiers (via `push_indexed_call_results`), not `UI.<method>`. They appear as
        // `cmd(args)(k)` indexing, and `MultiResultAccess::from_expr` resolves a bare-identifier
        // callee but not a `UI.` PropertyAccess one — so the generic `if_`/`cc_` → property branch
        // below would leave an unlowerable `ArrayAccess(Call(PropertyAccess))`. Keep them bare.
        "cc_getcharposatindex" | "if_getcharposatindex" => {
            return Expression::Identifier(Identifier {
                name: super::sanitize_ts_ident(&strip_underscores(name)),
            });
        }
        _ => {}
    }
    // The method's first-letter casing carries the cc_/if_ distinction for
    // `ts_lower::resolve_ui_command`: cc_ → lowercase-first, if_ → capital-first. `sanitize_camel`
    // capitalises the first letter (right for `if_`); lower it back for `cc_`.
    if let Some(suffix) = name.strip_prefix("cc_") {
        // Hooks (`seton*`) are routed by `method.contains("Seton")` in `ts_lower` (which also strips a
        // trailing `WithMode` first), so they MUST stay capital-first or the callback falls through to
        // a plain arg and bails. A handful of curated non-WithMode multi-word opcodes likewise only
        // resolve via the capital-first `resolve_ui_command` path. (A blanket "capital-first for any
        // underscore'd suffix" traded ~322 WithMode/setter failures for ~27 wins — reverted; this is the
        // targeted allowlist instead. The hook/resolve arms re-derive cc_/if_ by arg-count, byte-safe.)
        const CAPITAL_FIRST: [&str; 5] = [
            "find_parent",
            "setobject_nonum",
            "setobject_alwaysnum",
            "button_setcantoggle",
            "button_settoggled",
        ];
        // The emitter renders a `WithMode` method via `to_snake`, appending `_with_mode` to the
        // opcode (`cc_setobject_nonum_with_mode`). For these multi-word opcodes that suffix defeats
        // both the allowlist *and* `ts_lower`'s lowercase fallback (which can't reinsert the lost
        // `setobject_nonum` underscore). Test the base (sans `_with_mode`) and capital-first the
        // whole name — `ts_lower` strips `WithMode`, then resolves `SetobjectNonum` via
        // `resolve_ui_command`. A blanket "capital-first every multi-word cc_ opcode" regressed hard
        // (26→293: most resolve fine lowercase and capital-first misroutes them), so this stays a
        // targeted allowlist of the opcodes whose lowercase fallback genuinely can't round-trip.
        let base = suffix.strip_suffix("_with_mode").unwrap_or(suffix);
        if suffix.starts_with("seton") || CAPITAL_FIRST.contains(&base) {
            return ui_property(sanitize_camel(suffix));
        }
        return ui_property(lower_first(&sanitize_camel(suffix)));
    }
    if let Some(suffix) = name.strip_prefix("if_") {
        return ui_property(sanitize_camel(suffix));
    }
    // The TS surface renders an opcode as `sanitize_ts_ident(strip_underscores(opcode))`; the
    // `sanitize_ts_ident` step adds the reserved-word guard that turns the `enum` opcode into the
    // identifier `enum_` (which `ts_lower::resolve_command` matches). Skipping it left `enum(...)` as
    // `Identifier("enum")`, unresolvable → the `unsupported call expression` wall (1,699 scripts).
    let ts_name = if ctx.is_canonical_command(name) {
        super::sanitize_ts_ident(&strip_underscores(name))
    } else {
        name.to_string()
    };
    Expression::Identifier(Identifier { name: ts_name })
}

fn ui_property(method: String) -> Expression {
    Expression::PropertyAccess(PropertyAccess {
        object: Box::new(Expression::Identifier(Identifier {
            name: "UI".to_string(),
        })),
        property: method,
    })
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn expr_to_target(expr: Expression) -> Result<AssignmentTarget> {
    match expr {
        Expression::Identifier(id) => Ok(AssignmentTarget::Identifier(id.name)),
        Expression::ArrayAccess(access) => {
            let Expression::Identifier(arr) = *access.array else {
                bail!("rs_parse: array assignment target not a plain array");
            };
            Ok(AssignmentTarget::ArrayAccess {
                array: arr.name,
                index: *access.index,
            })
        }
        _ => bail!("rs_parse: invalid assignment target"),
    }
}

fn op_prec(op: BinaryOp) -> i32 {
    match op {
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            40
        }
        BinaryOp::Or => 50,
        BinaryOp::And => 60,
        BinaryOp::Add | BinaryOp::Sub => 70,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 80,
    }
}

/// `cc_setparam_string` suffix `setparam_string` → `SetparamString` (the emitter's `sanitize_camel`,
/// matching `expr_recovery::sanitize_camel`; ts_lower re-derives the opcode from arg-count).
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

fn strip_underscores(name: &str) -> String {
    name.chars().filter(|&c| c != '_').collect()
}

#[cfg(test)]
mod tests {
    use super::parse_runescript;
    use crate::transpile::ast::{
        ArgumentVariable, BinaryOp, BinaryOperation, CallExpr, Expression, Identifier,
        LocalVariable, NumberLiteral, PropertyAccess, ScriptId, StringLiteral, TypeAnnotation,
    };
    use crate::transpile::runescript::{RuneScriptContext, render_runescript};
    use crate::transpile::structured::{AssignmentTarget, StructuredScript, StructuredStmt};
    use std::collections::HashSet;

    fn ctx() -> RuneScriptContext {
        let mut scripts = HashSet::new();
        scripts.insert("my_helper".to_string());
        RuneScriptContext::new(scripts)
    }

    fn ident(name: &str) -> Expression {
        Expression::Identifier(Identifier {
            name: name.to_string(),
        })
    }

    fn num(value: i32) -> Expression {
        Expression::NumberLiteral(NumberLiteral { value })
    }

    fn script(
        arguments: Vec<ArgumentVariable>,
        locals: Vec<LocalVariable>,
        return_type: &str,
        body: Vec<StructuredStmt>,
    ) -> StructuredScript {
        StructuredScript {
            script_id: ScriptId(0),
            raw_name: None,
            header_comments: Vec::new(),
            imports: Vec::new(),
            function_name: "demo".to_string(),
            arguments,
            locals,
            arrays: Vec::new(),
            return_type: return_type.to_string(),
            body,
        }
    }

    fn arg(name: &str) -> ArgumentVariable {
        ArgumentVariable {
            index: 0,
            name: name.to_string(),
            type_annotation: TypeAnnotation::Number,
        }
    }

    fn local(name: &str) -> LocalVariable {
        LocalVariable {
            index: 0,
            name: name.to_string(),
            type_annotation: TypeAnnotation::Number,
        }
    }

    /// The core G1.3 property: emit → parse → emit is idempotent (the parser faithfully inverts
    /// the emitter's grammar).
    fn assert_roundtrip(s: &StructuredScript) {
        let c = ctx();
        let once = render_runescript(s, &c);
        let parsed = parse_runescript(&once, &c).expect("parse");
        let twice = render_runescript(&parsed, &c);
        assert_eq!(once, twice, "round-trip not idempotent\n---\n{once}");
    }

    #[test]
    fn roundtrip_header_locals_arithmetic() {
        let s = script(
            vec![arg("arg_int_0"), arg("arg_int_1")],
            vec![local("local_int_0")],
            "number",
            vec![
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_int_0".to_string()),
                    value: Expression::BinaryOperation(BinaryOperation {
                        op: BinaryOp::Add,
                        left: Box::new(ident("arg_int_0")),
                        right: Box::new(num(1)),
                    }),
                },
                StructuredStmt::Return {
                    value: Some(ident("local_int_0")),
                },
            ],
        );
        assert_roundtrip(&s);
    }

    #[test]
    fn roundtrip_control_flow_and_commands() {
        let s = script(
            Vec::new(),
            vec![local("local_int_0")],
            "void",
            vec![
                StructuredStmt::If {
                    condition: Expression::BinaryOperation(BinaryOperation {
                        op: BinaryOp::Lt,
                        left: Box::new(ident("local_int_0")),
                        right: Box::new(num(5)),
                    }),
                    then_body: vec![StructuredStmt::Expr {
                        expr: Expression::Call(CallExpr {
                            callee: Box::new(ident("enumgetoutputcount")),
                            arguments: vec![num(14058)],
                        }),
                    }],
                    else_body: Some(vec![StructuredStmt::Expr {
                        expr: Expression::Call(CallExpr {
                            callee: Box::new(ident("my_helper")),
                            arguments: vec![ident("local_int_0")],
                        }),
                    }]),
                },
                StructuredStmt::Return { value: None },
            ],
        );
        assert_roundtrip(&s);
    }

    #[test]
    fn roundtrip_ui_call_and_string_and_var() {
        let s = script(
            Vec::new(),
            vec![local("local_obj_0")],
            "void",
            vec![
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_obj_0".to_string()),
                    value: Expression::StringLiteral(StringLiteral {
                        value: "hi there".to_string(),
                    }),
                },
                StructuredStmt::Expr {
                    expr: Expression::Call(CallExpr {
                        callee: Box::new(Expression::PropertyAccess(PropertyAccess {
                            object: Box::new(ident("UI")),
                            property: "deleteAll".to_string(),
                        })),
                        arguments: vec![ident("varplayer_1186")],
                    }),
                },
            ],
        );
        assert_roundtrip(&s);
    }

    #[test]
    fn parses_header_argument_indices() {
        let rs = "demo(int $int0, int $int1)\nreturn;\n";
        let parsed = parse_runescript(rs, &ctx()).expect("parse");
        assert_eq!(parsed.arguments.len(), 2);
        assert_eq!(parsed.arguments[0].name, "arg_int_0");
        assert_eq!(parsed.arguments[1].name, "arg_int_1");
    }

    #[test]
    fn local_index_follows_arguments() {
        // one int arg → `$int1` is local int slot 0 (`local_int_0`).
        let rs = "demo(int $int0)\ndef_int $int1 = 7;\nreturn;\n";
        let parsed = parse_runescript(rs, &ctx()).expect("parse");
        assert_eq!(parsed.locals.len(), 1);
        assert_eq!(parsed.locals[0].name, "local_int_0");
        let StructuredStmt::Assignment { target, .. } = &parsed.body[0] else {
            panic!("expected assignment, got {:?}", parsed.body[0]);
        };
        assert!(matches!(target, AssignmentTarget::Identifier(n) if n == "local_int_0"));
    }
}
