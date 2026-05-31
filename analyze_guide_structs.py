#!/usr/bin/env python3
"""Compute the skill-guide activity-list STRUCT closure (947) and classify each
struct vs 910 (identical / changed / new), so Phase 1 imports only what differs.

Closure root: enum_5420 + enum_5421 (skill -> activity enum) for all 29 skills,
then each activity enum's values (-> struct ids). Also includes struct->struct
refs from refs/struct.json (defensive; activity structs are usually leaf).
"""
import json, re, sys
from collections import defaultdict

A = "/Users/robert/projects/alerion"

def parse_enums(path):
    """returns {enum_id: {'out': outputtype, 'vals':[(key,valstr)]}}"""
    enums = {}
    cur = None
    out = None
    vals = []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            m = re.match(r"^\[enum_(\d+)\]$", line)
            if m:
                if cur is not None:
                    enums[cur] = {"out": out, "vals": vals}
                cur = int(m.group(1)); out = None; vals = []
                continue
            if cur is None:
                continue
            if line.startswith("outputtype="):
                out = line.split("=", 1)[1]
            elif line.startswith("val="):
                body = line[4:]
                key, _, v = body.partition(",")
                vals.append((key, v))
    if cur is not None:
        enums[cur] = {"out": out, "vals": vals}
    return enums

def name_to_id(s):
    m = re.match(r"^\w+?_(\d+)$", s.strip())
    return int(m.group(1)) if m else None

def parse_structs(path):
    """returns {struct_id: normalized_text_block}"""
    structs = {}
    cur = None
    buf = []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            m = re.match(r"^\[struct_(\d+)\]$", line)
            if m:
                if cur is not None:
                    structs[cur] = "\n".join(buf).strip()
                cur = int(m.group(1)); buf = []
                continue
            if cur is None:
                continue
            if line.strip() == "":
                continue
            buf.append(line)
    if cur is not None:
        structs[cur] = "\n".join(buf).strip()
    return structs

enums947 = parse_enums(f"{A}/cache/rs3-cache/947-all/config/dump.enum")

# Resolve enum_5420 + enum_5421 -> activity enum ids (all 29 skills)
activity_enum_ids = set()
skill_to_activity = {}
for root in (5420, 5421):
    e = enums947.get(root)
    if not e:
        print(f"WARN: enum_{root} missing in 947", file=sys.stderr); continue
    for key, v in e["vals"]:
        aid = name_to_id(v)
        if aid is not None:
            activity_enum_ids.add(aid)
            skill_to_activity.setdefault(int(key), []).append(aid)

print(f"activity enums referenced: {len(activity_enum_ids)}")

# Collect struct ids from each activity enum
struct_ids = set()
enum_struct_counts = {}
for aid in sorted(activity_enum_ids):
    e = enums947.get(aid)
    if not e:
        print(f"WARN: activity enum_{aid} missing in 947", file=sys.stderr); continue
    cnt = 0
    for key, v in e["vals"]:
        sid = name_to_id(v) if v.startswith("struct_") else None
        if sid is not None:
            struct_ids.add(sid); cnt += 1
    enum_struct_counts[aid] = cnt

print(f"struct ids in activity closure: {len(struct_ids)}")

# struct->struct refs (defensive closure expansion)
try:
    sref = json.load(open(f"{A}/cache/rs3-cache/947-all/refs/struct.json"))
    added = True
    while added:
        added = False
        for sid in list(struct_ids):
            refs = sref.get(str(sid), {})
            for child in refs.get("struct", []) or []:
                if child not in struct_ids:
                    struct_ids.add(child); added = True
except FileNotFoundError:
    pass
print(f"struct ids after struct->struct expansion: {len(struct_ids)}")

# Classify vs 910
structs947 = parse_structs(f"{A}/cache/rs3-cache/947-all/config/dump.struct")
structs910 = parse_structs(f"{A}/cache/rs3-cache/910-all/config/dump.struct")

new, changed, identical, missing947 = [], [], [], []
for sid in sorted(struct_ids):
    s947 = structs947.get(sid)
    s910 = structs910.get(sid)
    if s947 is None:
        missing947.append(sid)
    elif s910 is None:
        new.append(sid)
    elif s947 != s910:
        changed.append(sid)
    else:
        identical.append(sid)

print(f"\n=== struct classification (947 guide-activity closure) ===")
print(f"  new (947-only):   {len(new)}")
print(f"  changed vs 910:   {len(changed)}")
print(f"  identical:        {len(identical)}")
print(f"  missing in 947:   {len(missing947)} (closure ref but no 947 def: {missing947[:20]})")

# Import set = new + changed (identical already in 910)
import_ids = sorted(set(new) | set(changed))
print(f"\nIMPORT struct ids (new+changed): {len(import_ids)}")

# Group by struct group (id>>>5), 32/group
groups = defaultdict(list)
for sid in import_ids:
    groups[sid >> 5].append(sid & 31)
print(f"affected struct GROUPS: {len(groups)}")
print(f"group->files sample: {dict(list(sorted(groups.items()))[:8])}")

json.dump({
    "activity_enum_ids": sorted(activity_enum_ids),
    "skill_to_activity": {str(k): v for k, v in sorted(skill_to_activity.items())},
    "struct_closure": sorted(struct_ids),
    "new": new, "changed": changed, "identical": identical, "missing947": missing947,
    "import_ids": import_ids,
    "groups": {str(g): sorted(files) for g, files in sorted(groups.items())},
}, open("/tmp/guide_struct_closure.json", "w"), indent=1)
print("\nwrote /tmp/guide_struct_closure.json")
