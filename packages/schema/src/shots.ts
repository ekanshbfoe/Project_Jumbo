import type { BBox, Point } from "./common";
import type { CharacterBox } from "./panels";

/** One framing the camera cuts to partway through a beat. */
export interface ShotPlan {
  /** Seconds into the beat. */
  at: number;
  /** Centre of the framing, as fractions of the panel. */
  focus: Point;
  /** How much closer than the beat's own framing. */
  zoom: number;
}

export interface ShotOptions {
  /** Seconds a framing should usually hold; recap channels move every 3-5 s. */
  every?: number;
  /** Shortest a framing may hold, in seconds. */
  shortest?: number;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * Seconds into the narration at which each sentence (then clause) ends, estimated from where its text falls in the
 * line: a narrator's speaking rate is steady enough within one line for cuts to land on the breaks.
 */
export function breakTimes(narration: string, seconds: number): { sentence: number[]; clause: number[] } {
  const total = narration.length || 1;
  const at = (re: RegExp) =>
    [...narration.matchAll(re)].map((m) => ((m.index ?? 0) + m[0].length) / total * seconds).filter((t) => t > 0.5 && t < seconds - 0.5);
  return { sentence: at(/[।?!.]+\s+/g), clause: at(/[,;:]\s+/g) };
}

/** Where to look in a panel: its characters' heads (the top of each box), largest first; else its thirds. */
export function focusPoints(panelBox: BBox, characters: readonly CharacterBox[]): Point[] {
  const heads = [...characters]
    .sort((a, b) => b.bbox.w * b.bbox.h - a.bbox.w * a.bbox.h)
    .map((c): Point => [
      clamp((c.bbox.x + c.bbox.w / 2 - panelBox.x) / panelBox.w, 0.15, 0.85),
      clamp((c.bbox.y + c.bbox.h * 0.25 - panelBox.y) / panelBox.h, 0.15, 0.85),
    ]);
  if (heads.length) return heads;
  // No one in the panel: tall panels are read top to bottom, wide ones left to right.
  return panelBox.h > panelBox.w ? [[0.5, 0.3], [0.5, 0.7]] : [[0.3, 0.5], [0.7, 0.5]];
}

/**
 * The cuts of one single-panel beat: the beat opens on its preset's own framing, then every few seconds (at a sentence
 * break where there is one near, else a clause break, else evenly) it cuts closer on a character or detail, coming back
 * wide every third cut so the viewer keeps their bearings. A beat shorter than two framings keeps its one framing.
 */
export function planShots(narration: string, spoken: number, beat: number, focus: Point[], options: ShotOptions = {}): ShotPlan[] {
  const { every = 4, shortest = 2.5 } = options;
  if (beat < 2 * shortest || focus.length === 0) return [];
  const { sentence, clause } = breakTimes(narration, spoken);
  const cuts: number[] = [];
  let last = 0;
  while (beat - last >= every + shortest * 0.5) {
    const want = last + every;
    const near = (ts: number[]) => ts.filter((t) => t >= last + shortest && t <= want + 1.5 && beat - t >= shortest);
    const pick = (ts: number[]) => ts.reduce((best, t) => (Math.abs(t - want) < Math.abs(best - want) ? t : best));
    const candidates = near(sentence).length ? near(sentence) : near(clause);
    const t = candidates.length ? pick(candidates) : want;
    if (beat - t < shortest) break;
    cuts.push(Math.round(t * 100) / 100);
    last = t;
  }
  return cuts.map((at, i) => {
    const wide = (i + 1) % 3 === 0;
    const point = focus[Math.floor((i - Math.floor((i + 1) / 3)) % focus.length)]!;
    return wide ? { at, focus: [0.5, 0.5] as Point, zoom: 1.08 } : { at, focus: point, zoom: i % 2 === 0 ? 1.45 : 1.3 };
  });
}
