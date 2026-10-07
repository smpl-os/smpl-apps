#!/usr/bin/python3
"""Owner-side native readback through the lease driver's /Editor op (read-only methods
plus explicitly named history/save verification calls). Usage:
  rb.py RUN LABEL                -> state summary (history, clip ranges, effect values, tracks)
  rb.py RUN LABEL undo|redo [N]  -> native history.undo/redo N times (verification only)"""
import json, subprocess, sys, time
from pathlib import Path
F = Path("/home/blin/.copilot/session-state/9147f090-d696-4b70-af0f-aa5a935f364f/files")
run = Path(sys.argv[1]); label = sys.argv[2]
fx = json.loads((run / "fixture.json").read_text())
pid = fx["project"]["project_id"]
def editor(method, params):
    out = subprocess.run([str(F / "k23_ctl.py"), str(run), json.dumps({"op": "editor", "method": method, "params": params}), "60"],
                         capture_output=True, text=True).stdout
    res = json.loads(out.split(" ", 1)[1])
    return res.get("reply", res)
tl = editor("timeline.get", {"expected_project_id": pid, "sequence_id": fx["timeline"]["sequence_id"]})
seq = tl["result"]["sequence_id"]
if len(sys.argv) > 3:
    for _ in range(int(sys.argv[4]) if len(sys.argv) > 4 else 1):
        tl = editor("timeline.get", {"expected_project_id": pid, "sequence_id": seq})
        r = editor("history." + sys.argv[3], {"expected_project_id": pid, "sequence_id": seq,
                                               "expected_undo_index": tl["result"]["history"]["undo_index"]})
        print(sys.argv[3], json.dumps(r)[:300], file=sys.stderr)
    tl = editor("timeline.get", {"expected_project_id": pid, "sequence_id": seq})
t = tl["result"]
summary = {"label": label, "at": time.time(), "history": t["history"], "selection": t.get("selection"),
           "tracks": [{k: tr.get(k) for k in ("track_id", "audio", "muted", "hidden", "locked", "name", "target", "solo")} for tr in t["tracks"]],
           "clips": [{k: c.get(k) for k in ("clip_id", "track_id", "position_frame", "end_frame", "duration_frames", "producer_in_frame", "group_members")} for c in t["clips"]],
           "effects": {}}
for clip in (fx["video_clip_id"], fx["audio_clip_id"]):
    e = editor("effects.get", {"expected_project_id": pid, "sequence_id": seq, "clip_id": clip})
    eff = {}
    for ef in e.get("result", {}).get("effects", []):
        eff[ef["asset_id"]] = {p["name"]: p.get("value") for p in ef.get("parameters", [])}
        eff[ef["asset_id"]]["_keyframes"] = {p["name"]: p.get("keyframe_count", p.get("keyframes")) for p in ef.get("parameters", []) if "keyframe_count" in p or "keyframes" in p}
    summary["effects"][str(clip)] = eff
(run / "readback").mkdir(exist_ok=True)
(run / "readback" / f"{label}.json").write_text(json.dumps({"summary": summary, "timeline": t}, indent=1))
print(json.dumps(summary, indent=1)[:6000])
