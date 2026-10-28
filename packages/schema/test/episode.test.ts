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
    expect(spec.beats[0]?.audio).toEqual({ src: "chapters/ch_0001/audio/l_0001.wav", duration: 4.2 });
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

  it("keeps only referenced panels and applies format defaults", () => {
    const spec = buildEpisodeSpec(script, panels);
    expect(spec.panels.map((p) => p.id)).toEqual(["p_0001", "p_0002"]);
    expect(spec.video).toEqual({ width: 1920, height: 1080, fps: 30, background: "#0b0b10" });
    expect(spec.title).toBe("Pehla Darwaza");
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
