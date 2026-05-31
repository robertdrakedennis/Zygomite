#!/usr/bin/env python3
"""From the --all-scripts transpile output, extract the byte-exact @cs2 ASM
trailer for each needed script id into scriptNNNN.asm.ts (pragma-asm mode),
assemble for 910 (enum alias + noVerify), and emit CacheOverlay entries."""
import json, os, re, glob, subprocess, hashlib, sys

ALLTS = "/tmp/allts"
DEST = "/Users/robert/projects/alerion/server/cache-patches/skills-29/scripts"
BIN = "/Users/robert/projects/alerion/tools/rs3-cache-rs/target/release/rs3-cache-rs"
C910 = "/Users/robert/projects/alerion/cache/unpacked/910"
DATA = "/Users/robert/projects/alerion/tools/rs3-cache-rs/data"

GRID8 = [14578, 15721, 15891, 16004, 16005, 16574, 17470, 19632]
ALL32 = [14490,14578,14602,14603,14610,14715,14716,14739,14789,15721,15891,16004,
         16005,16574,17442,17443,17470,17495,17498,17503,17504,17505,17507,17510,
         17511,17512,17513,17522,17527,17670,17817,19632]
WANT = set(int(a) for a in sys.argv[1:]) or set(ALL32)

# index /tmp/allts by group_id (file_id==0)
id2path = {}
for ts in glob.glob(f"{ALLTS}/*.ts"):
    try:
        with open(ts) as f:
            head = f.readline()
    except Exception:
        continue
    m = re.search(r'@rs3cache-meta\s+(\{.*\})', head)
    if not m:
        continue
    try:
        meta = json.loads(m.group(1))
    except Exception:
        continue
    if meta.get("file_id") == 0 and meta.get("group_id") in WANT:
        id2path[meta["group_id"]] = ts

def extract_asm(src, sid):
    lines = open(src).read().splitlines()
    asm = [ln for ln in lines if ln.startswith("// @cs2 ")]
    if not asm:
        return None
    # ensure a name line exists (assemble metadata; synthesize if absent)
    if not any(ln.startswith("// @cs2 name ") for ln in asm):
        asm.insert(0, f'// @cs2 name "[clientscript,script{sid}]"')
    return "\n".join(asm) + "\n"

entries, fails = [], []
for sid in sorted(WANT):
    src = id2path.get(sid)
    if not src:
        fails.append((sid, "not found in allts")); continue
    body = extract_asm(src, sid)
    if not body:
        fails.append((sid, "no @cs2 trailer")); continue
    dest = f"{DEST}/script{sid}.asm.ts"
    open(dest, "w").write(body)
    out = f"/tmp/asm-x-{sid}.cs2"
    r = subprocess.run([BIN, "--cache-dir", C910, "--data-dir", DATA, "--build", "910",
                        "--subbuild", "0", "assemble-script", "--input", dest,
                        "--output", out, "--no-verify"], capture_output=True, text=True)
    if r.returncode != 0:
        fails.append((sid, "assemble: " + (r.stderr or r.stdout).strip()[:140])); continue
    sha = hashlib.sha256(open(dest, "rb").read()).hexdigest()
    entries.append((sid, sha, os.path.getsize(out)))

print(f"assembled {len(entries)}/{len(WANT)}; failed {len(fails)}")
for sid, why in fails:
    print(f"  FAIL {sid}: {why}")
print("=== entries ===")
for sid, sha, nb in entries:
    flag = "  // GRID" if sid in GRID8 else ""
    print(f"    {{scriptId: {sid}, input: 'scripts/script{sid}.asm.ts', noVerify: true, sha256: '{sha}'}},{flag}")
json.dump({"entries":[{"scriptId":s,"sha256":h,"bytes":b} for s,h,b in entries],"fails":fails},
          open("/tmp/extract_entries.json","w"), indent=1)
