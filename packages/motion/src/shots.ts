import type { Shot } from "@jumbo/schema";
import type { CSSProperties } from "react";
import { Easing, interpolate } from "remotion";

/** Seconds the camera takes to reach a new framing: a snap, not a drift. */
export const SHOT_MOVE_SECONDS = 0.2;
/** How much a framing keeps creeping in while it holds, per second, so a held shot never looks frozen. */
const CREEP_PER_SECOND = 0.006;

export interface Framing {
  zoom: number;
  /** Centre of the framing, as fractions of the panel. */
  fx: number;
  fy: number;
}

export const WIDE: Readonly<Framing> = Object.freeze({ zoom: 1, fx: 0.5, fy: 0.5 });

const toFraming = (s: Shot): Framing => ({ zoom: s.zoom, fx: s.focus[0], fy: s.focus[1] });

/** The framing at a frame of the beat: the latest shot that has begun, reached from the one before in SHOT_MOVE_SECONDS. */
export function framingAt(shots: readonly Shot[], frame: number, fps: number): Framing {
  const t = frame / fps;
  let i = -1;
  while (i + 1 < shots.length && shots[i + 1]!.at <= t) i++;
  if (i < 0) return WIDE;
  const from = i === 0 ? WIDE : toFraming(shots[i - 1]!);
  const to = toFraming(shots[i]!);
  const since = t - shots[i]!.at;
  const k = interpolate(since, [0, SHOT_MOVE_SECONDS], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.out(Easing.cubic) });
  const mix = (a: number, b: number) => a + (b - a) * k;
  return { zoom: mix(from.zoom, to.zoom) * (1 + CREEP_PER_SECOND * since), fx: mix(from.fx, to.fx), fy: mix(from.fy, to.fy) };
}

/**
 * CSS for a panel at a framing: magnified about its centre and moved so the focus comes toward the middle, but never
 * so far that the panel's edge comes inside the area it filled before (which would show the backdrop through it).
 */
export function framingStyle(f: Framing): CSSProperties {
  if (f.zoom === 1 && f.fx === 0.5 && f.fy === 0.5) return {};
  const room = (f.zoom - 1) / 2;
  const shift = (v: number) => Math.max(-room, Math.min(room, -(v - 0.5) * f.zoom)) * 100;
  return {
    transform: `translate(${shift(f.fx).toFixed(3)}%, ${shift(f.fy).toFixed(3)}%) scale(${f.zoom.toFixed(4)})`,
    transformOrigin: "50% 50%",
  };
}
