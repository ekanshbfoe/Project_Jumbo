"""The `director` job: a chapter's panels in, its ScriptDoc out, written scene by scene by Director v3 (passes.py).

Input blob (JSON):

    {"panels": PanelsDoc,
     "files": {"chapters/ch_0001/panels/crops/p_0001.png": "<blob>", ...},  # every panel's crop
     "memory": "story so far",                                              # optional: from the previous chapter
     "last_lines": ["…", "…"]}                                              # optional: its last two lines

The chapter is cut into consecutive scenes of at most SCENE_PANELS panels, in near-equal parts (spike 7e's rule).
Scenes run in reading order and each passes its MEMORY and last lines to the next, so the story carries through,
and on to the next chapter through the manifest. A scene whose passes fail is tried once more; if it fails again
its panels are left out and the report says so, so the work of the other scenes is kept.

Output manifest:

    {"chapter_id", "script": ScriptDoc, "memory": "story so far", "last_lines": [...],
     "report": {"scenes": [{"scene", "panels", "summary", "pass2_mode", "panel_repairs", "cleanup_notes", "wall_s",
                            "error"?}], "failed": ["s03"], "problems": ["lines.4.narration: ..."]},
     "raw": [every pass's raw output per scene]}
"""

from __future__ import annotations

import asyncio
import base64
import json
import time

from ...contracts import PanelsDoc, ScriptDoc
from ...contracts.rules import parse, script_problems
from ...jobs import JobContext, JobError
from . import passes

MODEL = "qwen3vl_32b"
SCENE_PANELS = 15
# Long side of the panel images the model sees: 20 panels at 512 px cost ~6K prompt tokens (spike 7b).
IMAGE_SIDE = 512
ATTEMPTS = 2


def split_scenes(panel_ids: list[str], size: int = SCENE_PANELS) -> list[list[str]]:
    """Consecutive scenes of at most `size` panels, as equal as possible."""
    if not panel_ids:
        return []
    n = -(-len(panel_ids) // size)
    k, extra = divmod(len(panel_ids), n)
    out, start = [], 0
    for i in range(n):
        end = start + k + (1 if i < extra else 0)
        out.append(panel_ids[start:end])
        start = end
    return out


def panel_lines(panel) -> list[dict]:
    """What the reader found in a panel, sound effects left out (spike 7 input)."""
    return [{"speaker": b.speaker or "narration", "text": b.text} for b in panel.bubbles or [] if b.text and b.kind != "sfx"]


def encode_image(path: str, side: int = IMAGE_SIDE) -> str:
    import cv2

    img = cv2.imread(path)
    if img is None:
        raise JobError(f"cannot read panel image {path}")
    h, w = img.shape[:2]
    k = side / max(h, w)
    if k < 1:
        img = cv2.resize(img, (round(w * k), round(h * k)), interpolation=cv2.INTER_AREA)
    ok, buf = cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 90])
    if not ok:
        raise JobError(f"cannot encode panel image {path}")
    return base64.b64encode(buf.tobytes()).decode()


async def run(ctx: JobContext) -> str:
    try:
        request = json.loads(ctx.store.path(ctx.input).read_bytes())
    except (OSError, ValueError) as e:
        raise JobError(f"director needs a JSON request blob: {e}") from e
    doc = parse(PanelsDoc, request.get("panels"))
    files: dict[str, str] = request.get("files") or {}
    panels = sorted(doc.panels, key=lambda p: p.order)
    missing = [p.id for p in panels if not (p.crop and files.get(p.crop) and ctx.store.has(files[p.crop]))]
    if missing:
        raise JobError(f"panel crops not uploaded: {missing[:10]}{' …' if len(missing) > 10 else ''}")
    by_id = {p.id: p for p in panels}
    prompts = passes.load_prompts()
    memory, last = request.get("memory") or None, list(request.get("last_lines") or [])

    async def chat(messages, **params):
        return await ctx.call(MODEL, "chat", {"messages": messages, **params})

    scenes = split_scenes([p.id for p in panels])
    lines: list[dict] = []
    report: list[dict] = []
    raw: list[dict] = []
    failed: list[str] = []
    for i, ids in enumerate(scenes):
        name = f"s{i + 1:02d}"
        ctx.progress(0.02 + 0.96 * i / len(scenes), f"scene {i + 1}/{len(scenes)} ({len(ids)} panels)")
        scene = {"name": name, "panels": [{"panel_id": pid, "lines": panel_lines(by_id[pid]),
                                           "jpg": await asyncio.to_thread(encode_image, str(ctx.store.path(files[by_id[pid].crop])))}
                                          for pid in ids]}
        t = time.monotonic()
        for _ in range(ATTEMPTS):
            res = await passes.run_scene(chat, prompts, scene, memory, last, doc.chapter_id, log=lambda m: None)
            if res.get("doc"):
                break
        res["wall_s"] = round(time.monotonic() - t, 1)
        raw.append(res)
        entry = {"scene": name, "panels": ids, **{k: res.get(k) for k in ("summary", "pass2_mode", "panel_repairs", "cleanup_notes", "wall_s")}}
        if not res.get("doc"):
            entry["error"] = res.get("error") or (res.get("pass1") or {}).get("error") or "no script"
            failed.append(name)
        else:
            lines += res["doc"]["lines"]
            if res.get("memory_out"):
                memory, last = res["memory_out"], res.get("last_lines_out") or []
        report.append(entry)
    if not lines:
        raise JobError(f"the Director wrote nothing: {report[0].get('error') if report else 'no panels'}")
    for n, line in enumerate(lines, 1):
        line["id"] = f"l_{n:04d}"
    script = parse(ScriptDoc, {"schema_version": 1, "chapter_id": doc.chapter_id, "lines": lines})
    ctx.progress(1.0, f"{len(lines)} lines from {len(scenes) - len(failed)}/{len(scenes)} scenes")
    manifest = {
        "chapter_id": doc.chapter_id,
        "script": script.model_dump(mode="json", exclude_none=True),
        "memory": memory,
        "last_lines": last,
        "report": {"scenes": report, "failed": failed, "problems": script_problems(script)},
        "raw": raw,
    }
    return ctx.store.put(json.dumps(manifest, ensure_ascii=False).encode())
