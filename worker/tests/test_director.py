"""The frozen Director v3 (passes.py) without the model: parsing Pass 1, panel coverage, the per-line Pass 2
schema, the Pass 3 cleanup, and one whole scene against a scripted chat backend."""

import json
from pathlib import Path

from jumbo_worker.contracts import ScriptDoc, rules
from jumbo_worker.pipeline.director import passes

ROOT = Path(__file__).parents[2]


def scene(n: int) -> dict:
    return {"name": "A_first_day", "panels": [{"panel_id": f"p_{i:04d}", "lines": [], "jpg": "AAAA"} for i in range(1, n + 1)]}


PASS1 = """BEATS:
- Akira arrives at the office

NARRATION:
L1 [p_0001, p_0002] (calm) Akira पहली बार ऑफिस पहुँचता है और 24 घंटे की नौकरी शुरू होती है।
L2 [p_0003] (shock) उसका बॉस उसे 3:58 पर फोन करता है।
not a line
L3 [p_0004, p_0009] (angry) वह चुपचाप बैठ जाता है।

MEMORY:
Akira joins a harsh company."""


def test_the_prompts_load_and_the_camera_schema_matches_scriptdoc():
    files = passes.load_prompts()
    assert "## Answer format" in files["pass1_system.md"]
    assert "Answer with the JSON object only." in files["pass2_user.md"]
    # The llama.cpp copy is ScriptDoc's schema with only Point rewritten; anything else is drift.
    ours = json.loads(files["camera.schema.json"])
    source = json.loads((ROOT / "packages/schema/json/ScriptDoc.schema.json").read_text())
    assert ours["$defs"]["Point"] == {"type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 2, "maxItems": 2}
    ours["$defs"]["Point"] = source["$defs"]["Point"]
    assert ours == source


def test_pass1_is_parsed_into_tagged_lines():
    parsed = passes.parse_pass1(PASS1)
    assert parsed["beats"] == "- Akira arrives at the office"
    assert parsed["memory"] == "Akira joins a harsh company."
    assert [l["panel_ids"] for l in parsed["lines"]] == [["p_0001", "p_0002"], ["p_0003"], ["p_0004", "p_0009"]]
    # An emotion outside the list is dropped, not guessed.
    assert [l["emotion"] for l in parsed["lines"]] == ["calm", "shock", None]


def test_every_panel_is_shown_once_in_reading_order():
    parsed = passes.parse_pass1(PASS1)
    lines, repairs = passes.fix_panels(parsed["lines"], scene(5))
    assert [l["panel_ids"] for l in lines] == [["p_0001", "p_0002"], ["p_0003"], ["p_0004", "p_0005"]]
    assert repairs == ["L3: unknown panel p_0009 dropped", "p_0005 was not narrated: joined L3"]


def test_a_line_left_without_panels_is_dropped():
    lines = [{"n": 1, "panel_ids": ["p_0001"], "emotion": None, "narration": "एक।"},
             {"n": 2, "panel_ids": ["p_0001", "p_0042"], "emotion": None, "narration": "दो।"},
             {"n": 3, "panel_ids": ["p_0002"], "emotion": None, "narration": "तीन।"}]
    lines, repairs = passes.fix_panels(lines, scene(2))
    assert [l["n"] for l in lines] == [1, 3]
    assert repairs == ["L2: p_0001 already shown, dropped", "L2: unknown panel p_0042 dropped", "L2: no valid panel, line merged away"]


def test_pass2_schema_ties_each_line_to_its_panel_count():
    base = json.loads(passes.load_prompts()["camera.schema.json"])
    schema = passes.pass2_schema(base, [{"panel_ids": ["p_0001", "p_0002"]}, {"panel_ids": ["p_0003"]}])
    items = schema["properties"]["lines"]["prefixItems"]
    assert [i["properties"]["id"]["const"] for i in items] == ["L1", "L2"]
    assert {r["$ref"].split("/")[-1] for r in items[0]["properties"]["preset"]["oneOf"]} == set(passes.MULTI)
    assert {r["$ref"].split("/")[-1] for r in items[1]["properties"]["preset"]["oneOf"]} == set(passes.SINGLE)
    loose = passes.pass2_loose_schema(base, 2)
    assert loose["properties"]["lines"]["minItems"] == 2 and "items" in loose["properties"]["lines"]


def test_pass3_writes_numbers_in_hindi_and_loanwords_in_latin():
    assert passes.numbers_to_words("3:58 बजे") == "तीन बजकर अट्ठावन मिनट"
    assert passes.numbers_to_words("50% और 120") == "पचास प्रतिशत और एक सौ बीस"
    assert passes.hindi_number(250000) == "दो लाख पचास हज़ार"
    assert passes.loanwords_to_latin("ऑफिस में बॉस") == "office में boss"


def test_assemble_repairs_mismatched_presets_and_fills_gaps():
    lines = [{"n": 1, "panel_ids": ["p_0001", "p_0002"], "emotion": "calm", "narration": "दो पैनल।"},
             {"n": 2, "panel_ids": ["p_0003"], "emotion": "sad", "narration": "एक पैनल।"}]
    camera = {"lines": [{"id": "L1", "preset": {"name": "zoom", "params": {}}, "emotion": "tense",
                         "voice": {"tone": "warm", "pace": "fast", "intensity": 0.7}, "transition_in": {"name": "fade", "duration": 0.3}}]}
    doc, notes = passes.assemble("ch_0001", lines, camera)
    assert doc["lines"][0]["preset"]["name"] == "split_screen"
    assert doc["lines"][0]["voice"] == {"speaker": "narrator", "tone": "warm", "pace": "fast", "intensity": 0.7}
    assert doc["lines"][1]["preset"] == {"name": "hold", "params": {}}
    assert doc["lines"][1]["emotion"] == "sad"
    assert notes == ["L1: multi-panel line given a single-panel move: split_screen", "L2: no camera choice from Pass 2, used hold"]
    script = ScriptDoc.model_validate(doc)
    assert rules.script_doc(script) == []


async def test_a_whole_scene_runs_through_both_passes():
    calls = []

    async def chat(messages, **params):
        calls.append(params)
        if len(calls) == 1:
            return {"status": 200, "content": PASS1}
        if "response_format" in params and len(calls) == 2:
            return {"status": 400, "error": "prefixItems not supported"}
        return {"status": 200, "content": json.dumps({"notes": "calm open", "lines": [
            {"id": "L1", "preset": {"name": "split_screen", "params": {}}, "emotion": "calm",
             "voice": {"tone": "neutral", "pace": "normal", "intensity": 0.4}, "transition_in": {"name": "cut", "duration": 0.2}},
            {"id": "L2", "preset": {"name": "zoom", "params": {}}, "emotion": "shock",
             "voice": {"tone": "tense", "pace": "fast", "intensity": 0.8}, "transition_in": {"name": "cut", "duration": 0.2}},
            {"id": "L3", "preset": {"name": "hold", "params": {}}, "emotion": "sad",
             "voice": {"tone": "grim", "pace": "slow", "intensity": 0.3}, "transition_in": {"name": "fade", "duration": 0.3}}]})}

    res = await passes.run_scene(chat, passes.load_prompts(), scene(5), None, [], "ch_0001", log=lambda m: None)
    assert calls[0] == {"max_tokens": 4096, "seed": 7, **passes.PASS1_SAMPLING}
    assert res["pass2_mode"] == "loose"
    assert res["summary"] == {"lines": 3, "panels": 5, "covered": 5, "words": res["summary"]["words"],
                              "words_per_panel": res["summary"]["words_per_panel"], "quote_marks": 0}
    narration = res["doc"]["lines"][0]["narration"]
    assert "चौबीस" in narration and "office" in narration
    assert res["memory_out"] == "Akira joins a harsh company."
    assert rules.script_doc(ScriptDoc.model_validate(res["doc"])) == []
