"""The generated models must accept the fixtures the TypeScript and Rust sides check too."""

import json
from pathlib import Path

import pytest
from pydantic import ValidationError

from jumbo_worker.contracts import (
    MULTI_PANEL_PRESETS,
    PRESET_CATALOG,
    PRESET_NAMES,
    AudioManifest,
    EpisodeSpec,
    LedgerEvent,
    MemoryDelta,
    PanelsDoc,
    ScriptDoc,
    SessionTicket,
)

FIXTURES = Path(__file__).resolve().parents[2] / "packages" / "schema" / "fixtures"


def fixture(name: str):
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


@pytest.mark.parametrize(
    ("model", "name"),
    [
        (PanelsDoc, "panels-doc.sample.json"),
        (ScriptDoc, "script-doc.sample.json"),
        (AudioManifest, "audio-manifest.sample.json"),
        (EpisodeSpec, "episode-spec.sample.json"),
        (MemoryDelta, "memory-delta.sample.json"),
        (SessionTicket, "session-ticket.sample.json"),
    ],
)
def test_fixture_round_trips(model, name):
    data = fixture(name)
    parsed = model.model_validate(data)
    # exclude_unset keeps explicit nulls (e.g. a beat whose audio is not voiced yet).
    again = model.model_validate(parsed.model_dump(mode="json", exclude_unset=True))
    assert again == parsed


def test_every_ledger_line_is_a_ledger_event():
    lines = (FIXTURES / "ledger.sample.jsonl").read_text(encoding="utf-8").splitlines()
    events = [LedgerEvent.model_validate_json(line) for line in lines]
    assert [e.seq for e in events] == [0, 1, 2]
    # Timestamps stay exact strings: the ledger hash covers the nanosecond text Rust wrote.
    assert all(json.loads(line)["ts"] == e.ts for line, e in zip(lines, events))


def test_defaults_match_zod():
    script = ScriptDoc.model_validate(fixture("script-doc.sample.json"))
    line = script.lines[1]
    assert line.voice is None or line.voice.tone == "neutral"
    assert script.lines[2].preset.params.position == "top"
    assert script.lines[0].transition_in is None or script.lines[0].transition_in.name == "cut"


def test_rejects_what_the_json_schema_rejects():
    bad = fixture("memory-delta.sample.json")
    bad["ops"][0]["receipts"] = []
    with pytest.raises(ValidationError):
        MemoryDelta.model_validate(bad)
    with pytest.raises(ValidationError):
        SessionTicket.model_validate({**fixture("session-ticket.sample.json"), "proto": 2})


def test_catalog_matches_the_schema():
    assert len(PRESET_NAMES) == 11 and [p["name"] for p in PRESET_CATALOG] == list(PRESET_NAMES)
    assert MULTI_PANEL_PRESETS == {"split_screen", "floating_panels", "horizontal_slices"}
