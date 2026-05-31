#!/usr/bin/env python3
"""Phase-0 core-closure analysis for the 947→910 skill-guide migration.

Computes the CORE render closure from 947 deps, classifies each script against
the migrate-check status (SAFE / SCRIPT_CHANGED / MISSING), and for the
changed-shared scripts lists their NON-guide 910 callers (reverse edges) so we
can decide which can safely take 947's version.
"""
import json, sys
from collections import defaultdict, Counter

A = "/Users/robert/projects/alerion"
DEPS_947 = f"{A}/cache/rs3-cache/947-all/deps/scripts.jsonl"
DEPS_910 = f"{A}/cache/rs3-cache/910-all/deps/scripts.jsonl"
PLAN_1218 = "/tmp/mig-1218/plan.json"
PLAN_1217 = "/tmp/mig-1217/plan.json"

# Core render seed (from plan §2.5)
SEED = [5689,5690,5683,5691,12097,12098,2553,12099,
        13693,13694,13695,13697,13699,5697,5692,5693]

def load_forward(path):
    """script_id -> set(called script ids)"""
    g = defaultdict(set)
    ids = set()
    with open(path) as f:
        for line in f:
            o = json.loads(line)
            sid = o["script_id"]
            ids.add(sid)
            for s in o.get("dependency_sites", []):
                if s.get("entity_type") == "script":
                    g[sid].add(s["id"])
    return g, ids

def load_reverse(path):
    """called id -> set(caller script ids)"""
    r = defaultdict(set)
    ids = set()
    with open(path) as f:
        for line in f:
            o = json.loads(line)
            sid = o["script_id"]
            ids.add(sid)
            for s in o.get("dependency_sites", []):
                if s.get("entity_type") == "script":
                    r[s["id"]].add(sid)
    return r, ids

def closure(seed, fwd):
    seen = set()
    stack = list(seed)
    while stack:
        n = stack.pop()
        if n in seen:
            continue
        seen.add(n)
        for m in fwd.get(n, ()):
            if m not in seen:
                stack.append(m)
    return seen

def plan_status_map(path):
    d = json.load(open(path))
    return {e["id"]: e["status"] for e in d["entities"] if e["type"] == "script"}, d

fwd947, ids947 = load_forward(DEPS_947)
rev910, ids910 = load_reverse(DEPS_910)
fwd910, _ = load_forward(DEPS_910)

core = closure(SEED, fwd947)
print(f"CORE closure size (947): {len(core)}")

# status maps from both interface migrate reports (union)
st1218, d1218 = plan_status_map(PLAN_1218)
st1217, d1217 = plan_status_map(PLAN_1217)
status = {}
for m in (st1218, st1217):
    for k, v in m.items():
        # prefer a non-SAFE status if any report flags it
        if k not in status or (status[k] == "SAFE" and v != "SAFE"):
            status[k] = v

# full guide system = union of both closures' script ids
guide_system = set(st1218) | set(st1217) | core

# classify core
by_status = defaultdict(list)
for sid in sorted(core):
    s = status.get(sid)
    if s is None:
        # not in either migrate report; decide via presence in 910
        s = "SAFE" if sid in ids910 else "MISSING(notrace)"
    by_status[s].append(sid)

print("\n=== CORE classification ===")
for s in sorted(by_status):
    print(f"  {s}: {len(by_status[s])}")

new_ids = [s for s in core if status.get(s) == "MISSING" or (status.get(s) is None and s not in ids910)]
changed_shared = [s for s in core if status.get(s) == "SCRIPT_CHANGED"]
safe_shared = [s for s in core if (status.get(s) == "SAFE") or (status.get(s) is None and s in ids910)]

print(f"\nNEW (947-only) core: {len(new_ids)}")
print(sorted(new_ids))
print(f"\nCHANGED-SHARED core: {len(changed_shared)}")
print(sorted(changed_shared))
print(f"\nSAFE-SHARED core: {len(safe_shared)}")

# reverse-caller classification for changed-shared
print("\n=== CHANGED-SHARED: non-guide 910 callers ===")
guide_only = []
has_nonguide = []
report = {}
for sid in sorted(changed_shared):
    callers = rev910.get(sid, set())
    nonguide = sorted(c for c in callers if c not in guide_system)
    report[sid] = {"all_callers": sorted(callers), "nonguide_callers": nonguide}
    tag = "GUIDE-ONLY (safe to import 947)" if not nonguide else f"HAS NON-GUIDE CALLERS ({len(nonguide)}) -> keep 910"
    if nonguide:
        has_nonguide.append(sid)
    else:
        guide_only.append(sid)
    print(f"  script {sid}: {len(callers)} callers, {len(nonguide)} non-guide -> {tag}")
    if nonguide:
        print(f"      non-guide callers: {nonguide[:25]}{' ...' if len(nonguide)>25 else ''}")

print(f"\nSUMMARY changed-shared: {len(guide_only)} guide-only (import 947), {len(has_nonguide)} keep-910")
json.dump({"core": sorted(core), "new": sorted(new_ids),
           "changed_shared": sorted(changed_shared),
           "safe_shared": sorted(safe_shared),
           "guide_only_changed": sorted(guide_only),
           "keep_910_changed": sorted(has_nonguide),
           "changed_caller_report": report},
          open("/tmp/core_closure_analysis.json","w"), indent=2)
print("\nwrote /tmp/core_closure_analysis.json")
