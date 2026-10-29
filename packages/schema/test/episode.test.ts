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
    // 4.2 s of audio + 0.12 s pause + the next beat's 0.5 s fade cut to 0.3 s, rounded up to a frame.
    expect(spec.beats[0]?.duration).toBeCloseTo(4.6333, 3);
    expect(spec.beats[1]?.transition_in.duration).toBe(0.3);
    expect(spec.beats[0]?.audio).toEqual({ src: "chapters/ch_0001/audio/l_0001.wav", duration: 4.2, from: 0 });
    // 2 words / 2.6 wps + 0.12 padding is under the 1.5 s floor.
    expect(spec.beats[1]?.duration).toBe(1.5);
    expect(spec.beats[1]?.audio).toBeNull();
  });

  it("never lets the next beat's audio start before this one's narration ends", () => {
    const spec = buildEpisodeSpec(script, panels, audio);
    const [first, second] = spec.beats;
    expect(first!.duration - second!.transition_in.duration).toBeGreaterThanOrEqual(first!.audio!.duration + 0.12);
    expect(Number.isInteger(Math.round(first!.duration * 30 * 1e6) / 1e6)).toBe(true);
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
    expect(spec.opening).toBeNull();
  });

  it("opens an episode part with its chapter title and passes the follow card's part settings", () => {
    const spec = buildEpisodeSpec(script, panels, audio, { opening: { title: "Pehla Darwaza" }, promo: { early: false, outro_s: 0 } });
    expect(spec.opening).toEqual({ title: "Pehla Darwaza", subtitle: "", seconds: 2.5 });
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
