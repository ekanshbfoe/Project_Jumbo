import type { EpisodeSpec } from "@jumbo/schema";

export interface BeatTiming {
  /** First frame of the beat in the episode. */
  from: number;
  durationInFrames: number;
  /** Overlap with the previous beat (0 for cuts and the first beat). */
  transitionInFrames: number;
}

export interface EpisodeTimeline {
  beats: BeatTiming[];
  durationInFrames: number;
}

/** Frame layout of an episode. Transitions overlap neighbouring beats, so they shorten the total. */
export function episodeTimeline(spec: Pick<EpisodeSpec, "beats" | "video">): EpisodeTimeline {
  const { fps } = spec.video;
  const beats: BeatTiming[] = [];
  let cursor = 0;

  spec.beats.forEach((beat, i) => {
    const durationInFrames = Math.max(1, Math.round(beat.duration * fps));
    const prev = beats[i - 1];
    const wanted = i === 0 || beat.transition_in.name === "cut" ? 0 : Math.round(beat.transition_in.duration * fps);
    // Rounding can push an allowed transition to the full length of a neighbour, which Remotion rejects.
    const transitionInFrames = prev ? Math.min(wanted, prev.durationInFrames - 1, durationInFrames - 1) : 0;
    const from = cursor - transitionInFrames;
    beats.push({ from, durationInFrames, transitionInFrames });
    cursor = from + durationInFrames;
  });

  return { beats, durationInFrames: Math.max(1, cursor) };
}

export interface FrameWindow {
  from: number;
  durationInFrames: number;
}

/**
 * Frames an overlay occupies between two fractions of the episode's length, so it scales with any
 * runtime: `startAt` 0.05 and `endAt` 0.95 of a 10 minute episode is 0:30 to 9:30. Returns null when
 * the window is shorter than `minFrames`, i.e. too short to play the overlay's own animation.
 */
export function overlayWindow(
  totalFrames: number,
  startAt: number,
  endAt: number,
  minFrames: number,
): FrameWindow | null {
  const from = Math.round(totalFrames * startAt);
  const durationInFrames = Math.round(totalFrames * endAt) - from;
  return durationInFrames >= minFrames ? { from, durationInFrames } : null;
}
