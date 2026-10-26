"""Cross-field rules of packages/schema that JSON Schema cannot express (zod `refine`/`superRefine`).

Hand-written and kept next to the generated models. The generated models check shapes, types and
ranges; these check how fields relate. Every message matches its zod counterpart.
"""

from __future__ import annotations

from typing import Any, TypeVar

from pydantic import BaseModel, RootModel

from . import (
    AudioManifest,
    EpisodeSpec,
    LedgerEvent,
    MemoryDelta,
    PanelsDoc,
    ScriptDoc,
)
from .catalog import MULTI_PANEL_PRESETS

M = TypeVar("M", bound=BaseModel)

EPS = 1e-6


class ContractError(ValueError):
    """A document is well-formed but breaks a cross-field rule."""

    def __init__(self, problems: list[str]) -> None:
        super().__init__("; ".join(problems))
        self.problems = problems


def _s(value: Any) -> Any:
    return value.root if isinstance(value, RootModel) else value


def _bbox(where: str, bbox: Any) -> list[str]:
    if bbox.x + bbox.w > 1 + EPS or bbox.y + bbox.h > 1 + EPS:
        return [f"{where}: bbox extends past the image"]
    return []


def _panel_count(where: str, preset: str, count: int) -> list[str]:
    if preset in MULTI_PANEL_PRESETS and count < 2:
        return [f"{where}: {preset} needs 2-3 panels"]
    if preset not in MULTI_PANEL_PRESETS and count != 1:
        return [f"{where}: {preset} takes exactly 1 panel"]
    return []


def _promo(where: str, promo: Any) -> list[str]:
    out = []
    if promo.platforms and len(set(promo.platforms)) != len(promo.platforms):
        out.append(f"{where}: platforms must not repeat")
    if promo.start_at is not None and promo.end_at is not None and promo.start_at >= promo.end_at:
        out.append(f"{where}: start_at must come before end_at")
    return out


def panels_doc(doc: PanelsDoc) -> list[str]:
    out: list[str] = []
    ids: set[str] = set()
    orders: set[int] = set()
    for i, panel in enumerate(doc.panels):
        if panel.id in ids:
            out.append(f"panels.{i}.id: duplicate panel id {panel.id}")
        if panel.order in orders:
            out.append(f"panels.{i}.order: duplicate reading order {panel.order}")
        ids.add(panel.id)
        orders.add(panel.order)
        out += _bbox(f"panels.{i}", panel.bbox)
        for j, character in enumerate(panel.characters or []):
            out += _bbox(f"panels.{i}.characters.{j}", character.bbox)
    return out


def script_doc(doc: ScriptDoc) -> list[str]:
    out: list[str] = []
    ids: set[str] = set()
    for i, line in enumerate(doc.lines):
        if line.id in ids:
            out.append(f"lines.{i}.id: duplicate line id {line.id}")
        ids.add(line.id)
        out += _panel_count(f"lines.{i}.panel_ids", line.preset.name, len(line.panel_ids))
    return out


def audio_manifest(doc: AudioManifest) -> list[str]:
    out: list[str] = []
    ids: set[str] = set()
    for i, clip in enumerate(doc.clips):
        if clip.line_id in ids:
            out.append(f"clips.{i}.line_id: duplicate clip for {clip.line_id}")
        ids.add(clip.line_id)
    return out


def episode_spec(spec: EpisodeSpec) -> list[str]:
    out: list[str] = []
    panel_ids = {p.id for p in spec.panels}
    for i, panel in enumerate(spec.panels):
        out += _bbox(f"panels.{i}", panel.bbox)
    for i, beat in enumerate(spec.beats):
        ids = [_s(p) for p in beat.panel_ids]
        out += [f"beats.{i}.panel_ids: unknown panel {p}" for p in ids if p not in panel_ids]
        multi = beat.preset.name in MULTI_PANEL_PRESETS
        if multi != (len(ids) > 1):
            out.append(f"beats.{i}.panel_ids: wrong panel count for {beat.preset.name}")
        if beat.audio is not None and beat.duration < beat.audio.duration:
            out.append(f"beats.{i}.duration: beat is shorter than its narration audio")
        transition = beat.transition_in
        overlap = 0 if transition is None or transition.name == "cut" else transition.duration
        if i > 0 and overlap >= min(spec.beats[i - 1].duration, beat.duration):
            out.append(f"beats.{i}.transition_in: transition is longer than the beats it joins")
        prev = spec.beats[i - 1] if i > 0 else None
        # The next beat's audio starts when its transition does, so two voices would overlap.
        if prev is not None and prev.audio is not None and prev.duration - overlap < prev.audio.duration:
            out.append(f"beats.{i}.transition_in: transition starts before the previous narration ends")
    if spec.social_promo is not None:
        out += _promo("social_promo", spec.social_promo)
    return out


def memory_delta(delta: MemoryDelta) -> list[str]:
    out: list[str] = []
    added: set[str] = set()
    for i, op in enumerate(delta.ops):
        if op.op == "NOOP":
            continue
        if op.op == "ADD":
            if op.entity_id in added:
                out.append(f"ops.{i}: {op.entity_id} added twice")
            added.add(op.entity_id)
        if not op.entity_id.startswith(f"{op.entity_type}_"):
            out.append(f"ops.{i}.entity_id: {op.entity_id} does not match type {op.entity_type}")
        for j, receipt in enumerate(op.receipts):
            # ch_NNNN ids compare correctly as strings.
            if receipt.chapter_id > delta.chapter_id:
                out.append(f"ops.{i}.receipts.{j}: receipt cites future chapter {receipt.chapter_id}")
    return out


def ledger_event(event: LedgerEvent) -> list[str]:
    return [] if event.part < event.parts else ["part: part must be below parts"]


RULES = {
    PanelsDoc: panels_doc,
    ScriptDoc: script_doc,
    AudioManifest: audio_manifest,
    EpisodeSpec: episode_spec,
    MemoryDelta: memory_delta,
    LedgerEvent: ledger_event,
}


def problems(doc: BaseModel) -> list[str]:
    """Every cross-field rule `doc` breaks; empty when it is valid."""
    rule = RULES.get(type(doc))
    return rule(doc) if rule else []


def parse(model: type[M], data: Any) -> M:
    """Validates `data` like zod would: shape first, then the cross-field rules."""
    doc = model.model_validate(data)
    found = problems(doc)
    if found:
        raise ContractError(found)
    return doc
