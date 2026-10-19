/** Inputs every ambient effect is a pure function of. */
export interface AmbientClock {
  /** Frame on the episode's master clock, so motion flows across beat cuts instead of restarting. */
  frame: number;
  fps: number;
  width: number;
  height: number;
}

/** Wraps `v` into [0, span). */
export const wrap = (v: number, span: number) => ((v % span) + span) % span;
