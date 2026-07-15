#!/usr/bin/env python3
"""Analyze a samply profile: aggregate + time-windowed self-time by method.
Usage: HASH=<hash> PORT=<port> PROF=<file.json.gz> python prof_analyze.py [NWINDOWS]
Requires a running `samply load --no-open --port PORT PROF` server.
"""
import gzip, json, collections, urllib.request, os, re, sys

hash_ = os.environ["HASH"]
port = os.environ["PORT"]
prof = os.environ.get("PROF", "alpine_boot.json.gz")
NB = int(sys.argv[1]) if len(sys.argv) > 1 else 8

d = json.load(gzip.open(prof))
# pick our binary's lib (has a .pdb debugName matching the exe stem)
lib = None
for l in d["libs"]:
    dn = (l.get("debugName") or "").lower()
    if any(s in dn for s in ("rusty_box_gui", "alpine", "perfbench")):
        lib = l
        break
if lib is None:
    # fallback: largest non-system lib
    lib = d["libs"][-1]

th = max(d["threads"], key=lambda t: len(t["samples"]["stack"]))
sframe = th["stackTable"]["frame"]
faddr = th["frameTable"]["address"]
stacks = th["samples"]["stack"]
deltas = th["samples"]["timeDeltas"]
times = []
acc = 0.0
for dt in deltas:
    acc += dt
    times.append(acc)

uniq = list({faddr[sframe[s]] for s in stacks if s is not None})
url = f"http://127.0.0.1:{port}/{hash_}/symbolicate/v5"

def short(fn):
    fn = fn or "?"
    m = re.search(r">::([A-Za-z0-9_]+)", fn)
    if m:
        return m.group(1)
    return fn.split("::")[-1].split("(")[0]

# Chunked + tolerant: a few addresses trip the server's request parser; skip them.
name = {}
CH = 200
for i in range(0, len(uniq), CH):
    chunk = uniq[i:i + CH]
    req = {"jobs": [{"memoryMap": [[lib["debugName"], lib["breakpadId"]]],
                     "stacks": [[[0, int(a)] for a in chunk]]}]}
    try:
        res = json.load(urllib.request.urlopen(url, data=json.dumps(req).encode(), timeout=120))
        frames = res["results"][0]["stacks"][0]
        for a, fr in zip(chunk, frames):
            name[a] = short(fr.get("function"))
    except Exception:
        for a in chunk:
            name.setdefault(a, "?")

# aggregate
agg = collections.Counter()
tot = 0
for s in stacks:
    if s is None:
        continue
    agg[name[faddr[sframe[s]]]] += 1
    tot += 1
print(f"thread {th.get('name')} | {tot} leaf samples | wall {(times[-1]-times[0])/1000:.1f}s")
print("=== AGGREGATE self-time ===")
for nm, c in agg.most_common(30):
    print(f"{100*c/tot:6.2f}%  {nm}")

# windowed
t0, t1 = times[0], times[-1]
span = (t1 - t0) / NB
buckets = [collections.Counter() for _ in range(NB)]
btot = [0]*NB
for s, t in zip(stacks, times):
    if s is None:
        continue
    b = min(NB-1, int((t-t0)/span))
    buckets[b][name[faddr[sframe[s]]]] += 1
    btot[b] += 1
print(f"\n=== {NB} TIME WINDOWS (~{span/1000:.1f}s each) ===")
for b in range(NB):
    top = buckets[b].most_common(7)
    hdr = f"[w{b} {b*span/1000:5.1f}-{(b+1)*span/1000:5.1f}s n={btot[b]}]"
    body = "  ".join(f"{nm} {100*c/max(btot[b],1):.0f}%" for nm, c in top)
    print(f"{hdr} {body}")
