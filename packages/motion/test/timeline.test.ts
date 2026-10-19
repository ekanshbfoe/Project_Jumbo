import type { EpisodeSpec } from "@jumbo/schema";
import { describe, expect, it } from "vitest";
import { episodeTimeline, overlayWindow } from "../src/timeline";

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

describe("overlayWindow", () => {
  it("spans from 5 minutes in to 5 minutes before the end", () => {
    // A 20 minute episode at 30 fps.
    expect(overlayWindow(36_000, 30, 300, 300, 2)).toEqual({ from: 9_000, durationInFrames: 18_000 });
  });

  it("is absent when the episode is too short for both offsets plus the animation", () => {
    expect(overlayWindow(18_000, 30, 300, 300, 2)).toBeNull();
    expect(overlayWindow(18_059, 30, 300, 300, 2)).toBeNull();
    expect(overlayWindow(18_060, 30, 300, 300, 2)).toEqual({ from: 9_000, durationInFrames: 60 });
  });
});
