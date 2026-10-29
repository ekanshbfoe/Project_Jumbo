import type { EpisodeSpec } from "@jumbo/schema";
import { describe, expect, it } from "vitest";
import { episodeTimeline, promoWindows } from "../src/timeline";

type Transition = EpisodeSpec["beats"][number]["transition_in"];

const spec = (beats: [number, Transition["name"], number][]) => ({
  video: { width: 1920, height: 1080, fps: 30, background: "#000000" },
  beats: beats.map(([duration, name, transition]) => ({ duration, transition_in: { name, duration: transition } })) as EpisodeSpec["beats"],
});

describe("episodeTimeline", () => {
  it("lays cut beats end to end", () => {
    const t = episodeTimeline(spec([[2, "cut", 0.4], [1, "cut", 0.4]]));
    expect(t.beats).toEqual([
      { from: 0, durationInFrames: 60, transitionInFrames: 0 },
      { from: 60, durationInFrames: 30, transitionInFrames: 0 },
    ]);
    expect(t.durationInFrames).toBe(90);
  });

  it("overlaps beats by their transition length", () => {
    const t = episodeTimeline(spec([[2, "fade", 0.5], [2, "fade", 0.5], [1, "slide", 0.2]]));
    expect(t.beats.map((b) => [b.from, b.transitionInFrames])).toEqual([[0, 0], [45, 15], [99, 6]]);
    expect(t.durationInFrames).toBe(129);
  });

  it("caps transitions below the length of either neighbour", () => {
    const t = episodeTimeline(spec([[0.5, "cut", 0], [1, "wipe", 0.5]]));
    expect(t.beats[1]?.transitionInFrames).toBe(14);
  });
});

describe("promoWindows", () => {
  const promo = { start_s: 20, duration_s: 8, outro_s: 6 };

  it("shows the card for 8 s from 0:20 and for the last 6 s, whatever the runtime", () => {
    // 3 and 50 minute episodes at 30 fps.
    expect(promoWindows(5_622, 30, promo, 60)).toEqual([{ from: 600, durationInFrames: 240 }, { from: 5_442, durationInFrames: 180 }]);
    expect(promoWindows(90_000, 30, promo, 60)).toEqual([{ from: 600, durationInFrames: 240 }, { from: 89_820, durationInFrames: 180 }]);
  });

  it("drops the outro when it is off, and the early card when the episode ends before it can finish", () => {
    expect(promoWindows(5_622, 30, { ...promo, outro_s: 0 }, 60)).toEqual([{ from: 600, durationInFrames: 240 }]);
    // 25 s: the early card would run into the outro (from 0:19), so only the outro is left.
    expect(promoWindows(750, 30, promo, 60)).toEqual([{ from: 570, durationInFrames: 180 }]);
    // A later part of an episode: only its outro, or nothing.
    expect(promoWindows(5_622, 30, { ...promo, early: false }, 60)).toEqual([{ from: 5_442, durationInFrames: 180 }]);
    // Too short for the card's own animation: nothing.
    expect(promoWindows(40, 30, promo, 60)).toEqual([]);
  });
});
