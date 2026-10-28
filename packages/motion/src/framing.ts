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

/**
 * A tall panel shown wide enough to read and scrolled from its top to its bottom over the beat ("down"), or the
 * other way ("up"), as a webtoon is read. Null when the panel is not tall enough to need it.
 */
export function scrollShot(
  source: Size,
  frame: Size,
  progress: number,
  direction: "up" | "down",
): { rect: Rect; transform: CameraTransform } | null {
  const width = Math.min(frame.width * SCROLL_WIDTH, source.width * MAX_UPSCALE);
  const height = (width * source.height) / source.width;
  const m = frame.height * SCROLL_MARGIN;
  if (height <= frame.height - 2 * m) return null;
  const t = Math.min(1, Math.max(0, (progress - SCROLL_HOLD) / (1 - 2 * SCROLL_HOLD)));
  const eased = 0.5 - Math.cos(Math.PI * (direction === "down" ? t : 1 - t)) / 2;
  const y = m + eased * (frame.height - height - 2 * m);
  return {
    rect: { left: (frame.width - width) / 2, top: 0, width, height },
    transform: { ...IDENTITY_CAMERA, y },
  };
}
