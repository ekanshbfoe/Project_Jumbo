"""Director v3 (spike 7e, frozen as Method 1): three passes per scene, the same on T4 (llama.cpp) and TPU (vLLM).

Pass 1  free writing: beats, Hindi narration tagged with panels, a memory update. No JSON, no limits.
Pass 2  the editor: a small JSON object with the camera move, emotion and voice for each Pass 1 line
        (same conversation, so the panel images are not read again).
Pass 3  Python cleanup: panel coverage, numbers as Hindi words, English loanwords in Latin letters, measurements.

`chat(messages, **params)` is the backend, awaited: it sends an OpenAI-style chat request and returns
{"status", "content", "finish", "usage", "timings", "wall_s", "error"}.

The prompts, sampling and cleanup rules are frozen; change them only with a new spike.
"""

from __future__ import annotations

import copy
import json
import re
from pathlib import Path

PROMPTS = Path(__file__).parent / "prompts"
PROMPT_FILES = ("pass1_system.md", "pass2_user.md", "camera.schema.json")


def load_prompts(root: Path = PROMPTS) -> dict[str, str]:
    """The prompt files by name. camera.schema.json is ScriptDoc's JSON Schema with Point written as a plain
    array, since llama.cpp's grammar converter rejects tuple arrays."""
    return {name: (root / name).read_text(encoding="utf-8") for name in PROMPT_FILES}

EMOTIONS = ("calm", "tense", "action", "shock", "sad", "comedic", "romantic", "mysterious", "triumphant")
SINGLE = ("HoldPreset", "ZoomPreset", "PanPreset", "ShakePreset", "ActionTiltPreset", "KineticJitterPreset", "PanoramicFadePreset", "HudPopupPreset")
MULTI = ("SplitScreenPreset", "FloatingPanelsPreset", "HorizontalSlicesPreset")
MULTI_NAMES = ("split_screen", "floating_panels", "horizontal_slices")
PASS1_SAMPLING = {"temperature": 0.7, "top_p": 0.8, "top_k": 20, "presence_penalty": 1.5}
PASS2_SAMPLING = {"temperature": 0.3, "top_p": 0.8, "top_k": 20, "presence_penalty": 0.0}


# ---------- Pass 1 ----------

def pass1_messages(files, scene, memory, last_lines):
    content = [{"type": "text", "text": f"Story so far: {memory or 'This is the first scene of the chapter.'}\n"
                                        f"Last narrated lines: {' / '.join(last_lines) if last_lines else 'none'}\n\n"
                                        f"Scene {scene['name']}: {len(scene['panels'])} panels follow in reading order."}]
    for p in scene["panels"]:
        text = " / ".join(f"[{l['speaker']}] {l['text']}" for l in p["lines"]) if p["lines"] else "(no text)"
        content.append({"type": "text", "text": f"Panel {p['panel_id']}. Text: {text}"})
        content.append({"type": "image_url", "image_url": {"url": "data:image/jpeg;base64," + p["jpg"]}})
    content.append({"type": "text", "text": "Narrate this scene now, starting with BEATS."})
    return [{"role": "system", "content": files["pass1_system.md"]}, {"role": "user", "content": content}]


LINE_RE = re.compile(r"^\s*L(\d+)\s*\[([^\]]*)\]\s*(?:\(([^)]*)\))?\s*(.+?)\s*$")


def parse_pass1(text):
    """{"beats": str, "lines": [{"n", "panel_ids", "emotion", "narration"}], "memory": str}."""
    text = text or ""
    sections = re.split(r"^\s*(BEATS|NARRATION|MEMORY)\s*:\s*$", text, flags=re.M)
    part = {sections[i]: sections[i + 1].strip() for i in range(1, len(sections) - 1, 2)}
    lines = []
    for raw in (part.get("NARRATION") or text).splitlines():
        m = LINE_RE.match(raw)
        if not m:
            continue
        ids = re.findall(r"p_\d{4}", m[2])
        emotion = (m[3] or "").strip().lower()
        lines.append({"n": int(m[1]), "panel_ids": ids, "emotion": emotion if emotion in EMOTIONS else None, "narration": m[4].strip()})
    return {"beats": part.get("BEATS", ""), "lines": lines, "memory": part.get("MEMORY", "")}


def fix_panels(lines, scene):
    """Every panel on screen exactly once, in reading order, at most 3 per line. Returns (lines, repairs)."""
    order = [p["panel_id"] for p in scene["panels"]]
    pos = {pid: i for i, pid in enumerate(order)}
    repairs, seen = [], set()
    for l in lines:
        ids = []
        for pid in l["panel_ids"]:
            if pid not in pos:
                repairs.append(f"L{l['n']}: unknown panel {pid} dropped")
            elif pid in seen:
                repairs.append(f"L{l['n']}: {pid} already shown, dropped")
            else:
                ids.append(pid)
                seen.add(pid)
        ids.sort(key=pos.get)
        if len(ids) > 3:
            repairs.append(f"L{l['n']}: {len(ids)} panels, kept the first 3")
            for pid in ids[3:]:
                seen.discard(pid)
            ids = ids[:3]
        l["panel_ids"] = ids
    lines = [l for l in lines if l["panel_ids"] or not repairs.append(f"L{l['n']}: no valid panel, line merged away")]
    for pid in order:
        if pid in seen:
            continue
        # Attach a missing panel to the line holding its nearest neighbour, if that line has room.
        best = None
        for l in lines:
            if len(l["panel_ids"]) < 3:
                d = min(abs(pos[pid] - pos[q]) for q in l["panel_ids"])
                if best is None or d < best[0]:
                    best = (d, l)
        if best and best[0] == 1:
            best[1]["panel_ids"] = sorted(best[1]["panel_ids"] + [pid], key=pos.get)
            seen.add(pid)
            repairs.append(f"{pid} was not narrated: joined L{best[1]['n']}")
        else:
            repairs.append(f"{pid} was not narrated and has no neighbour line with room")
    lines.sort(key=lambda l: pos[l["panel_ids"][0]])
    return lines, repairs


# ---------- Pass 2 ----------

def pass2_schema(base, lines):
    """One schema entry per Pass 1 line: the preset choice is limited by that line's panel count."""
    s = copy.deepcopy(base)
    d = s["$defs"]
    items = []
    for i, l in enumerate(lines, 1):
        presets = MULTI if len(l["panel_ids"]) > 1 else SINGLE
        items.append({"type": "object",
                      "properties": {"id": {"type": "string", "const": f"L{i}"},
                                     "preset": {"oneOf": [{"$ref": f"#/$defs/{p}"} for p in presets]},
                                     "emotion": {"type": "string", "enum": list(EMOTIONS)},
                                     "voice": {"type": "object",
                                               "properties": {"tone": {"type": "string", "minLength": 1, "maxLength": 40},
                                                              "pace": {"type": "string", "enum": ["slow", "normal", "fast"]},
                                                              "intensity": {"type": "number", "minimum": 0, "maximum": 1}},
                                               "required": ["tone", "pace", "intensity"]},
                                     "transition_in": {"$ref": "#/$defs/TransitionSpec"}},
                      "required": ["id", "preset", "emotion", "voice", "transition_in"]})
    return {"$schema": s.get("$schema"), "type": "object",
            "properties": {"notes": {"type": "string", "maxLength": 400},
                           "lines": {"type": "array", "prefixItems": items, "minItems": len(items), "maxItems": len(items)}},
            "required": ["notes", "lines"], "$defs": {k: v for k, v in d.items() if k not in ("ScriptLine", "SingleLine", "MultiLine")}}


def pass2_loose_schema(base, n):
    """Fallback when a backend rejects prefixItems: same fields, any preset; the code repairs mismatches."""
    strict = pass2_schema(base, [{"panel_ids": ["x"]}])
    item = strict["properties"]["lines"]["prefixItems"][0]
    item["properties"]["id"] = {"type": "string", "pattern": "^L\\d+$"}
    item["properties"]["preset"] = {"oneOf": [{"$ref": f"#/$defs/{p}"} for p in SINGLE + MULTI]}
    strict["properties"]["lines"] = {"type": "array", "items": item, "minItems": n, "maxItems": n}
    return strict


def parse_json(text):
    text = re.sub(r"```(?:json)?", "", text or "")
    i = text.find("{")
    try:
        return json.JSONDecoder().raw_decode(text[i:])[0] if i >= 0 else None
    except json.JSONDecodeError:
        return None


# ---------- Pass 3: Python cleanup ----------

ONES = ("शून्य एक दो तीन चार पाँच छह सात आठ नौ दस ग्यारह बारह तेरह चौदह पंद्रह सोलह सत्रह अठारह उन्नीस बीस इक्कीस बाईस तेईस चौबीस "
        "पच्चीस छब्बीस सत्ताईस अट्ठाईस उनतीस तीस इकतीस बत्तीस तैंतीस चौंतीस पैंतीस छत्तीस सैंतीस अड़तीस उनतालीस चालीस इकतालीस बयालीस "
        "तैंतालीस चवालीस पैंतालीस छियालीस सैंतालीस अड़तालीस उनचास पचास इक्यावन बावन तिरेपन चौवन पचपन छप्पन सत्तावन अट्ठावन उनसठ साठ "
        "इकसठ बासठ तिरेसठ चौंसठ पैंसठ छियासठ सड़सठ अड़सठ उनहत्तर सत्तर इकहत्तर बहत्तर तिहत्तर चौहत्तर पचहत्तर छिहत्तर सतहत्तर अठहत्तर "
        "उन्यासी अस्सी इक्यासी बयासी तिरासी चौरासी पचासी छियासी सत्तासी अट्ठासी नवासी नब्बे इक्यानवे बानवे तिरानवे चौरानवे पचानवे छियानवे "
        "सत्तानवे अट्ठानवे निन्यानवे").split()
assert len(ONES) == 100


def hindi_number(n):
    if n < 100:
        return ONES[n]
    for size, word in ((10 ** 7, "करोड़"), (10 ** 5, "लाख"), (1000, "हज़ार"), (100, "सौ")):
        if n >= size:
            head, rest = divmod(n, size)
            return f"{hindi_number(head)} {word}" + (f" {hindi_number(rest)}" if rest else "")
    return str(n)


def numbers_to_words(text):
    text = re.sub(r"\b(\d{1,2}):(\d{2})\b(\s*बजे)?", lambda m: f"{hindi_number(int(m[1]))} बजकर {hindi_number(int(m[2]))} मिनट", text)
    text = re.sub(r"(\d+)%", lambda m: f"{hindi_number(int(m[1]))} प्रतिशत", text)
    return re.sub(r"\d+", lambda m: hindi_number(int(m[0])) if len(m[0]) <= 9 else m[0], text)


# English words commonly written in Devanagari by the model; the voice reads them better in Latin letters.
LOANWORDS = {
    "ऑफिस": "office", "ऑफ़िस": "office", "बॉस": "boss", "टीम": "team", "सिस्टम": "system", "लेवल": "level", "स्किल": "skill",
    "ओवरटाइम": "overtime", "कंपनी": "company", "मैनेजर": "manager", "डिपार्टमेंट": "department", "प्रोजेक्ट": "project",
    "मीटिंग": "meeting", "बीयर": "beer", "अकाउंटिंग": "accounting", "एम्प्लॉई": "employee", "एम्प्लॉयी": "employee",
    "रग्बी": "rugby", "कॉलेज": "college", "क्लब": "club", "पार्टी": "party", "प्रोडक्शन": "production", "टीवी": "TV",
    "फोन": "phone", "फ़ोन": "phone", "ट्रेन": "train", "स्टेशन": "station", "कंप्यूटर": "computer", "डेस्क": "desk",
    "टाई": "tie", "सूट": "suit", "ज़ॉम्बी": "zombie", "जॉम्बी": "zombie", "ज़ॉम्बीज़": "zombies", "जॉम्बीज": "zombies",
    "स्टाइल": "style", "प्लान": "plan", "बॉडी": "body", "पावर": "power", "फाइट": "fight", "फ़ाइट": "fight",
}
WORD_RE = re.compile(r"[ऀ-ॿ]+")


def loanwords_to_latin(text):
    return WORD_RE.sub(lambda m: LOANWORDS.get(m[0], m[0]), text)


QUOTES_RE = re.compile(r"[\"“”‘’]|(?<![A-Za-z])'|'(?![A-Za-z])")


def measure(narration):
    words = narration.split()
    deva = sum(1 for w in words if re.search(r"[ऀ-ॿ]", w))
    return {"words": len(words), "devanagari_share": round(deva / max(len(words), 1), 2),
            "quote_marks": len(QUOTES_RE.findall(narration)), "digits": len(re.findall(r"\d", narration))}


def repair_preset(line):
    """Panel count and preset must agree (the strict Pass 2 schema guarantees it; the loose fallback does not)."""
    name = (line.get("preset") or {}).get("name")
    multi = len(line["panel_ids"]) > 1
    if multi and name not in MULTI_NAMES:
        line["preset"] = {"name": "split_screen", "params": {}}
        return "multi-panel line given a single-panel move: split_screen"
    if not multi and name in MULTI_NAMES:
        line["preset"] = {"name": "zoom", "params": {}}
        return "single-panel line given a multi-panel move: zoom"
    if name == "hud_popup" and not (line["preset"].get("params") or {}).get("text"):
        line["preset"] = {"name": "hold", "params": {}}
        return "hud_popup without text: hold"
    return None


def assemble(chapter_id, lines, camera):
    """ScriptDoc from the Pass 1 lines and the Pass 2 camera choices, after cleanup."""
    by_id = {c.get("id"): c for c in (camera or {}).get("lines", []) if isinstance(c, dict)}
    out, notes = [], []
    for i, l in enumerate(lines, 1):
        c = by_id.get(f"L{i}") or {}
        narration = loanwords_to_latin(numbers_to_words(l["narration"]))
        voice = c.get("voice") or {}
        line = {"id": f"l_{i:04d}", "panel_ids": l["panel_ids"], "narration": narration,
                "emotion": c.get("emotion") or l.get("emotion") or "calm",
                "voice": {"speaker": "narrator", "tone": voice.get("tone") or "neutral", "pace": voice.get("pace") or "normal",
                          "intensity": voice.get("intensity", 0.5)},
                "preset": c.get("preset") or {"name": "hold", "params": {}},
                "transition_in": c.get("transition_in") or {"name": "cut", "duration": 0.4}}
        if not c:
            notes.append(f"L{i}: no camera choice from Pass 2, used hold")
        fixed = repair_preset(line)
        if fixed:
            notes.append(f"L{i}: {fixed}")
        out.append(line)
    return {"schema_version": 1, "chapter_id": chapter_id, "lines": out}, notes


# ---------- One scene ----------

async def run_scene(chat, files, scene, memory, last_lines, chapter_id, log=print):
    """Returns a result dict with every pass's raw output, the final ScriptDoc and the cleanup report."""
    res = {"scene": scene["name"], "memory_in": memory, "last_lines_in": last_lines}
    msgs = pass1_messages(files, scene, memory, last_lines)
    p1 = await chat(msgs, max_tokens=4096, seed=7, **PASS1_SAMPLING)
    res["pass1"] = p1
    if p1.get("status") != 200:
        return res
    parsed = parse_pass1(p1["content"])
    res["pass1_parsed"] = {"beats": parsed["beats"], "memory": parsed["memory"], "lines": copy.deepcopy(parsed["lines"])}
    if not parsed["lines"]:
        res["error"] = "Pass 1 produced no tagged lines"
        return res
    lines, repairs = fix_panels(parsed["lines"], scene)
    res["panel_repairs"] = repairs
    log(f"{scene['name']}: pass 1 gave {len(lines)} lines, {len(repairs)} panel repairs")

    base = json.loads(files["camera.schema.json"])
    listing = "\n".join(f"L{i} {l['panel_ids']}: {l['narration'][:60]}…" for i, l in enumerate(lines, 1))
    follow = msgs + [{"role": "assistant", "content": p1["content"]},
                     {"role": "user", "content": files["pass2_user.md"] + "\n\nThe lines to direct, with their final panels:\n" + listing}]
    camera, mode = None, "strict"
    for schema in (pass2_schema(base, lines), pass2_loose_schema(base, len(lines)), None):
        body = {"max_tokens": 2048, "seed": 7, **PASS2_SAMPLING}
        if schema:
            body["response_format"] = {"type": "json_schema", "json_schema": {"name": "Camera", "schema": schema}}
        p2 = await chat(follow, **body)
        if p2.get("status") == 200:
            camera = parse_json(p2["content"])
            res["pass2"] = p2
            break
        res.setdefault("pass2_rejected", []).append({"mode": mode, "error": (p2.get("error") or "")[:500]})
        mode = "loose" if mode == "strict" else "free"
    res["pass2_mode"] = mode
    doc, notes = assemble(chapter_id, lines, camera)
    res["doc"] = doc
    res["camera_notes"] = (camera or {}).get("notes")
    res["cleanup_notes"] = notes
    res["measure"] = {l["id"]: measure(l["narration"]) for l in doc["lines"]}
    words = sum(m["words"] for m in res["measure"].values())
    res["summary"] = {"lines": len(doc["lines"]), "panels": len(scene["panels"]),
                      "covered": len({p for l in doc["lines"] for p in l["panel_ids"]}),
                      "words": words, "words_per_panel": round(words / len(scene["panels"]), 1),
                      "quote_marks": sum(m["quote_marks"] for m in res["measure"].values())}
    res["memory_out"] = parsed["memory"]
    res["last_lines_out"] = [l["narration"] for l in doc["lines"][-2:]]
    return res
