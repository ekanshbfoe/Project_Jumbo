import { describe, expect, it } from "vitest";
import { AudioManifest, buildEpisodeSpec, EpisodeSpec, PanelsDoc, ScriptDoc } from "../src";

const panel = (id: string, order: number) => ({
  id,
  order,
  page: "chapters/ch_0001/pages/page_001.webp",
  page_size: { width: 1000, height: 1400 },
  bbox: { x: 0, y: 0, w: 1, h: 0.5 },
  polygon: [
    [0, 0],
    [1, 0],
    [1, 0.5],
    [0, 0.5],
  ],
});

const panels = PanelsDoc.parse({
  schema_version: 1,
  chapter_id: "ch_0001",
  layout: "page",
  panels: [panel("p_0001", 0), panel("p_0002", 1), panel("p_0003", 2)],
});

const script = ScriptDoc.parse({
  schema_version: 1,
  chapter_id: "ch_0001",
  title: "Pehla Darwaza",
  lines: [
    { id: "l_0001", panel_ids: ["p_0001"], narration: "एक दो तीन चार पाँच", emotion: "calm", preset: { name: "pan" } },
    {
      id: "l_0002",
      panel_ids: ["p_0001", "p_0002"],
      narration: "आमने सामने",
      emotion: "tense",
      preset: { name: "split_screen" },
      transition_in: { name: "fade", duration: 0.5 },
    },
  ],
});

const audio = AudioManifest.parse({
  schema_version: 1,
  chapter_id: "ch_0001",
  sample_rate: 24000,
  clips: [{ line_id: "l_0001", src: "chapters/ch_0001/audio/l_0001.wav", duration: 4.2 }],
});

describe("buildEpisodeSpec", () => {
  it("sizes voiced beats from audio and unvoiced beats from word count", () => {
    const spec = buildEpisodeSpec(script, panels, audio);
    // 4.2 s of audio + 0.35 s pause + the next beat's 0.5 s fade cut to 0.3 s, rounded up to a frame.
    expect(spec.beats[0]?.duration).toBeCloseTo(4.8667, 3);
    expect(spec.beats[1]?.transition_in.duration).toBe(0.3);
    expect(spec.beats[0]?.audio).toEqual({ src: "chapters/ch_0001/audio/l_0001.wav", duration: 4.2, from: 0 });
    // 2 words / 2.6 wps + 0.35 padding is under the 1.5 s floor.
    expect(spec.beats[1]?.duration).toBe(1.5);
    expect(spec.beats[1]?.audio).toBeNull();
  });

  it("never lets the next beat's audio start before this one's narration ends", () => {
    const spec = buildEpisodeSpec(script, panels, audio);
    const [first, second] = spec.beats;
    expect(first!.duration - second!.transition_in.duration).toBeGreaterThanOrEqual(first!.audio!.duration + 0.35);
    expect(Number.isInteger(Math.round(first!.duration * 30 * 1e6) / 1e6)).toBe(true);
  });

  it("waits longer after a sound effect, a hit or a question", () => {
    const plain = buildEpisodeSpec(script, panels, audio).beats[0]!.duration;
    const big = (line: Partial<ScriptDoc["lines"][number]>) =>
      buildEpisodeSpec(ScriptDoc.parse({ ...script, lines: [{ ...script.lines[0]!, ...line }, script.lines[1]] }), panels, audio).beats[0]!.duration;
    expect(big({ sfx: "slam" })).toBeCloseTo(plain + 0.55, 1);
    expect(big({ preset: { name: "kinetic_jitter", params: { intensity: 10, decay: 0.3 } } })).toBeCloseTo(plain + 0.55, 1);
    expect(big({ narration: "पर क्या यह सच था?" })).toBeCloseTo(plain + 0.55, 1);
    expect(big({ narration: "यह सच था।" })).toBeCloseTo(plain, 5);
  });

  it("steps a multi-panel line through its panels one at a time, playing one clip across them", () => {
    const long = ScriptDoc.parse({
      ...script,
      lines: [{ id: "l_0001", panel_ids: ["p_0001", "p_0002", "p_0003"], narration: "एक दो तीन", emotion: "calm", preset: { name: "split_screen" } }],
    });
    const clip = AudioManifest.parse({ ...audio, clips: [{ line_id: "l_0001", src: "a.wav", duration: 6 }] });
    const spec = buildEpisodeSpec(long, panels, clip);
    expect(spec.beats.map((b) => b.panel_ids)).toEqual([["p_0001"], ["p_0002"], ["p_0003"]]);
    expect(spec.beats.map((b) => b.transition_in.name)).toEqual(["cut", "fade", "fade"]);
    // Each step plays the next part of the clip: no gap, no overlap, the whole clip heard.
    let heard = 0;
    spec.beats.forEach((b, i) => {
      expect(b.audio!.from).toBeCloseTo(heard, 6);
      heard += b.audio!.duration;
      const next = spec.beats[i + 1];
      if (next) expect(b.duration - next.transition_in.duration).toBeCloseTo(b.audio!.duration, 4);
    });
    expect(heard).toBeCloseTo(6, 4);
    // On the timeline the steps take as long as the line did in one beat.
    const together = buildEpisodeSpec(long, panels, clip, { sequential: false }).beats[0]!.duration;
    const stepped = spec.beats.reduce((t, b, i) => t + b.duration - (i ? b.transition_in.duration : 0), 0);
    expect(stepped).toBeCloseTo(together, 4);
    expect(() => EpisodeSpec.parse(spec)).not.toThrow();
  });

  it("shows a text card briefly and gives the rest of the line to the drawn panels", () => {
    const withCard = PanelsDoc.parse({ ...panels, panels: panels.panels.map((p) => (p.id === "p_0002" ? { ...p, text_only: true } : p)) });
    const voiced = (lines: ScriptDoc["lines"], clips: [string, number][]) =>
      buildEpisodeSpec(
        ScriptDoc.parse({ ...script, lines }),
        withCard,
        AudioManifest.parse({ ...audio, clips: clips.map(([line_id, duration]) => ({ line_id, src: `${line_id}.wav`, duration })) }),
      );
    const heardAll = (beats: { audio: { duration: number; from: number } | null }[], total: number) => {
      let heard = 0;
      for (const b of beats) {
        expect(b.audio!.from).toBeCloseTo(heard, 6);
        heard += b.audio!.duration;
      }
      expect(heard).toBeCloseTo(total, 4);
    };
    // A line over a card and a drawing: the card is read in 3.5 s, the drawing keeps the rest.
    const mixed = voiced([{ ...script.lines[1]!, id: "l_0001", panel_ids: ["p_0002", "p_0003"], transition_in: { name: "cut", duration: 0.3 } }], [["l_0001", 10]]);
    expect(mixed.beats.map((b) => b.panel_ids[0])).toEqual(["p_0002", "p_0003"]);
    expect(mixed.beats[0]!.duration - mixed.beats[1]!.transition_in.duration).toBeCloseTo(3.5, 1);
    heardAll(mixed.beats, 10);
    // A long line over a card alone: after 3.5 s the drawn panel shown before it comes back for the rest.
    const alone = voiced(
      [script.lines[0]!, { ...script.lines[0]!, id: "l_0002", panel_ids: ["p_0002"] }],
      [["l_0001", 3], ["l_0002", 11]],
    );
    expect(alone.beats.map((b) => b.panel_ids[0])).toEqual(["p_0001", "p_0002", "p_0001"]);
    expect(alone.beats[1]!.duration - alone.beats[2]!.transition_in.duration).toBeCloseTo(3.5, 1);
    heardAll(alone.beats.slice(1), 11);
    expect(() => EpisodeSpec.parse(alone)).not.toThrow();
    // A short line keeps its card.
    expect(voiced([{ ...script.lines[0]!, panel_ids: ["p_0002"] }], [["l_0001", 3]]).beats).toHaveLength(1);
  });

  it("scrolls a tall panel the script only holds or zooms", () => {
    const tallDoc = PanelsDoc.parse({ ...panels, panels: [{ ...panel("p_0001", 0), bbox: { x: 0, y: 0, w: 0.3, h: 1 } }] });
    const one = ScriptDoc.parse({ ...script, lines: [{ ...script.lines[0]!, preset: { name: "hold" } }] });
    expect(buildEpisodeSpec(one, tallDoc).beats[0]!.preset).toEqual({ name: "pan", params: { direction: "down", distance: 0.2 } });
    // So is a sideways pan, which is what a bare {pan} becomes (the real test chapter's first panel).
    const panned = ScriptDoc.parse({ ...script, lines: [{ ...script.lines[0]!, preset: { name: "pan" } }] });
    expect(buildEpisodeSpec(panned, tallDoc).beats[0]!.preset).toEqual({ name: "pan", params: { direction: "down", distance: 0.2 } });
    // A move the script chose on purpose stays.
    const shaken = ScriptDoc.parse({ ...script, lines: [{ ...script.lines[0]!, preset: { name: "shake" } }] });
    expect(buildEpisodeSpec(shaken, tallDoc).beats[0]!.preset.name).toBe("shake");
  });

  it("keeps only referenced panels and applies format defaults", () => {
    const spec = buildEpisodeSpec(script, panels);
    expect(spec.panels.map((p) => p.id)).toEqual(["p_0001", "p_0002"]);
    expect(spec.video).toEqual({ width: 1920, height: 1080, fps: 30, background: "#0b0b10" });
    expect(spec.title).toBe("Pehla Darwaza");
  });

  it("plays a line's sound effect once, as its first panel appears", () => {
    const withSfx = ScriptDoc.parse({ ...script, lines: script.lines.map((l, i) => (i === 1 ? { ...l, sfx: "clash" } : l)) });
    // Voiced long enough that the line's two panels are shown one after the other.
    const voiced = AudioManifest.parse({ ...audio, clips: [...audio.clips, { line_id: "l_0002", src: "chapters/ch_0001/audio/l_0002.wav", duration: 6 }] });
    const spec = buildEpisodeSpec(withSfx, panels, voiced);
    expect(spec.beats.map((b) => b.sfx)).toEqual([null, "clash", null]);
  });

  it("holds the last panel after the narration when asked", () => {
    const plain = buildEpisodeSpec(script, panels, audio);
    const held = buildEpisodeSpec(script, panels, audio, { endHold: 3 });
    expect(held.beats.at(-1)!.duration).toBeCloseTo(plain.beats.at(-1)!.duration + 3, 5);
    expect(held.beats.slice(0, -1)).toEqual(plain.beats.slice(0, -1));
  });

  it("ends on a face when the last panel has no one in it", () => {
    const faces = PanelsDoc.parse({ ...panels, panels: panels.panels.map((p) => (p.id === "p_0001" ? { ...p, characters: [{ cluster: "c", bbox: { x: 0, y: 0, w: 0.5, h: 0.4 } }] } : p)) });
    const lines = [script.lines[0]!, { ...script.lines[0]!, id: "l_0002", panel_ids: ["p_0002"] }];
    const clips = AudioManifest.parse({ ...audio, clips: [...audio.clips, { line_id: "l_0002", src: "l_0002.wav", duration: 8 }] });
    const spec = buildEpisodeSpec(ScriptDoc.parse({ ...script, lines }), faces, clips, { endHold: 3 });
    expect(spec.beats.map((b) => b.panel_ids[0])).toEqual(["p_0001", "p_0002", "p_0001"]);
    expect(spec.beats.at(-1)!.audio!.from).toBeGreaterThan(0);
    expect(() => EpisodeSpec.parse(spec)).not.toThrow();
    // Without an end hold (a part with more after it), the last panel stays.
    expect(buildEpisodeSpec(ScriptDoc.parse({ ...script, lines }), faces, clips).beats.map((b) => b.panel_ids[0])).toEqual(["p_0001", "p_0002"]);
  });

  it("passes an episode part's follow-card settings", () => {
    const spec = buildEpisodeSpec(script, panels, audio, { promo: { early: false, outro_s: 0 } });
    expect(spec.social_promo).toMatchObject({ early: false, outro_s: 0, start_s: 20 });
  });
});

describe("EpisodeSpec", () => {
  const base = () => structuredClone(buildEpisodeSpec(script, panels, audio)) as Record<string, any>;

  it("rejects beats that reference unknown panels", () => {
    const spec = base();
    spec.beats[0].panel_ids = ["p_0009"];
    expect(EpisodeSpec.safeParse(spec).success).toBe(false);
  });

  it("rejects beats shorter than their audio", () => {
    const spec = base();
    spec.beats[0].duration = 3;
    expect(EpisodeSpec.safeParse(spec).error?.issues[0]?.message).toBe("beat is shorter than its narration audio");
  });

  it("rejects a transition that starts under the previous narration", () => {
    const spec = base();
    spec.beats[0].duration = 4.4;
    expect(EpisodeSpec.safeParse(spec).error?.issues[0]?.message).toBe("transition starts before the previous narration ends");
  });

  it("rejects transitions longer than the beats they join", () => {
    const spec = base();
    spec.beats[1].duration = 1.4;
    spec.beats[1].transition_in = { name: "fade", duration: 1.5 };
    expect(EpisodeSpec.safeParse(spec).error?.issues[0]?.message).toBe("transition is longer than the beats it joins");
  });
});
