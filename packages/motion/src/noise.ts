import { random } from "remotion";

const smoothstep = (t: number) => t * t * (3 - 2 * t);

/**
 * Deterministic 1-D value noise in [-1, 1]. `t` is in lattice units: one unit per oscillation.
 * Seeded through Remotion's `random`, so every render of a frame is identical.
 */
export function noise1D(seed: string, t: number): number {
  const i = Math.floor(t);
  const a = random(`${seed}:${i}`) * 2 - 1;
  const b = random(`${seed}:${i + 1}`) * 2 - 1;
  return a + (b - a) * smoothstep(t - i);
}
