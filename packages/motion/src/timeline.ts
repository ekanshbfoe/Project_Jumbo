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
