import type { CameraTransform } from "./components/Camera";
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
