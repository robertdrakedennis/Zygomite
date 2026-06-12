#!/usr/bin/env python3
"""Derive a new build's CS2 opcode book from a previous build's book by
cross-cache script alignment.

CS2 opcodes are fully rescrambled every RS3 build, but unchanged scripts keep
identical instruction structure. For every clientscripts group (archive 12)
present in both caches with the same instruction count, we decode the OLD
script with the OLD book, then walk the NEW script's bytecode in lockstep
(operand widths are determined by command semantics, which are identical for
the same instruction), reading the NEW opcode at each position. Votes per
(old command -> new opcode) across ~30k scripts give the new book; a script
is discarded unless the lockstep walk lands exactly on the header boundary
on both sides.

Usage:
  python3 scripts/derive-opcode-book.py \
      --old-cache ../../cache/unpacked/947 --old-book data/opcodes-947.txt \
      --new-cache ../../cache/unpacked/948 --out data/opcodes-948.txt

Mirrors src/script.rs decode_script/decode_operand and src/js5.rs decompress.
"""
from __future__ import annotations

import argparse
import bz2
import gzip
import io
import lzma
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

VERSION = 948  # both eras are >= 800; only era thresholds matter

FOUR_BYTE = {
    "push_constant_int", "join_string", "gosub_with_params", "switch",
    "push_int_local", "pop_int_local", "push_string_local",
    "pop_string_local", "push_long_local", "pop_long_local",
    "branch", "branch_not", "branch_equals", "branch_less_than",
    "branch_greater_than", "branch_less_than_or_equals",
    "branch_greater_than_or_equals", "long_branch_not",
    "long_branch_equals", "long_branch_less_than",
    "long_branch_greater_than", "long_branch_less_than_or_equals",
    "long_branch_greater_than_or_equals", "branch_if_true",
    "branch_if_false",
    "define_array", "push_array_int", "pop_array_int",
    "push_array_int_leave_index_on_stack", "push_array_int_and_index",
    "pop_array_int_leave_value_on_stack",
    # fixed var/varbit commands (g4 id)
    "push_varc_int", "pop_varc_int", "push_varc_string", "pop_varc_string",
    "push_varclan", "push_varclan_long", "push_varclan_string",
    "push_varclansetting", "push_varclansetting_long",
    "push_varclansetting_string", "push_varclanbit",
    "push_varclansettingbit",
}
EIGHT_BYTE = {"push_long_constant"}
VAR_CMDS = {"push_var", "pop_var"}          # 1 domain + 2 id + 1 transmog
VARBIT_CMDS = {"push_varbit", "pop_varbit"}  # 2 id + 1 transmog
STRINGY = {"push_constant_string"}           # 1 tag + (4 | 8 | nul-str)


def decompress(blob: bytes) -> bytes:
    ctype = blob[0]
    csize = struct.unpack(">i", blob[1:5])[0]
    if ctype == 0:
        return blob[5:5 + csize]
    usize = struct.unpack(">i", blob[5:9])[0]
    payload = blob[9:9 + csize]
    if ctype == 1:
        out = bz2.decompress(b"BZh1" + payload)
    elif ctype == 2:
        out = gzip.GzipFile(fileobj=io.BytesIO(payload)).read()
    elif ctype == 3:
        # JS5 lzma: 5-byte props header then raw stream with known size
        props = payload[:5]
        filt = [{"id": lzma.FILTER_LZMA1,
                 "lc": props[0] % 9, "lp": (props[0] // 9) % 5,
                 "pb": props[0] // 45,
                 "dict_size": struct.unpack("<I", props[1:5])[0]}]
        out = lzma.LZMADecompressor(lzma.FORMAT_RAW, filters=filt) \
            .decompress(payload[5:], usize)
    else:
        raise ValueError(f"compression type {ctype}")
    if len(out) != usize:
        raise ValueError("size mismatch")
    return out


class Reader:
    __slots__ = ("d", "p")

    def __init__(self, d: bytes, p: int = 0):
        self.d, self.p = d, p

    def g1(self):
        v = self.d[self.p]
        self.p += 1
        return v

    def g2(self):
        v = struct.unpack_from(">H", self.d, self.p)[0]
        self.p += 2
        return v

    def g4s(self):
        v = struct.unpack_from(">i", self.d, self.p)[0]
        self.p += 4
        return v

    def skip_str(self):
        i = self.d.index(0, self.p)  # gjstr also stops at \n; \0 dominant
        nl = self.d.find(10, self.p, i)
        self.p = (nl if nl != -1 else i) + 1


def parse_header(d: bytes):
    """Returns (name_end_pos, code_end_pos=header_pos, code_len)."""
    trailer = struct.unpack(">H", d[-2:])[0]
    header_size = 12 + 4 + 2 + trailer
    header_pos = len(d) - header_size
    r = Reader(d, header_pos)
    code_len = r.g4s()
    # skip name (gjstrnull) from pos 0
    r2 = Reader(d, 0)
    if d[0] == 0:
        r2.p = 1
    else:
        r2.skip_str()
    return r2.p, header_pos, code_len


def skip_operand(cmd: str, r: Reader):
    if cmd in FOUR_BYTE:
        r.p += 4
    elif cmd in EIGHT_BYTE:
        r.p += 8
    elif cmd in VAR_CMDS:
        r.p += 4  # 1 domain + 2 id + 1 transmog
    elif cmd in VARBIT_CMDS:
        r.p += 3  # 2 id + 1 transmog
    elif cmd in STRINGY:
        tag = r.g1()
        if tag == 0:
            r.p += 4
        elif tag == 1:
            r.p += 8
        elif tag == 2:
            r.skip_str()
        else:
            raise ValueError(f"string tag {tag}")
    else:
        r.p += 1  # default byte operand (no large-operand list for 947/948)


def walk(d: bytes, names_by_op: dict[int, str] | None,
         cmds: list[str] | None):
    """Walk a script's code region. If names_by_op given, commands come from
    the book (old build). If cmds given, commands are imposed per index (new
    build). Returns list of (command_or_None, opcode)."""
    start, header_pos, code_len = parse_header(d)
    r = Reader(d, start)
    out = []
    i = 0
    while r.p < header_pos:
        if cmds is not None and i >= len(cmds):
            raise ValueError("more instructions than counterpart")
        op = r.g2()
        cmd = (names_by_op.get(op) if names_by_op is not None else cmds[i])
        if cmd is None:
            raise ValueError(f"unknown opcode {op}")
        skip_operand(cmd, r)
        out.append((cmd, op))
        i += 1
    if r.p != header_pos or len(out) != code_len:
        raise ValueError("walk did not land on header boundary")
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--old-cache", required=True, type=Path)
    ap.add_argument("--new-cache", required=True, type=Path)
    ap.add_argument("--old-book", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--archive", default="12")
    a = ap.parse_args()

    name_to_old: dict[str, int] = {}
    order: list[str] = []
    for line in a.old_book.read_text().splitlines():
        line = line.strip()
        if not line or "," not in line:
            continue
        name, op = line.rsplit(",", 1)
        name_to_old[name] = int(op)
        order.append(name)
    old_by_op = {v: k for k, v in name_to_old.items()}

    old_dir = a.old_cache / a.archive
    new_dir = a.new_cache / a.archive
    groups = sorted(
        {p.stem for p in old_dir.glob("*.dat")} &
        {p.stem for p in new_dir.glob("*.dat")},
        key=lambda s: int(s) if s.isdigit() else 1 << 30)

    votes: dict[str, Counter] = defaultdict(Counter)
    used = skipped = failed = 0
    for g in groups:
        try:
            od = decompress((old_dir / f"{g}.dat").read_bytes())
            nd = decompress((new_dir / f"{g}.dat").read_bytes())
            _, _, ocl = parse_header(od)
            _, _, ncl = parse_header(nd)
            if ocl != ncl:
                skipped += 1
                continue
            old_walk = walk(od, old_by_op, None)
            new_walk = walk(nd, None, [c for c, _ in old_walk])
            for (cmd, _), (_, nop) in zip(old_walk, new_walk):
                votes[cmd][nop] += 1
            used += 1
        except Exception:
            failed += 1
    print(f"groups: common={len(groups)} used={used} "
          f"skipped(len-change)={skipped} failed={failed}")

    # Votes are individually structure-verified (exact header-boundary walk),
    # so a single uncontradicted vote is acceptable. Resolve in confidence
    # order (highest top-vote first) and enforce bijectivity.
    mapping: dict[str, int] = {}
    conflicts = []
    claimed: dict[int, str] = {}
    for cmd, ctr in sorted(votes.items(),
                           key=lambda kv: -kv[1].most_common(1)[0][1]):
        top = ctr.most_common(2)
        op1, n1 = top[0]
        n2 = top[1][1] if len(top) > 1 else 0
        if n2 and n1 < max(4 * n2, n2 + 2):
            conflicts.append((cmd, ctr.most_common(3)))
            continue
        if op1 in claimed:
            conflicts.append((cmd, ctr.most_common(3)))
            continue
        claimed[op1] = cmd
        mapping[cmd] = op1

    covered = [n for n in order if n in mapping]
    missing = [n for n in order if n not in mapping]
    print(f"commands: book={len(order)} derived={len(covered)} "
          f"missing/unvoted={len(missing)} conflicts={len(conflicts)}")
    for c in conflicts[:10]:
        print("  conflict:", c)
    if missing[:15]:
        print("  missing sample:", missing[:15])

    with a.out.open("w") as f:
        for n in order:
            if n in mapping:
                f.write(f"{n},{mapping[n]}\n")
        for n in sorted(set(mapping) - set(order)):
            f.write(f"{n},{mapping[n]}\n")
    print(f"wrote {a.out} ({len(mapping)} entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
