import type { EpisodePanel } from "@jumbo/schema";
import { IDENTITY_CAMERA, type CameraTransform } from "./components/Camera";
import type { Rect, Size } from "./geometry";

/** How far a panel may reach past the frame's edge, as a fraction of the frame: the panel itself is never cropped. */
export const OVERSCAN = 0.03;
/** On-screen pixels per source pixel at most: past this a scan turns soft and blocky (video test 2, 5.3x on a 424 px crop). */
export const MAX_UPSCALE = 2;

/** `limit * tanh(v / limit)`: follows `v` while it is small and eases into the limit, so a move never stops dead at it. */
function soft(v: number, limit: number): number {
  return limit <= 0 ? 0 : limit * Math.tanh(v / limit);
}

/** `rect` shrunk about its centre so the panel is never shown larger than MAX_UPSCALE times its source pixels. */
export function capUpscale(rect: Rect, source: Size, max = MAX_UPSCALE): Rect {
  const k = Math.min(1, (source.width * max) / rect.width, (source.height * max) / rect.height);
  if (k >= 1) return rect;
  const width = rect.width * k;
  const height = rect.height * k;
  return { width, height, left: rect.left + (rect.width - width) / 2, top: rect.top + (rect.height - height) / 2 };
}

/**
 * A preset's camera move on one panel, limited so the whole panel stays on screen (within OVERSCAN) and is never
 * magnified past MAX_UPSCALE source pixels. Zoom and travel are eased into their limits rather than clamped.
 */
export function keepInFrame(t: CameraTransform, rect: Rect, frame: Size, source: Size): CameraTransform {
  const mx = frame.width * OVERSCAN;
  const my = frame.height * OVERSCAN;
  const rad = (Math.abs(t.rotate) * Math.PI) / 180;
  // The rotated panel's bounding box at scale 1.
  const bw = rect.width * Math.cos(rad) + rect.height * Math.sin(rad);
  const bh = rect.width * Math.sin(rad) + rect.height * Math.cos(rad);
  const maxScale = Math.max(
    1,
    Math.min((frame.width + 2 * mx) / bw, (frame.height + 2 * my) / bh, (source.width * MAX_UPSCALE) / rect.width, (source.height * MAX_UPSCALE) / rect.height),
  );
  const scale = t.scale > 1 ? 1 + soft(t.scale - 1, maxScale - 1) : t.scale;
  // Where the scaled panel's box sits before the move, then how far the move may shift it each way.
  const ox = rect.left + (rect.width * t.originX) / 100;
  const oy = rect.top + (rect.height * t.originY) / 100;
  const cx = ox + (rect.left + rect.width / 2 - ox) * scale;
  const cy = oy + (rect.top + rect.height / 2 - oy) * scale;
  const halfW = (bw * scale) / 2;
  const halfH = (bh * scale) / 2;
  const limit = (centre: number, half: number, size: number, m: number, v: number) => {
    // Room to each side before the box passes the frame's edge plus overscan; a box already past it is pulled back in.
    const lo = -m + half - centre;
    const hi = size + m - half - centre;
    if (lo > hi) return (lo + hi) / 2;
    const mid = (lo + hi) / 2;
    return mid + soft(v - mid, (hi - lo) / 2);
  };
  return { ...t, scale, x: limit(cx, halfW, frame.width, mx, t.x), y: limit(cy, halfH, frame.height, my, t.y) };
}

/** Widest a scrolled tall panel is shown, as a share of the frame width. */
export const SCROLL_WIDTH = 0.56;
/** Margin kept above the panel's top at the start of a scroll and below its bottom at the end, as a share of the frame height. */
const SCROLL_MARGIN = 0.04;
/** Share of the beat held still at each end of a scroll, so the first and last shots can be read. */
const SCROLL_HOLD = 0.08;

/** Share of a character box, from its top, where the face is: what a scroll stops on. */
const FACE_SHARE = 0.4;
/** How long a scroll lingers on a stretch with nothing in it, against one full of faces and bubbles (1 + 1). */
const EMPTY_WEIGHT = 0.15;
/** Positions sampled along a scroll to time it. */
const SCROLL_SAMPLES = 64;

/**
 * Rows of a panel worth stopping on while it scrolls, as [top, bottom] shares of its height: every speech bubble,
 * and the top of every character box, where the face is.
 */
export function interestRows(panel: Pick<EpisodePanel, "bbox" | "bubbles" | "characters">): [number, number][] {
  const { y, h } = panel.bbox;
  const row = (top: number, bottom: number): [number, number] => [Math.max(0, (top - y) / h), Math.min(1, (bottom - y) / h)];
  const bubbles = panel.bubbles.map((b) => {
    const ys = b.polygon.map((p) => p[1]);
    return row(Math.min(...ys), Math.max(...ys));
  });
  const faces = (panel.characters ?? []).map((c) => row(c.bbox.y, c.bbox.y + c.bbox.h * FACE_SHARE));
  return [...bubbles, ...faces].filter(([a, b]) => b > a);
}

/**
 * Where a scroll is (0 at the top of the panel, 1 at the bottom) when a share `e` of its time has gone: it lingers
 * where the window shows faces and bubbles and hurries past empty sky (viewer review 2026-11-02: a head scrolled out
 * of frame for 8 s, 7 s of empty sky). With nothing marked it moves evenly.
 */
function scrollAt(e: number, window: number, interest: [number, number][]): number {
  if (interest.length === 0 || window >= 1) return e;
  const weight = (s: number) => {
    const top = s * (1 - window);
    const seen = interest.reduce((sum, [a, b]) => sum + Math.max(0, Math.min(b, top + window) - Math.max(a, top)), 0);
    return EMPTY_WEIGHT + Math.min(1, seen / window);
  };
  const cumulative = [0];
  for (let i = 1; i <= SCROLL_SAMPLES; i++) cumulative.push(cumulative[i - 1]! + (weight((i - 1) / SCROLL_SAMPLES) + weight(i / SCROLL_SAMPLES)) / 2);
  const target = e * cumulative[SCROLL_SAMPLES]!;
  const i = Math.max(1, cumulative.findIndex((c) => c >= target));
  const [lo, hi] = [cumulative[i - 1]!, cumulative[i]!];
  return (i - 1 + (hi > lo ? (target - lo) / (hi - lo) : 0)) / SCROLL_SAMPLES;
}

/**
 * A tall panel shown wide enough to read and scrolled from its top to its bottom over the beat ("down"), or the
 * other way ("up"), as a webtoon is read, lingering on the rows in `interest` (interestRows). Null when the panel is
 * not tall enough to need it.
 */
export function scrollShot(
  source: Size,
  frame: Size,
  progress: number,
  direction: "up" | "down",
  interest: [number, number][] = [],
): { rect: Rect; transform: CameraTransform } | null {
  const width = Math.min(frame.width * SCROLL_WIDTH, source.width * MAX_UPSCALE);
  const height = (width * source.height) / source.width;
  const m = frame.height * SCROLL_MARGIN;
  if (height <= frame.height - 2 * m) return null;
  const t = Math.min(1, Math.max(0, (progress - SCROLL_HOLD) / (1 - 2 * SCROLL_HOLD)));
  const eased = 0.5 - Math.cos(Math.PI * t) / 2;
  const at = scrollAt(direction === "down" ? eased : 1 - eased, (frame.height - 2 * m) / height, interest);
  const y = m + at * (frame.height - height - 2 * m);
  return {
    rect: { left: (frame.width - width) / 2, top: 0, width, height },
    transform: { ...IDENTITY_CAMERA, y },
  };
}
