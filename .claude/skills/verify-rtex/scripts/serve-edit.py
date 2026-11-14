#!/usr/bin/env python3
"""serve-edit.py RUN_ID [--find TEXT] [--text TEXT] — drive `rtex serve` the way an editor does.

Opens a session on the run's project, waits for the first settled layout, inserts --text after
the first occurrence of --find in main.tex, then waits for the live ParagraphUpdate, requests a layout and waits
for it. Writes transcript.jsonl (every event, display lists reduced to counts) and
live-edit.json (the verdict) to build/verify-evidence/RUN_ID/. Exit 0 only when both arrived."""
import argparse, json, os, queue, subprocess, sys, threading, time

ap = argparse.ArgumentParser()
ap.add_argument("run")
ap.add_argument("--find", default="\n\n")
ap.add_argument("--text", default=" verified")
a = ap.parse_args()
root = subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
scratch, evidence = f"{root}/build/verify-run/{a.run}", f"{root}/build/verify-evidence/{a.run}"
project = f"{scratch}/project"
src = open(f"{project}/main.tex").read()
# the default lands at the start of a paragraph in the middle of the book
at = src.index(a.find, len(src) // 2 if a.find == "\n\n" else 0) + len(a.find)

p = subprocess.Popen([f"{root}/target/release/rtex", "serve", "--project", project, "--build", f"{scratch}/session"],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(f"{evidence}/serve.stderr.txt", "w"), text=True, bufsize=1)
open(f"{scratch}/serve.pid", "w").write(str(p.pid))
q = queue.Queue()
threading.Thread(target=lambda: [q.put(l) for l in p.stdout], daemon=True).start()
log = open(f"{evidence}/transcript.jsonl", "w")
t0 = time.monotonic()

def slim(m):
    def dl(d):
        return {"kind": d.get("kind"), "lines": len(d.get("lines", [])), "glyphs": sum(1 for l in d.get("lines", []) for i in l.get("items", []) if i and i[0] == "g"), "flags": d.get("flags")}
    m = dict(m)
    if "dl" in m: m["dl"] = dl(m["dl"])
    if "pages_changed" in m: m["pages_changed"] = [{"page": pg["page"], "exact": pg["exact"], "dl": dl(pg["dl"])} for pg in m["pages_changed"]]
    for k in ("placements", "eligible_paragraphs"):
        if k in m: m[k] = len(m[k])
    return m

def wait(pred, what, timeout=180):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try: line = q.get(timeout=1)
        except queue.Empty: continue
        m = json.loads(line)
        s = slim(m)
        log.write(json.dumps({"t": round(time.monotonic() - t0, 3), **s}) + "\n"); log.flush()
        if m.get("reply") == "error": fail(f"serve error: {m.get('message')}")
        if pred(m): return s
    fail(f"timeout waiting for {what}")

def fail(why):
    json.dump({"ok": False, "why": why}, open(f"{evidence}/live-edit.json", "w"), indent=1)
    p.kill(); print("FAIL:", why); sys.exit(1)

def send(o):
    log.write(json.dumps({"t": round(time.monotonic() - t0, 3), "sent": o}) + "\n")
    p.stdin.write(json.dumps(o) + "\n"); p.stdin.flush()

# a layout with compile errors stays "Converging" and no pass follows it
settled = lambda m: m.get("event") == "LayoutUpdate" and (
    m["convergence"]["state"] in ("Converged", "PassLimitReached") or m["compile"]["state"] != "Ok")
first = wait(settled, "the first layout")
send({"cmd": "edit", "path": "main.tex", "start": at, "end": at, "text": a.text})
reply = wait(lambda m: m.get("reply") == "edit", "the edit reply")
verdict = {"edit_at_byte": at, "inserted": a.text, "first_layout": {k: first[k] for k in ("pages_total", "passes", "wall_ms")}, "routed": reply["result"]["routed"], "reasons": reply["result"]["reasons"]}
if reply["result"]["routed"] == "fast":
    par = wait(lambda m: m.get("event") == "ParagraphUpdate", "the ParagraphUpdate", 30)
    verdict["paragraph"] = {"status": par["status"], "lines": par["dl"]["lines"], "glyphs": par["dl"]["glyphs"], "engine_ms": par["timing"]["total_us"] / 1e3,
                            "pagination_stale": par["pagination_stale"]}
# a pass follows on its own only when the edit left pagination stale: ask for one
send({"cmd": "request_layout"})
lay = wait(settled, "the layout after the edit")
verdict["layout_after"] = {"compile": lay["compile"]["state"], "state": lay["convergence"]["state"], "pages_changed": [pg["page"] for pg in lay["pages_changed"]], "wall_ms": lay["wall_ms"]}
send({"cmd": "quit"})
p.wait(timeout=30)
os.remove(f"{scratch}/serve.pid")
par = verdict.get("paragraph", {})
verdict["ok"] = par.get("status") == "ok" and par.get("glyphs", 0) > 0 and lay["compile"]["state"] == "Ok" and len(lay["pages_changed"]) >= 1
json.dump(verdict, open(f"{evidence}/live-edit.json", "w"), indent=1)
print(json.dumps(verdict))
sys.exit(0 if verdict["ok"] else 1)
