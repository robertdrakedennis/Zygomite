#!/usr/bin/env python3
"""Select the correct transpiled .ts per new-script id (by @rs3cache-meta group_id),
copy to the skills-29 scripts dir as scriptNNNN.asm.ts, assemble-test for 910
(alias + noVerify), and emit the CacheOverlay patch entries with sha256."""
import json, os, re, glob, subprocess, hashlib, sys

NEW = [14490,14578,14602,14603,14610,14715,14716,14739,14789,15721,15891,16004,
       16005,16574,17442,17443,17470,17495,17498,17503,17504,17505,17507,17510,
       17511,17512,17513,17522,17527,17670,17817,19632]
DEST = "/Users/robert/projects/alerion/server/cache-patches/skills-29/scripts"
BIN = "/Users/robert/projects/alerion/tools/rs3-cache-rs/target/release/rs3-cache-rs"
C910 = "/Users/robert/projects/alerion/cache/unpacked/910"
DATA = "/Users/robert/projects/alerion/tools/rs3-cache-rs/data"

def meta_group_id(path):
    with open(path) as f:
        head = f.readline()
    m = re.search(r'@rs3cache-meta\s+(\{.*\})', head)
    if not m:
        return None
    try:
        return json.loads(m.group(1)).get("group_id")
    except Exception:
        return None

entries = []
fails = []
for sid in NEW:
    d = f"/tmp/newscripts/d{sid}"
    cand = None
    for ts in glob.glob(f"{d}/*.ts"):
        if meta_group_id(ts) == sid:
            cand = ts; break
    if not cand:
        fails.append((sid, "no transpiled .ts with matching group_id")); continue
    dest = f"{DEST}/script{sid}.asm.ts"
    data = open(cand, "rb").read()
    open(dest, "wb").write(data)
    out = f"/tmp/asm-new-{sid}.cs2"
    r = subprocess.run([BIN, "--cache-dir", C910, "--data-dir", DATA, "--build", "910",
                        "--subbuild", "0", "assemble-script", "--input", dest,
                        "--output", out, "--no-verify"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        fails.append((sid, "assemble failed: " + (r.stderr.strip() or r.stdout.strip())[:160])); continue
    sha = hashlib.sha256(data).hexdigest()
    nbytes = os.path.getsize(out)
    entries.append((sid, sha, nbytes))

print(f"OK {len(entries)}/{len(NEW)} assembled; {len(fails)} failed")
for sid, why in fails:
    print(f"  FAIL {sid}: {why}")
print("\n=== CacheOverlay patch entries ===")
for sid, sha, nbytes in entries:
    print(f"    {{scriptId: {sid}, input: 'scripts/script{sid}.asm.ts', noVerify: true, sha256: '{sha}'}},  // {nbytes}B")
json.dump({"entries": [{"scriptId": s, "sha256": h, "bytes": b} for s, h, b in entries],
           "fails": fails}, open("/tmp/newscript_entries.json", "w"), indent=1)
print("\nwrote /tmp/newscript_entries.json")
