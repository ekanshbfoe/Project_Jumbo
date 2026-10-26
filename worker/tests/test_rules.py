"""Each cross-field rule mirrors a zod refinement in packages/schema; the messages match too."""

import copy
import json
from pathlib import Path

import pytest

from jumbo_worker.contracts import AudioManifest, EpisodeSpec, LedgerEvent, MemoryDelta, PanelsDoc, ScriptDoc
from jumbo_worker.contracts.rules import ContractError, parse, problems

FIXTURES = Path(__file__).resolve().parents[2] / "packages" / "schema" / "fixtures"


def fixture(name: str) -> dict:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


@pytest.mark.parametrize(
    ("model", "name"),
    [
        (PanelsDoc, "panels-doc.sample.json"),
        (ScriptDoc, "script-doc.sample.json"),
        (AudioManifest, "audio-manifest.sample.json"),
        (EpisodeSpec, "episode-spec.sample.json"),
        (MemoryDelta, "memory-delta.sample.json"),
    ],
)
def test_the_shared_fixtures_break_no_rules(model, name):
    parse(model, fixture(name))


def broken(name: str, change) -> dict:
    data = copy.deepcopy(fixture(name))
    change(data)
    return data


def test_panels_doc_rules():
    def dupes(d):
        d["panels"][1]["id"] = "p_0001"
        d["panels"][2]["order"] = 0

    def overflow(d):
        d["panels"][0]["bbox"] = {"x": 0.5, "y": 0.1, "w": 0.6, "h": 0.2}

    assert problems(PanelsDoc.model_validate(broken("panels-doc.sample.json", dupes))) == [
        "panels.1.id: duplicate panel id p_0001",
        "panels.2.order: duplicate reading order 0",
    ]
    assert problems(PanelsDoc.model_validate(broken("panels-doc.sample.json", overflow))) == ["panels.0: bbox extends past the image"]


def test_script_doc_rules():
    def wrong_counts(d):
        d["lines"][0]["panel_ids"] = ["p_0001", "p_0002"]  # zoom takes one
        d["lines"][1]["panel_ids"] = ["p_0002"]  # split_screen needs two
        d["lines"][2]["id"] = "l_0001"

    assert problems(ScriptDoc.model_validate(broken("script-doc.sample.json", wrong_counts))) == [
        "lines.0.panel_ids: zoom takes exactly 1 panel",
        "lines.1.panel_ids: split_screen needs 2-3 panels",
        "lines.2.id: duplicate line id l_0001",
    ]


def test_audio_manifest_rules():
    data = broken("audio-manifest.sample.json", lambda d: d["clips"].append(dict(d["clips"][0])))
    assert problems(AudioManifest.model_validate(data)) == ["clips.2.line_id: duplicate clip for l_0001"]


def test_episode_spec_rules():
    def wreck(d):
        d["beats"][0]["panel_ids"] = ["p_0009"]
        d["beats"][0]["duration"] = 1.0  # shorter than its 3.4 s clip
        d["beats"][1]["panel_ids"] = ["p_0002"]
        d["beats"][2]["duration"] = 1.0  # no audio, so only the transition rule applies
        d["beats"][2]["transition_in"] = {"name": "wipe", "duration": 1.2}
        d["social_promo"]["start_at"] = 0.9
        d["social_promo"]["end_at"] = 0.1

    assert problems(EpisodeSpec.model_validate(broken("episode-spec.sample.json", wreck))) == [
        "beats.0.panel_ids: unknown panel p_0009",
        "beats.0.duration: beat is shorter than its narration audio",
        "beats.1.panel_ids: wrong panel count for split_screen",
        "beats.2.transition_in: transition is longer than the beats it joins",
        "social_promo: start_at must come before end_at",
    ]


def test_memory_delta_rules():
    def wreck(d):
        d["ops"][1] = dict(d["ops"][0])  # same ADD twice
        d["ops"][2]["entity_type"] = "faction"
        d["ops"][2]["receipts"][0]["chapter_id"] = "ch_0004"

    assert problems(MemoryDelta.model_validate(broken("memory-delta.sample.json", wreck))) == [
        "ops.1: character_jinwoo added twice",
        "ops.2.entity_id: character_jinwoo does not match type faction",
        "ops.2.receipts.0: receipt cites future chapter ch_0004",
    ]


def test_ledger_event_part_must_fit_its_commit():
    line = json.loads((FIXTURES / "ledger.sample.jsonl").read_text(encoding="utf-8").splitlines()[0])
    assert problems(LedgerEvent.model_validate(line)) == []
    assert problems(LedgerEvent.model_validate({**line, "part": 3})) == ["part: part must be below parts"]


def test_parse_raises_with_every_problem():
    data = broken("script-doc.sample.json", lambda d: d["lines"][2].update(id="l_0001"))
    with pytest.raises(ContractError) as err:
        parse(ScriptDoc, data)
    assert err.value.problems == ["lines.2.id: duplicate line id l_0001"]
