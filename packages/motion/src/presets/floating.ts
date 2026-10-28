import type { PresetParams } from "@jumbo/schema";
import { interpolate, spring } from "remotion";
import { IDENTITY_CAMERA, type CameraTransform } from "../components/Camera";
import type { Size } from "../geometry";
import type { PanelSlot, PresetContext } from "./types";

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

/** -1 for the leftmost panel, 0 for the centre, 1 for the rightmost. */
const spread = (index: number, count: number) => (count > 1 ? (index - (count - 1) / 2) / ((count - 1) / 2) : 0);

/**
 * Fans 2-3 tall panels across the frame like a hand of photos: each is scaled up to most of the frame
 * height within its own slot (no overlap, so no caption is covered), alternate panels sit a little higher
 * or lower, and the centre panel is in front.
 */
export function floatingLayout(sizes: Size[], frame: Size): PanelSlot[] {
  const n = sizes.length;
  const slotWidth = (frame.width * 0.84) / n;
  // Each panel stays inside its own slot: overlapping neighbours hid their captions (Method 2 test video, 1:04).
  const maxWidth = slotWidth * 0.96;
  const maxHeight = frame.height * 0.76;
  return sizes.map((size, i) => {
    const scale = Math.min(maxWidth / size.width, maxHeight / size.height);
    const width = size.width * scale;
    const height = size.height * scale;
    const centreX = frame.width * 0.08 + slotWidth * (i + 0.5);
    const stagger = n > 1 ? (i % 2 === 0 ? -1 : 1) * frame.height * 0.025 : 0;
    return {
      left: centreX - width / 2,
      top: (frame.height - height) / 2 + stagger,
      width,
      height,
      zIndex: n - Math.round(Math.abs(spread(i, n)) * n),
    };
  });
}

/** Slow, out-of-phase 3D drift per panel, after a staggered settle-in. */
export function floatingPanels(ctx: PresetContext, p: PresetParams<"floating_panels">): CameraTransform {
  const u = ctx.frameSize.height / 1080;
  const side = spread(ctx.index, ctx.count);
  const phase = ctx.index * 2.1;
  const w = (ctx.frame / ctx.fps / p.period) * Math.PI * 2;
  const enter = spring({
    frame: Math.max(0, ctx.frame - ctx.index * Math.round(0.12 * ctx.fps)),
    fps: ctx.fps,
    config: { damping: 18, stiffness: 90, mass: 1 },
  });
  return {
    ...IDENTITY_CAMERA,
    x: Math.sin(w * 0.7 + phase) * 6 * u,
    y: (1 - enter) * 60 * u + Math.sin(w + phase) * 10 * u,
    rotate: side * p.tilt + Math.sin(w * 0.8 + phase) * 0.8,
    // Outer panels turn toward the centre, then all of them rock gently in depth.
    rotateY: -side * p.drift * 0.6 + Math.sin(w + phase) * p.drift * 0.4,
    rotateX: Math.cos(w * 0.9 + phase) * p.drift * 0.35,
    scale: (1 - Math.abs(side) * 0.04) * (0.94 + 0.06 * enter),
    opacity: interpolate(enter, [0, 0.5], [0, 1], clamp),
  };
}

/** Heavy cinematic shadow, as if the panels were prints held above the background. */
export const floatingShadow = (u: number) => `drop-shadow(0 ${20 * u}px ${50 * u}px rgba(0, 0, 0, 0.8))`;
