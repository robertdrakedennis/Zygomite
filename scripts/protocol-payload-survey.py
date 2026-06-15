#!/usr/bin/env python3
"""Stage 7/8 Part A — protocol payload classification + schema survey.

One-off audit script (kept for the record per Stage 7 §2 / Stage 8 §3). Parses
every `ServerProt.<NAME>.encode = function (...) {...}` override in the server's
`ServerProt.ts` and the matching `ServerProt.NAME == arg0.packetType` decode
branch in the client's `Client.java`, classifies each end as simple/complex per
the DSL rules, and emits:

  * data/protocol/910/payload-classification.json   (Part A)
  * data/protocol/910/payloads.json                 (Part B, tranche only)

DSL v1 (Stage 7) admits straight-line `buf.<codec>(<param|literal>)` bodies.
DSL v2 (Stage 8) additionally admits, classified as server `v2-simple`:

  * integer expression args over `& | << >> + -`, parens, int/hex literals
    (e.g. `(interfaceId << 16) | component`, `snapshotId & 0xff`);
  * `int[]` params carrying a verbatim TS `default` (e.g. `[0, 0, 0, 0]`), with
    literal-index access (`key[3]!`) bounded by the default's element count;
  * single-assignment `const x = <in-grammar expr>;` locals, inlined at use;
  * loop-free computed allocation sizes (`Packet.alloc(<expr>)` /
    `new Uint8Array(<expr>)`) whose terms are in-grammar or the Stage-7
    string/smart width terms.

Anything outside that grammar (ternaries, `===`, `??`, helper calls, loops,
non-v1 codecs, `pdata`, multi-statement locals) keeps the packet complex.

Deterministic, no timestamps, sorted by name. Read-only over the source trees.

Run from tools/rs3-cache-rs:

    python3 scripts/protocol-payload-survey.py

Optional flags:
    --client-root PATH (default ../../client)
    --server-root PATH (default ../../server)
    --out-dir PATH     (default data/protocol/910)
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------
# DSL v1 codec vocabulary (spec §1.2). Maps a write codec to its fixed byte
# width, or marks it variable. Anything not present here => complex codec.
# --------------------------------------------------------------------------

# Fixed-width write codecs: name -> byte width.
FIXED_CODEC_WIDTH = {
    "p1": 1,
    "p1_alt1": 1,
    "p1_alt2": 1,
    "p1_alt3": 1,
    "p2": 2,
    "p2_alt1": 2,
    "p2_alt2": 2,
    "p2_alt3": 2,
    "p3": 3,
    "p4": 4,
    "p4_alt1": 4,
    "p4_alt2": 4,
    "p4_alt3": 4,
    "p5": 5,
    "p6": 6,
    "p8": 8,
    "pbool": 1,
}

# Variable-width write codecs admitted by v1.
VAR_CODECS = {"pjstr", "pSmart1or2"}

DSL_V1_CODECS = set(FIXED_CODEC_WIDTH) | VAR_CODECS

# --------------------------------------------------------------------------
# Mirror table (spec §1.4): write codec -> set of acceptable client reads.
# Width + alt variant must match; signedness and string charset are read-side
# choices, so signed (`g..s` / `g..b`) variants are accepted. Only reads that
# actually exist in the client Packet.java are listed.
# --------------------------------------------------------------------------

MIRROR = {
    "p1": {"g1", "g1b"},
    "p1_alt1": {"g1_alt1", "g1b_alt1"},
    "p1_alt2": {"g1_alt2", "g1b_alt2"},
    "p1_alt3": {"g1_alt3", "g1b_alt3"},
    "p2": {"g2", "g2s"},
    "p2_alt1": {"g2_alt1", "g2s_alt1"},
    "p2_alt2": {"g2_alt2", "g2s_alt2"},
    "p2_alt3": {"g2_alt3"},
    "p3": {"g3", "g3s"},
    "p4": {"g4s", "g4"},
    "p4_alt1": {"g4_alt1"},
    "p4_alt2": {"g4_alt2"},
    "p4_alt3": {"g4_alt3", "g3_alt3"},
    "p5": {"g5"},
    "p6": {"g6"},
    "p8": {"g8"},
    "pbool": {"g1", "g1b"},
    "pjstr": {"gjstr", "gjstr2"},
    "pSmart1or2": {"gSmart1or2", "gSmart1or2s"},
}

# Param TS type per codec arg position (Part B `params` typing).
CODEC_PARAM_TYPE = {
    "p5": "bigint",
    "p6": "bigint",
    "p8": "bigint",
    "pbool": "boolean",
    "pjstr": "string",
}


# --------------------------------------------------------------------------
# DSL v2 expression grammar (Stage 8 §2.2):
#
#   expr := term (op term)*           op := '&' | '|' | '<<' | '>>' | '+' | '-'
#   term := ident | ident '[' INT ']' | INT | HEX | '(' expr ')'
#
# We tokenize with string ops only (no regex over the whole expr) and parse a
# flat operator/term sequence with explicit parens. Validation collects the
# identifiers and array accesses used; the caller checks them against declared
# params. Parsing produces a canonical re-emitted string (used as the schema
# `arg`) that round-trips through the Rust emitter unchanged.
# --------------------------------------------------------------------------

EXPR_OPS = {"&", "|", "<<", ">>", "+", "-"}


class ExprError(Exception):
    """Raised when an expression is outside the v2 grammar."""


@dataclass
class ExprInfo:
    """Result of parsing one in-grammar expression."""

    idents: set[str]  # bare identifiers referenced
    # (ident, index) array accesses referenced
    array_access: set[tuple[str, int]]


def _tokenize_expr(text: str) -> list[str]:
    """Tokenize an expression into idents, ints, hex, ops, and `( ) [ ]`.

    String ops only. Any character outside the grammar's alphabet raises
    ExprError, which is how ternaries (`?`/`:`), `===`, `??`, `*`, `/`, etc.
    keep a packet complex.
    """
    toks: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch.isspace():
            i += 1
            continue
        if ch in "()[]":
            toks.append(ch)
            i += 1
            # A TS non-null assertion `!` after `]` (e.g. `key[3]!`) is a
            # compile-time no-op; consume it so array accesses parse.
            if ch == "]" and i < n and text[i] == "!":
                i += 1
            continue
        if ch in "<>":
            # Only the doubled shift operators are admitted.
            if i + 1 < n and text[i + 1] == ch:
                toks.append(ch + ch)
                i += 2
                continue
            raise ExprError(f"stray `{ch}` (only `<<`/`>>` admitted)")
        if ch in "&|+-":
            toks.append(ch)
            i += 1
            continue
        if ch == "0" and i + 1 < n and text[i + 1] in "xX":
            j = i + 2
            while j < n and text[j] in "0123456789abcdefABCDEF":
                j += 1
            if j == i + 2:
                raise ExprError("malformed hex literal")
            toks.append(text[i:j])
            i = j
            continue
        if ch.isdigit():
            j = i
            while j < n and text[j].isdigit():
                j += 1
            toks.append(text[i:j])
            i = j
            continue
        if ch.isalpha() or ch == "_":
            j = i
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            toks.append(text[i:j])
            i = j
            continue
        raise ExprError(f"illegal character {ch!r}")
    return toks


def _is_int_token(tok: str) -> bool:
    if tok.lower().startswith("0x"):
        return len(tok) > 2
    return tok.isdigit()


def _is_ident_token(tok: str) -> bool:
    return bool(tok) and (tok[0].isalpha() or tok[0] == "_") and tok.replace("_", "a").isalnum()


class _ExprParser:
    """Recursive-descent parser for the v2 grammar over a token list.

    Records identifiers and array accesses encountered; produces a canonical
    re-emitted string. Operators are left-associated as written (no precedence
    folding) and parenthesized exactly as the source parenthesized them — the
    Rust emitter mirrors this, so a parsed-then-emitted expression is stable.
    """

    def __init__(self, toks: list[str]) -> None:
        self.toks = toks
        self.pos = 0
        self.idents: set[str] = set()
        self.array_access: set[tuple[str, int]] = set()

    def _peek(self) -> str | None:
        return self.toks[self.pos] if self.pos < len(self.toks) else None

    def _next(self) -> str:
        tok = self.toks[self.pos]
        self.pos += 1
        return tok

    def parse(self) -> str:
        out = self._expr()
        if self.pos != len(self.toks):
            raise ExprError(f"trailing tokens at {self.pos}: {self.toks[self.pos:]}")
        return out

    def _expr(self) -> str:
        parts = [self._term()]
        while True:
            tok = self._peek()
            if tok in EXPR_OPS:
                self._next()
                parts.append(tok)
                parts.append(self._term())
            else:
                break
        return " ".join(parts)

    def _term(self) -> str:
        tok = self._peek()
        if tok is None:
            raise ExprError("unexpected end of expression")
        if tok == "(":
            self._next()
            inner = self._expr()
            if self._peek() != ")":
                raise ExprError("missing `)`")
            self._next()
            return f"({inner})"
        if _is_int_token(tok):
            self._next()
            return tok
        if _is_ident_token(tok):
            self._next()
            # Optional array index.
            if self._peek() == "[":
                self._next()
                idx_tok = self._peek()
                if idx_tok is None or not idx_tok.isdigit():
                    raise ExprError("array index must be a decimal literal")
                self._next()
                if self._peek() != "]":
                    raise ExprError("missing `]`")
                self._next()
                idx = int(idx_tok)
                self.array_access.add((tok, idx))
                return f"{tok}[{idx}]"
            self.idents.add(tok)
            return tok
        raise ExprError(f"unexpected token {tok!r}")


def parse_expr(text: str) -> tuple[str, ExprInfo]:
    """Parse one in-grammar expression. Returns (canonical_text, info).

    Raises ExprError if `text` is outside the v2 grammar.
    """
    toks = _tokenize_expr(text)
    if not toks:
        raise ExprError("empty expression")
    parser = _ExprParser(toks)
    canon = parser.parse()
    return canon, ExprInfo(parser.idents, parser.array_access)


@dataclass
class Field:
    codec: str
    arg: str  # param name or integer literal text


@dataclass
class ServerEncoder:
    name: str
    # (name, ts_type, default_or_None) verbatim from the signature.
    params: list[tuple[str, str, str | None]]
    fields: list[Field]
    simple: bool
    reason: str = ""
    # DSL tier: "v1" (straight-line) or "v2" (expressions/arrays/computed alloc).
    tier: str = "v1"
    # Computed allocation expression for variable-size v2 packets (loop-free),
    # or None for fixed-size packets (size comes from the schema).
    alloc: str | None = None


@dataclass
class ClientBranch:
    name: str
    reads: list[str]
    simple: bool
    reason: str = ""


# --------------------------------------------------------------------------
# Server TS encode parsing
# --------------------------------------------------------------------------

ENCODE_RE = re.compile(
    r"ServerProt\.([A-Z0-9_]+)\.encode = function \((.*?)\): Packet \{",
    re.DOTALL,
)

# An already-migrated packet: `ServerProt.NAME.encode = encodeName;` (the
# generated-function assignment form). These have no hand-written body left to
# parse — their schema lives in the existing `payloads.json`, which we carry
# forward unchanged (Stage 7 entries are append-only / byte-identical).
ENCODE_ASSIGN_RE = re.compile(
    r"^ServerProt\.([A-Z0-9_]+)\.encode = (encode[A-Za-z0-9_]+);$",
    re.MULTILINE,
)

# A single `buf.<codec>(<arg>)` statement.
BUF_CALL_RE = re.compile(r"^buf\.([A-Za-z0-9_]+)\((.*)\);$")
ALLOC_FIXED_RE = re.compile(r"^const buf: Packet = new Packet\(new Uint8Array\((.+)\)\);$")
# Variable-size allocation via `Packet.alloc(<expr>)`.
ALLOC_DYNAMIC_RE = re.compile(r"^const buf: Packet = Packet\.alloc\((.+)\);$")
# A single-assignment `const x = <expr>;` local (no type annotation forms in
# these encoders other than this; multi-statement locals keep packets complex).
CONST_DECL_RE = re.compile(r"^const ([A-Za-z_][A-Za-z0-9_]*)(?:: [^=]+)? = (.+);$")
RETURN_RE = re.compile(r"^return buf;$")


def parse_params(sig: str) -> list[tuple[str, str, str | None]]:
    """Parse a TS parameter list into ordered (name, type, default) triples.

    Handles `name: type`, `name: type = default`, and bare `name` (untyped).
    `default` is the verbatim default expression text, or None when absent.
    Splits on top-level commas (no nested generics appear in these sigs).
    """
    sig = sig.strip()
    if not sig:
        return []
    out: list[tuple[str, str, str | None]] = []
    for raw in split_top_level(sig):
        part = raw.strip()
        default: str | None = None
        if "=" in part:
            decl, default = part.split("=", 1)
            part = decl.strip()
            default = default.strip()
        if ":" in part:
            name, ty = part.split(":", 1)
            out.append((name.strip(), ty.strip(), default))
        else:
            out.append((part, "", default))
    return out


def split_top_level(text: str) -> list[str]:
    """Split on commas not nested in (), <>, [], {}."""
    parts: list[str] = []
    depth = 0
    cur = []
    for ch in text:
        if ch in "(<[{":
            depth += 1
        elif ch in ")>]}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if cur:
        parts.append("".join(cur))
    return parts


def extract_encoders(ts_src: str) -> list[ServerEncoder]:
    """Find every `ServerProt.X.encode = function (...): Packet {` override
    (signatures may span multiple lines) and classify its body."""
    encoders: list[ServerEncoder] = []
    for m in ENCODE_RE.finditer(ts_src):
        name = m.group(1)
        params = parse_params(m.group(2))
        body = collect_body(ts_src, m.end())
        enc = classify_server(name, params, body)
        encoders.append(enc)
    return encoders


def collect_body(src: str, body_start: int) -> list[str]:
    """Return the encoder body as stripped statement lines.

    `body_start` is the offset just after the opening `{` of the function. We
    read until the matching closing brace, then split into trimmed lines.
    """
    depth = 1
    i = body_start
    n = len(src)
    while i < n and depth > 0:
        ch = src[i]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                break
        i += 1
    body_text = src[body_start:i]
    return [ln.strip() for ln in body_text.splitlines() if ln.strip()]


def dsl_param_type(ts_type: str) -> str | None:
    """Map a verbatim TS param type to its DSL kind, or None if unsupported.

    `int[]` (TS `number[]`) is the only array kind DSL v2 admits.
    """
    t = ts_type.strip()
    if t in ("number[]", "Array<number>"):
        return "int[]"
    if t == "number":
        return "int"
    if t == "string":
        return "string"
    if t == "bigint":
        return "bigint"
    if t == "boolean":
        return "boolean"
    return None


def entry_is_v2(entry: dict | None) -> bool:
    """Classify a payloads.json entry as v2 (vs straight-line v1).

    v2 markers: an `int[]` param, a computed `alloc`, or any field whose `arg`
    is not a bare param name / integer literal (i.e. an expression).
    """
    if not entry:
        return False
    if "alloc" in entry:
        return True
    param_names = {p["name"] for p in entry.get("params", [])}
    if any(p.get("type") == "int[]" for p in entry.get("params", [])):
        return True
    for fld in entry.get("fields", []):
        arg = fld.get("arg", "")
        if arg in param_names:
            continue
        if parse_int_literal(arg) is not None and re.fullmatch(r"-?(?:\d+|0[xX][0-9a-fA-F]+)", arg):
            continue
        return True
    return False


def parse_int_literal(text: str) -> int | None:
    """Parse a decimal/hex integer literal, or None when not a plain literal."""
    t = text.strip()
    try:
        if t.lower().startswith("0x") or t.lower().startswith("-0x"):
            return int(t, 16)
        return int(t, 10)
    except ValueError:
        return None


def classify_server(name: str, params: list[tuple[str, str, str | None]], body: list[str]) -> ServerEncoder:
    """Classify a server encoder body for DSL v1 / v2 (Stage 7 §1.1 / Stage 8 §3).

    A packet is `simple` (v1) when it is one fixed allocation, then a sequence of
    `buf.<codec>(<param|literal>)` calls using DSL-v1 codecs, then `return buf;`.

    It is `v2-simple` when, after admitting the v2 grammar, the only reason it
    failed v1 was an in-grammar expression arg, an `int[]` param with
    literal-index access, a single-assignment in-grammar local, or a loop-free
    computed `Packet.alloc(<expr>)` size — i.e. nothing left the grammar.

    Anything else stays complex with a one-word reason.
    """
    # Build the param model. Array params carry their default's element count.
    param_names = {p[0] for p in params}
    array_lengths: dict[str, int] = {}
    dsl_types: dict[str, str] = {}
    for pname, pty, pdefault in params:
        kind = dsl_param_type(pty)
        if kind == "int[]":
            if pdefault is None:
                return ServerEncoder(name, params, [], False, "array-param-no-default")
            elems = split_top_level(pdefault.strip().lstrip("[").rstrip("]"))
            array_lengths[pname] = len([e for e in elems if e.strip()])
        if kind is not None:
            dsl_types[pname] = kind

    fields: list[Field] = []
    saw_alloc = False
    saw_return = False
    alloc_expr: str | None = None
    tier = "v1"
    # Single-assignment in-grammar locals available for inlining.
    consts: dict[str, str] = {}

    def resolve_arg(arg: str) -> tuple[str | None, str]:
        """Validate an arg as v1-plain or a v2 expression after const-inlining.

        Returns (canonical_arg, tier) where tier is "v1" for a bare param/literal
        and "v2" for an in-grammar expression; canonical_arg is None on rejection.
        """
        arg = arg.strip()
        # v1: bare param name or integer literal.
        if arg in param_names and arg not in array_lengths:
            return arg, "v1"
        if parse_int_literal(arg) is not None and re.fullmatch(r"-?(?:\d+|0[xX][0-9a-fA-F]+)", arg):
            return arg, "v1"
        # A bare const reference inlines to its (already-canonical) expression.
        if arg in consts:
            return consts[arg], ("v1" if consts[arg] in param_names else "v2")
        # v2: parse the expression. Inline single-assignment consts first.
        expr_text = inline_consts(arg, consts)
        try:
            canon, info = parse_expr(expr_text)
        except ExprError:
            return None, "v2"
        if not validate_expr_idents(info):
            return None, "v2"
        return canon, "v2"

    def validate_expr_idents(info: ExprInfo) -> bool:
        # Every bare ident must be a declared scalar param (not an array).
        for ident in info.idents:
            if ident not in param_names or ident in array_lengths:
                return False
        # Array accesses only on int[] params, with in-bounds literal index.
        for ident, idx in info.array_access:
            length = array_lengths.get(ident)
            if length is None or idx < 0 or idx >= length:
                return False
        return True

    stmts = [s for s in body if s]
    for s in stmts:
        if saw_return:
            return ServerEncoder(name, params, [], False, "code-after-return")
        am = ALLOC_FIXED_RE.match(s)
        if am:
            if saw_alloc:
                return ServerEncoder(name, params, [], False, "multi-alloc")
            saw_alloc = True
            # Fixed alloc: must be a plain integer literal (size from schema).
            raw = am.group(1).strip()
            if parse_int_literal(raw) is None:
                # A computed `new Uint8Array(<expr>)` — admit as v2 alloc.
                expr_text = inline_consts(raw, consts)
                try:
                    canon, info = parse_expr(expr_text)
                except ExprError:
                    return ServerEncoder(name, params, [], False, "alloc-computed-nongrammar")
                if not validate_expr_idents(info):
                    return ServerEncoder(name, params, [], False, "alloc-computed-badident")
                alloc_expr = canon
                tier = "v2"
            continue
        adm = ALLOC_DYNAMIC_RE.match(s)
        if adm:
            if saw_alloc:
                return ServerEncoder(name, params, [], False, "multi-alloc")
            saw_alloc = True
            raw = adm.group(1).strip()
            canon = resolve_alloc_size(raw, consts, param_names, array_lengths)
            if canon is None:
                return ServerEncoder(name, params, [], False, "alloc-dynamic")
            alloc_expr = canon
            tier = "v2"
            continue
        if RETURN_RE.match(s):
            saw_return = True
            continue
        cm = BUF_CALL_RE.match(s)
        if cm:
            codec = cm.group(1)
            arg = cm.group(2).strip()
            if codec not in DSL_V1_CODECS:
                return ServerEncoder(name, params, [], False, f"non-v1-codec:{codec}")
            canon, arg_tier = resolve_arg(arg)
            if canon is None:
                return ServerEncoder(name, params, [], False, "computed-arg")
            if arg_tier == "v2":
                tier = "v2"
            fields.append(Field(codec, canon))
            continue
        # const local: admit only single-assignment in-grammar expressions.
        cd = CONST_DECL_RE.match(s)
        if cd and not s.startswith("const buf"):
            local_name = cd.group(1)
            rhs = cd.group(2).strip()
            expr_text = inline_consts(rhs, consts)
            try:
                canon, info = parse_expr(expr_text)
            except ExprError:
                return ServerEncoder(name, params, [], False, "local")
            if not validate_expr_idents(info):
                return ServerEncoder(name, params, [], False, "local")
            consts[local_name] = canon
            tier = "v2"
            continue
        # Anything else: conditionals, loops, helper calls, let-locals, etc.
        if s.startswith("if ") or s.startswith("if("):
            return ServerEncoder(name, params, [], False, "conditional")
        if s.startswith("for ") or s.startswith("for(") or s.startswith("while"):
            return ServerEncoder(name, params, [], False, "loop")
        if "Packet.alloc" in s:
            return ServerEncoder(name, params, [], False, "alloc-dynamic")
        if s.startswith("const ") or s.startswith("let "):
            return ServerEncoder(name, params, [], False, "local")
        if s.startswith("buf."):
            return ServerEncoder(name, params, [], False, "non-codec-call")
        return ServerEncoder(name, params, [], False, "other")

    if not saw_alloc:
        return ServerEncoder(name, params, [], False, "no-alloc")
    if not saw_return:
        return ServerEncoder(name, params, [], False, "no-return")
    if not fields:
        return ServerEncoder(name, params, [], False, "empty")
    return ServerEncoder(name, params, fields, True, "", tier=tier, alloc=alloc_expr)


def inline_consts(expr_text: str, consts: dict[str, str]) -> str:
    """Substitute single-assignment const names with their canonical exprs.

    Whole-word replacement using the tokenizer's ident boundaries (string ops
    only — no regex). A const value already canonical is wrapped in parens when
    it is a compound expression, so operator nesting stays correct.
    """
    if not consts:
        return expr_text
    try:
        toks = _tokenize_expr(expr_text)
    except ExprError:
        return expr_text
    out: list[str] = []
    for tok in toks:
        if _is_ident_token(tok) and tok in consts:
            val = consts[tok]
            # Parenthesize compound substitutions to preserve grouping.
            out.append(val if (" " not in val) else f"({val})")
        else:
            out.append(tok)
    # Re-join with spacing around operators and tight brackets; the result is
    # re-parsed by the caller, so exact spacing is irrelevant.
    return " ".join(out)


def resolve_alloc_size(
    raw: str,
    consts: dict[str, str],
    param_names: set[str],
    array_lengths: dict[str, int],
) -> str | None:
    """Validate a `Packet.alloc(<expr>)` size as a loop-free computed size.

    Admits the v2 integer grammar plus the Stage-7 string/smart width terms
    `<param>.length + 1` (pjstr) and `(<param> < 128 ? 1 : 2)` (pSmart1or2),
    which the generator reproduces. Returns the canonical size text, or None
    when the size needs a loop / helper / non-grammar term.
    """
    text = inline_consts(raw, consts)
    # Only the pure-integer grammar is admitted here; string/smart-width terms
    # carry `.length` / `?` which the expr tokenizer rejects, so any alloc that
    # needs them stays complex (those packets also use pjstr/pdata and fail the
    # codec check anyway). Keep this strictly in-grammar.
    try:
        canon, info = parse_expr(text)
    except ExprError:
        return None
    for ident in info.idents:
        if ident not in param_names or ident in array_lengths:
            return None
    for ident, idx in info.array_access:
        length = array_lengths.get(ident)
        if length is None or idx < 0 or idx >= length:
            return None
    return canon


# --------------------------------------------------------------------------
# Client Java decode parsing
# --------------------------------------------------------------------------

BRANCH_RE = re.compile(r"if \(ServerProt\.([A-Z0-9_]+) == arg0\.packetType\) \{")
# A read call on the packet buffer var2: var2.gXXX(...)
READ_RE = re.compile(r"var2\.(g[A-Za-z0-9_]+)\(")
CONTROL_RE = re.compile(r"\b(if|for|while|switch)\b\s*\(")


def extract_branches(java_src: str) -> dict[str, ClientBranch]:
    lines = java_src.splitlines()
    branches: dict[str, ClientBranch] = {}
    i = 0
    n = len(lines)
    while i < n:
        m = BRANCH_RE.search(lines[i])
        if not m:
            i += 1
            continue
        name = m.group(1)
        # Collect body lines until brace depth returns to opening level.
        body, end = collect_java_branch(lines, i)
        branches[name] = classify_client(name, body)
        i = end
    return branches


def collect_java_branch(lines: list[str], start: int) -> tuple[list[tuple[int, str]], int]:
    """Collect a decode-branch body with per-line nesting depth.

    `start` is the `[} else ]if (ServerProt.X == arg0.packetType) {` line. The
    branch body is every line after it until the matching `}` that closes the
    branch (which, in the `} else if` chain, is the leading `}` of the next
    `} else if (...) {` / `} else {` line, or the chain's final `}`).

    Returns `(rows, end_index)` where each row is `(nesting_depth, stripped)`:
    nesting_depth 0 = top level of the branch body, >0 = inside a nested
    `if`/`for`/`while`/`switch` block.
    """
    out: list[tuple[int, str]] = []
    # `nest` counts open braces *inside* the branch body (1 = branch top level).
    nest = 1  # the opening `{` of the branch itself
    idx = start + 1
    while idx < len(lines):
        ltxt = lines[idx]
        stripped = ltxt.strip()
        # At branch top level, a line that *starts* with `}` closes the branch
        # (e.g. `} else if (...) {`, `} else {`, or the chain's final `}`). The
        # trailing `{` of an `} else if` re-opens a sibling, not a child, so we
        # must stop before counting it.
        if nest == 1 and stripped.startswith("}"):
            break
        opens = ltxt.count("{")
        closes = ltxt.count("}")
        # Record reads at the *pre-line* nesting depth (branch top level == 0).
        out.append((nest - 1, stripped))
        nest += opens - closes
        idx += 1
    return out, idx


def classify_client(name: str, body: list[tuple[int, str]]) -> ClientBranch:
    """Classify a client decode branch per spec §1.3/§2.2.

    Simple iff every read sits at the branch's top level (no reads under
    if/for/while/switch) and there are no reads via helper methods. We record
    the top-level read sequence as evidence.
    """
    reads: list[str] = []
    has_control = False
    for depth, line in body:
        if line.endswith("else {") or line == "}":
            continue
        if CONTROL_RE.search(line):
            has_control = True
        line_reads = READ_RE.findall(line)
        if not line_reads:
            continue
        if depth > 0:
            return ClientBranch(name, [], False, "read-under-control")
        # Reads at top level: but if this same line also opens a control
        # structure, those reads are in the condition (acceptable position) —
        # however spec §1.3 marks any branch with reads under control complex.
        # A top-level `if (var2.g1() ...)` read is in a condition; treat as
        # complex to be safe (matches the conservative spec wording).
        if CONTROL_RE.search(line):
            return ClientBranch(name, [], False, "read-in-condition")
        reads.extend(line_reads)
    if has_control:
        # Control flow present but no reads under/in it: still classify by
        # whether reads were affected. If we got here, reads were all top-level
        # and none were in conditions — but presence of control with branching
        # writes (e.g. SET_MOVEACTION ternary reads) is risky. Conservatively
        # mark complex when control structures are present at all and the read
        # set is non-trivially gated. We only reach here if no read was nested;
        # but a top-level ternary read like `cond ? var2.gjstr() : ...` would
        # have matched READ_RE on a non-control line. Guard that next.
        pass
    # Detect ternary-gated reads on top-level lines (e.g. `cond ? var2.g2() : -1`).
    for depth, line in body:
        if depth == 0 and "?" in line and READ_RE.search(line):
            return ClientBranch(name, [], False, "ternary-read")
    if not reads:
        return ClientBranch(name, [], False, "no-reads")
    return ClientBranch(name, reads, True, "")


# --------------------------------------------------------------------------
# Mirror check (Part C uses this in Rust; we sanity-check it here too)
# --------------------------------------------------------------------------

def mirror_ok(fields: list[Field], reads: list[str]) -> tuple[bool, str]:
    if len(fields) != len(reads):
        return False, f"length {len(fields)} != {len(reads)}"
    for idx, (f, r) in enumerate(zip(fields, reads)):
        allowed = MIRROR.get(f.codec, set())
        if r not in allowed:
            return False, f"pos {idx}: {f.codec} !~ {r} (allowed {sorted(allowed)})"
    return True, ""


# --------------------------------------------------------------------------
# Param typing for Part B (refine `int` vs string/bigint/boolean by codec use)
# --------------------------------------------------------------------------

def refine_param_types(enc: ServerEncoder) -> dict[str, str]:
    """Map each param name to its DSL type using its TS type + codec usage.

    Array params (`number[]` → `int[]`) and other non-int TS types come from the
    signature directly; scalar `number` params default to `int` unless a codec
    that consumes a wider/string type names them as a bare arg (e.g. `pjstr` ⇒
    `string`, `p8` ⇒ `bigint`). v2 expression args never name a param as a bare
    field arg, so those params stay `int`, which is correct.
    """
    types: dict[str, str] = {}
    param_names = {p[0] for p in enc.params}
    # Signature-driven kinds first (arrays, strings, bigints, booleans).
    for pname, pty, _def in enc.params:
        kind = dsl_param_type(pty)
        types[pname] = kind if kind is not None else "int"
    # Codec usage refines bare-arg scalar params (matches Stage-7 behaviour).
    for fld in enc.fields:
        if fld.arg not in param_names:
            continue
        if types.get(fld.arg) == "int":
            types[fld.arg] = CODEC_PARAM_TYPE.get(fld.codec, "int")
    return types


# --------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--client-root", default="../../client")
    ap.add_argument("--server-root", default="../../server")
    ap.add_argument("--out-dir", default="data/protocol/910")
    args = ap.parse_args()

    client_root = Path(args.client_root)
    server_root = Path(args.server_root)
    out_dir = Path(args.out_dir)

    ts_path = server_root / "src/jagex/network/protocol/ServerProt.ts"
    java_path = client_root / "client/src/main/java/rs2/client/Client.java"

    ts_src = ts_path.read_text()
    java_src = java_path.read_text()

    encoders = extract_encoders(ts_src)
    branches = extract_branches(java_src)

    # Already-migrated packets (assigned a generated `encodeName`). Their schema
    # was authored in a prior stage and is carried forward verbatim from the
    # existing payloads.json (Stage 7 entries are byte-identical / append-only).
    migrated: dict[str, str] = {
        m.group(1): m.group(2) for m in ENCODE_ASSIGN_RE.finditer(ts_src)
    }
    existing_payloads: dict[str, dict] = {}
    existing_path = out_dir / "payloads.json"
    if existing_path.is_file():
        existing_payloads = json.loads(existing_path.read_text()).get("packets", {})

    # Full packet roster from the Stage-6 schema (all 195 ServerProt packets).
    schema = json.loads((out_dir / "server_prot.json").read_text())
    all_packets = [p["name"] for p in schema["packets"]]

    # ----- Part A: classification.json -----
    classification: dict[str, dict[str, str]] = {}
    enc_by_name = {e.name: e for e in encoders}

    # Packets that have no `encode` override inherit the empty-payload default;
    # they are not hand-written codec sequences, so they are not v1 candidates.
    # Already-migrated packets keep their prior (v1) classification.
    for name in all_packets:
        if name in enc_by_name:
            continue
        br = branches.get(name)
        client_class = "no_client_branch" if br is None else (
            "simple" if br.simple else f"complex:{br.reason}"
        )
        if name in migrated:
            # Carried-forward generated encoder; its DSL tier is read back from
            # the prior payloads.json entry so the classification stays stable
            # across the swap (an `int[]` param, an expression field arg, or a
            # computed `alloc` marks it `v2-simple`; otherwise `simple`).
            server_class = "v2-simple" if entry_is_v2(existing_payloads.get(name)) else "simple"
        else:
            server_class = "complex:no_encode_override"
        classification[name] = {
            "server": server_class,
            "client": client_class,
        }

    for enc in encoders:
        if enc.simple:
            server_class = "v2-simple" if enc.tier == "v2" else "simple"
        else:
            server_class = f"complex:{enc.reason}"
        br = branches.get(enc.name)
        if br is None:
            client_class = "no_client_branch"
        elif br.simple:
            client_class = "simple"
        else:
            client_class = f"complex:{br.reason}"
        classification[enc.name] = {"server": server_class, "client": client_class}

    classification = dict(sorted(classification.items()))

    # ----- tranche + mirror check -----
    # The tranche = (server `simple` OR `v2-simple`) AND client `simple`.
    tranche: list[str] = []
    mirror_mismatch: dict[str, str] = {}
    for name, cls in classification.items():
        if cls["server"] not in ("simple", "v2-simple") or cls["client"] != "simple":
            continue
        if name in enc_by_name:
            enc = enc_by_name[name]
            fields = enc.fields
        else:
            # Carried-forward (already-migrated) packet: mirror-check its prior
            # schema fields against the freshly-extracted client reads. The prior
            # entry MUST exist — a migrated packet with no schema means the file
            # was clobbered, which we surface loudly rather than silently drop.
            if name not in existing_payloads:
                print(
                    f"FATAL: migrated packet {name} has no entry in payloads.json "
                    "(file clobbered?) — restore it before re-running",
                    file=sys.stderr,
                )
                return 3
            prior = existing_payloads[name]
            fields = [Field(f["codec"], f["arg"]) for f in prior.get("fields", [])]
        br = branches[name]
        ok, why = mirror_ok(fields, br.reads)
        if ok:
            tranche.append(name)
        else:
            mirror_mismatch[name] = why
            # Reclassify per Stage 7 §8.3 / Stage 8 §4: leave it out of the
            # tranche and leave it hand-written.
            classification[name]["client"] = "complex:mirror_mismatch"

    # ----- Part B: payloads.json (tranche only) -----
    packets: dict[str, dict] = {}
    for name in tranche:
        if name not in enc_by_name:
            # Carry the prior v1 entry through byte-for-byte (Stage 7 invariant:
            # existing entries stay unchanged).
            packets[name] = existing_payloads[name]
            continue
        enc = enc_by_name[name]
        br = branches[name]
        ptypes = refine_param_types(enc)
        params_json = []
        for pname, _pty, pdefault in enc.params:
            entry = {"name": pname, "type": ptypes[pname]}
            if pdefault is not None:
                # Preserve the verbatim default so the generated encoder is a
                # drop-in for the hand-written one (callers omit these args).
                entry["default"] = pdefault
            params_json.append(entry)
        fields_json = [{"codec": f.codec, "arg": f.arg} for f in enc.fields]
        entry = {
            "params": params_json,
            "fields": fields_json,
            "client_reads": list(br.reads),
        }
        if enc.alloc is not None:
            # Computed (loop-free) allocation size for a variable-size packet.
            entry["alloc"] = enc.alloc
        packets[name] = entry
    packets = dict(sorted(packets.items()))

    # ----- required-member gate (BEFORE writing anything) -----
    # Required v1 tranche members (Stage 7).
    required_v1 = ["VARP_SMALL", "VARP_LARGE", "VARBIT_SMALL", "VARBIT_LARGE"]
    # Required v2 tranche members (Stage 8 §3).
    required_v2 = [
        "IF_OPENTOP", "IF_OPENSUB", "IF_SETEVENTS", "IF_SETANIM",
        "CLEAR_PLAYER_SNAPSHOT", "CHAT_FILTER_SETTINGS",
    ]
    missing = [r for r in (required_v1 + required_v2) if r not in tranche]
    if missing:
        print(f"FATAL: required tranche members missing: {missing}", file=sys.stderr)
        for r in missing:
            print(f"  {r}: {classification.get(r)}", file=sys.stderr)
        return 2

    # ----- write outputs (only after the gate passes) -----
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "payload-classification.json").write_text(
        json.dumps(classification, indent=2, sort_keys=True) + "\n"
    )
    payloads = {"schema": "protocol-payloads/v2", "packets": packets}
    (out_dir / "payloads.json").write_text(
        json.dumps(payloads, indent=2) + "\n"
    )

    # ----- stdout summary -----
    n_total = len(classification)
    server_simple = sum(1 for c in classification.values() if c["server"] == "simple")
    server_v2 = sum(1 for c in classification.values() if c["server"] == "v2-simple")
    no_branch = sum(1 for c in classification.values() if c["client"] == "no_client_branch")
    v2_tranche = sorted(n for n in tranche if classification[n]["server"] == "v2-simple")
    print(f"encoders parsed: {n_total}")
    print(f"server-simple (v1): {server_simple}")
    print(f"server-v2-simple: {server_v2}")
    print(f"no_client_branch: {no_branch}")
    print(f"tranche size: {len(tranche)} (v1 {len(tranche) - len(v2_tranche)} + v2 {len(v2_tranche)})")
    print(f"mirror_mismatch: {len(mirror_mismatch)}")
    for nm, why in sorted(mirror_mismatch.items()):
        print(f"  MIRROR_MISMATCH {nm}: {why}")
    print(f"v2 tranche: {v2_tranche}")
    print(f"required v1 present: {required_v1}")
    print(f"required v2 present: {required_v2}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
