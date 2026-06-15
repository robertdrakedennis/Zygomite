#!/usr/bin/env python3
"""Split ScriptRunner.java dispatch handlers into category classes.

One-shot, stdlib-only. Moves every method named in ``categories-910.json`` out of
``ScriptRunner.java`` into ``<ClassName>.java`` siblings in the same package, then
replaces the ``executeCommand`` body with a delegation to ``Cs2Dispatch.execute``.

Inputs (paths are resolved relative to this script unless overridden):
  - ScriptRunner.java
  - categories-910.json (the contract produced by `generate-cs2-java`)

This is an audit artifact; idempotence is not required (it rewrites the tree once).
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
CRATE_DIR = os.path.dirname(SCRIPT_DIR)
REPO_ROOT = os.path.dirname(os.path.dirname(CRATE_DIR))
CLIENT_PKG = os.path.join(
    REPO_ROOT, "client", "client", "src", "main", "java", "rs2", "client", "clientscript"
)
SCRIPT_RUNNER = os.path.join(CLIENT_PKG, "ScriptRunner.java")
CATEGORIES = os.path.join(CRATE_DIR, "data", "cs2", "categories-910.json")

IMPORT_FIRST_LINE = 3  # 1-based: first import line
STATIC_IMPORT = "import static rs2.client.clientscript.ScriptRunner.*;"


# ---------------------------------------------------------------------------
# Method span detection
# ---------------------------------------------------------------------------


@dataclass
class Span:
    """A top-level method span: [annotation?] signature .. closing brace + blank."""

    name: str
    start: int  # 0-based index of the first line of the span (annotation or signature)
    sig: int  # 0-based index of the signature line
    end: int  # 0-based index, exclusive (past the trailing blank line)
    is_public: bool


SIG_PREFIXES = ("\tpublic ", "\tprivate ", "\tprotected ")


def method_name_from_signature(line: str) -> str | None:
    """Extract the method name from a top-level method signature line.

    Returns ``None`` when the line is a field/other declaration, not a method.
    """
    # A method signature contains `name(` before the parameter list. Field
    # declarations have `=` or end with `;` and never an unescaped `(` in the
    # declarator. Find the first `(` and take the identifier immediately before it.
    paren = line.find("(")
    if paren == -1:
        return None
    head = line[:paren]
    # The identifier is the last token before `(`; reject if it looks like a
    # control statement or contains `=`.
    if "=" in head:
        return None
    token = ""
    for ch in reversed(head):
        if ch.isalnum() or ch == "_":
            token = ch + token
        else:
            break
    if not token:
        return None
    # Reject Java keywords that can precede `(` (none expected at top level, but
    # guard the generics/`new` edge anyway).
    if token in {"if", "for", "while", "switch", "catch", "new", "return"}:
        return None
    return token


def find_spans(lines: list[str]) -> list[Span]:
    """Locate every top-level method span in ``lines``."""
    spans: list[Span] = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        if line.startswith(SIG_PREFIXES):
            name = method_name_from_signature(line)
            if name is not None:
                # Annotation directly above?
                start = i
                if i > 0 and lines[i - 1].lstrip().startswith("@ObfuscatedName("):
                    start = i - 1
                # Find the matching closing brace by brace counting from the
                # signature line. Methods always open a brace on the sig line.
                depth = 0
                j = i
                opened = False
                while j < n:
                    depth += lines[j].count("{")
                    depth -= lines[j].count("}")
                    if "{" in lines[j]:
                        opened = True
                    if opened and depth == 0:
                        break
                    j += 1
                # j is the closing-brace line. Consume one trailing blank line.
                end = j + 1
                if end < n and lines[end].strip() == "":
                    end += 1
                spans.append(
                    Span(
                        name=name,
                        start=start,
                        sig=i,
                        end=end,
                        is_public=line.startswith("\tpublic "),
                    )
                )
                i = end
                continue
        i += 1
    return spans


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    dry_run = "--dry-run" in sys.argv[1:]
    with open(SCRIPT_RUNNER, "r", encoding="utf-8") as fh:
        text = fh.read()
    # Preserve exact bytes: split keeping the structure, operate on a line list
    # with explicit newline handling at write time.
    lines = text.split("\n")
    # `split("\n")` yields a trailing "" for a file ending in "\n"; track it.
    trailing_newline = text.endswith("\n")
    if trailing_newline:
        lines = lines[:-1]

    with open(CATEGORIES, "r", encoding="utf-8") as fh:
        categories = json.load(fh)
    method_to_class: dict[str, str] = {}
    for class_name, methods in categories["classes"].items():
        for method in methods:
            method_to_class[method] = class_name
    move_names = set(method_to_class)

    spans = find_spans(lines)

    # Import block (verbatim) — content of lines IMPORT_FIRST_LINE..last import.
    import_start = IMPORT_FIRST_LINE - 1  # 0-based
    import_end = import_start
    while import_end < len(lines) and lines[import_end].startswith("import "):
        import_end += 1
    # Trailing blank lines among imports are tolerated; stop at the class line.
    import_block = lines[import_start:import_end]

    # Partition spans into moved vs kept.
    moved_spans = [s for s in spans if s.name in move_names]
    moved_by_name: dict[str, list[Span]] = {}
    for s in moved_spans:
        moved_by_name.setdefault(s.name, []).append(s)

    # Assert every moved span is public.
    non_public = [s for s in moved_spans if not s.is_public]
    if non_public:
        for s in non_public:
            sys.stderr.write(
                f"ABORT: moved method `{s.name}` (sig line {s.sig + 1}) is not public\n"
            )
        return 2

    moved_count = len(moved_spans)
    print(f"move set: {len(move_names)} names, {moved_count} method spans")

    if dry_run:
        found = {s.name for s in spans}
        missing = sorted(n for n in move_names if n not in found)
        print(f"total spans: {len(spans)}")
        print(f"move names with no span: {len(missing)} {missing[:10]}")
        overloads = {}
        for s in moved_spans:
            overloads[s.name] = overloads.get(s.name, 0) + 1
        print("overloaded moved:", {k: v for k, v in overloads.items() if v > 1})
        return 0

    # Group moved spans by class, preserving original file order.
    class_to_spans: dict[str, list[Span]] = {}
    for s in moved_spans:
        cls = method_to_class[s.name]
        class_to_spans.setdefault(cls, []).append(s)
    for cls in class_to_spans:
        class_to_spans[cls].sort(key=lambda sp: sp.start)

    # ---- Emit category files ----
    # First materialize each class body so cross-category references can be
    # detected; a moved body may call a moved method that now lives in a
    # different category class (e.g. CcOps -> IfOps.if_setretex). Those resolve
    # by adding `import static rs2.client.clientscript.<OtherClass>.*;` rather
    # than rewriting bodies (mirrors the kept-source rule in spec §4.4).
    summary_rows: list[tuple[str, int, int]] = []
    cross_imports: list[tuple[str, str]] = []
    for cls in sorted(class_to_spans):
        spans_for_cls = class_to_spans[cls]
        body_lines: list[str] = []
        method_count = 0
        for s in spans_for_cls:
            body_lines.extend(lines[s.start : s.end])
            method_count += 1
        body_text = "\n".join(body_lines)

        # Detect unqualified references to moved methods owned by other classes.
        needed: set[str] = set()
        for name in move_names:
            target = method_to_class[name]
            if target == cls:
                continue
            token = name + "("
            idx = 0
            while True:
                pos = body_text.find(token, idx)
                if pos == -1:
                    break
                idx = pos + 1
                before = body_text[pos - 1] if pos > 0 else "\n"
                if before == "." or before.isalnum() or before == "_":
                    continue
                needed.add(target)
                cross_imports.append((cls, target))

        out_lines: list[str] = []
        out_lines.append("package rs2.client.clientscript;")
        out_lines.append("")
        out_lines.extend(import_block)
        out_lines.append(STATIC_IMPORT)
        for other in sorted(needed):
            out_lines.append(f"import static rs2.client.clientscript.{other}.*;")
        out_lines.append("")
        out_lines.append(f"public final class {cls} {{")
        out_lines.append("")
        out_lines.extend(body_lines)
        # Each span ends with its trailing blank line, so the body already has a
        # blank before the class close; add the closing brace.
        out_lines.append("}")
        out_path = os.path.join(CLIENT_PKG, f"{cls}.java")
        with open(out_path, "w", encoding="utf-8") as fh:
            fh.write("\n".join(out_lines) + "\n")
        summary_rows.append((cls, method_count, len(out_lines) + 1))

    # ---- Rewrite ScriptRunner.java ----
    # Locate executeCommand span (kept) to replace its body.
    exec_span = None
    for s in spans:
        if s.name == "executeCommand":
            exec_span = s
            break
    if exec_span is None:
        sys.stderr.write("ABORT: could not locate executeCommand span\n")
        return 2
    # The annotation line above executeCommand must be preserved unchanged.
    anno_line = (
        lines[exec_span.start] if exec_span.start < exec_span.sig else None
    )
    if anno_line is None or not anno_line.lstrip().startswith("@ObfuscatedName("):
        sys.stderr.write("ABORT: executeCommand annotation not found\n")
        return 2
    sig_line = lines[exec_span.sig]
    delegation = [
        anno_line,
        sig_line,
        "\t\tCs2Dispatch.execute(arg0, arg1);",
        "\t}",
        "",
    ]

    # Build the set of line ranges to drop (moved spans) and the replacement for
    # executeCommand. Process by walking lines and skipping/replacing.
    moved_ranges = sorted((s.start, s.end) for s in moved_spans)
    exec_range = (exec_span.start, exec_span.end)

    result: list[str] = []
    i = 0
    n = len(lines)
    drop_iter = iter(moved_ranges)
    next_drop = next(drop_iter, None)
    while i < n:
        if i == exec_range[0]:
            result.extend(delegation)
            i = exec_range[1]
            continue
        if next_drop is not None and i == next_drop[0]:
            i = next_drop[1]
            next_drop = next(drop_iter, None)
            continue
        result.append(lines[i])
        i += 1

    # ---- Safety-net scan: remaining bodies referencing moved methods unqualified ----
    remaining_text = "\n".join(result)
    needed_classes: set[str] = set()
    scan_hits: list[tuple[str, str]] = []
    for name in sorted(move_names):
        # Heuristic unqualified-call detection: `<name>(` not preceded by `.`
        # or an identifier char. Scan token-wise.
        idx = 0
        token = name + "("
        while True:
            pos = remaining_text.find(token, idx)
            if pos == -1:
                break
            idx = pos + 1
            before = remaining_text[pos - 1] if pos > 0 else "\n"
            if before == "." or before.isalnum() or before == "_":
                continue
            # An unqualified reference to a moved method in the kept source.
            needed_classes.add(method_to_class[name])
            scan_hits.append((name, method_to_class[name]))

    # Add static imports for any class whose moved methods are still referenced
    # unqualified in the kept ScriptRunner bodies (expected: none).
    if needed_classes:
        # Insert after the existing import block (which ends at import_end).
        # Recompute import_end in the rewritten list (unchanged region).
        ins = import_end
        added = [
            f"import static rs2.client.clientscript.{cls}.*;"
            for cls in sorted(needed_classes)
        ]
        result = result[:ins] + added + result[ins:]

    with open(SCRIPT_RUNNER, "w", encoding="utf-8") as fh:
        fh.write("\n".join(result) + "\n")

    # ---- Summary ----
    before_lines = n
    after_lines = len(result)
    print("category files:")
    for cls, mcount, lcount in summary_rows:
        print(f"  {cls}: {mcount} methods, {lcount} lines")
    print(f"ScriptRunner.java: {before_lines} -> {after_lines} lines")
    if scan_hits:
        print(f"safety-net scan: {len(scan_hits)} unqualified moved-method reference(s):")
        for name, cls in sorted(set(scan_hits)):
            print(f"  {name} -> added `import static ...{cls}.*;`")
    else:
        print("safety-net scan: no unqualified moved-method references in kept bodies")
    if cross_imports:
        print(f"cross-category static imports: {len(set(cross_imports))} added")
        for src, tgt in sorted(set(cross_imports)):
            print(f"  {src} -> import static {tgt}.*")
    else:
        print("cross-category static imports: none needed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
