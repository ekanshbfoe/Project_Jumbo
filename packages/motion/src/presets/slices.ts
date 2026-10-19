import type { PresetParams } from "@jumbo/schema";
import { Easing, interpolate, spring } from "remotion";
import { IDENTITY_CAMERA, type CameraTransform } from "../components/Camera";
import type { Size } from "../geometry";
import type { PanelSlot, PresetContext } from "./types";

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

/** Stacks 2-3 wide strips down the frame with even gaps, each centred and as wide as it can be. */
export function slicesLayout(sizes: Size[], frame: Size): PanelSlot[] {
  const n = sizes.length;
  const gap = frame.height * 0.03;
  const slotHeight = (frame.height * 0.84 - gap * (n - 1)) / n;
  const maxWidth = frame.width * 0.86;
  return sizes.map((size, i) => {
    const scale = Math.min(maxWidth / size.width, slotHeight / size.height);
    const width = size.width * scale;
    const height = size.height * scale;
    return {
      left: (frame.width - width) / 2,
      top: frame.height * 0.08 + i * (slotHeight + gap) + (slotHeight - height) / 2,
      width,
      height,
    };
  });
}

/** +1 when this strip drifts left, -1 when it drifts right. Strips alternate from the lead direction. */
export function sliceDirection(index: number, lead: "left" | "right"): 1 | -1 {
  const leadSign = lead === "left" ? 1 : -1;
  return (index % 2 === 0 ? leadSign : -leadSign) as 1 | -1;
}

/** Slow, weighted horizontal drift in alternating directions, after a staggered fade-in. */
export function horizontalSlices(ctx: PresetContext, p: PresetParams<"horizontal_slices">): CameraTransform {
  const t = ctx.durationInFrames <= 1 ? 1 : ctx.frame / (ctx.durationInFrames - 1);
  const eased = Easing.bezier(0.45, 0, 0.55, 1)(t);
  const travel = p.drift * ctx.frameSize.width;
  const enter = spring({
    frame: Math.max(0, ctx.frame - ctx.index * Math.round(0.1 * ctx.fps)),
    fps: ctx.fps,
    config: { damping: 20, stiffness: 110, mass: 1 },
  });
  return {
    ...IDENTITY_CAMERA,
    x: sliceDirection(ctx.index, p.lead) * interpolate(eased, [0, 1], [travel / 2, -travel / 2]),
    opacity: interpolate(enter, [0, 0.6], [0, 1], clamp),
  };
}

/** Subtle shadow that separates stacked strips without lifting them off the page. */
export const slicesShadow = (u: number) => `drop-shadow(0 ${8 * u}px ${18 * u}px rgba(0, 0, 0, 0.55))`;
