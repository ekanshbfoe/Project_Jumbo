import type { PresetParams } from "@jumbo/schema";
import { Easing, interpolate, spring } from "remotion";
import { IDENTITY_CAMERA, type CameraTransform } from "../components/Camera";
import { noise1D } from "../noise";
import type { PresetContext } from "./types";

type Direction = "left" | "right" | "up" | "down";

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

/** 0 at the first frame of the beat, 1 at the last. */
const progress = (ctx: PresetContext) => (ctx.durationInFrames <= 1 ? 1 : ctx.frame / (ctx.durationInFrames - 1));

/** Pixel unit scaled so intensities authored at 1080p look the same at any resolution. */
const unit = (ctx: PresetContext) => ctx.frameSize.height / 1080;

const seconds = (ctx: PresetContext) => ctx.frame / ctx.fps;

const cam = (t: Partial<CameraTransform>): CameraTransform => ({ ...IDENTITY_CAMERA, ...t });

/** Camera travel along `direction`; the content moves the opposite way. */
function travel(ctx: PresetContext, direction: Direction, distance: number, t: number) {
  const span = direction === "left" || direction === "right" ? ctx.frameSize.width : ctx.frameSize.height;
  const offset = interpolate(t, [0, 1], [distance / 2, -distance / 2]) * span;
  switch (direction) {
    case "right":
      return { x: offset, y: 0 };
    case "left":
      return { x: -offset, y: 0 };
    case "down":
      return { x: 0, y: offset };
    case "up":
      return { x: 0, y: -offset };
  }
}

function shakeOffset(ctx: PresetContext, amplitude: number, frequency: number) {
  const t = seconds(ctx) * frequency;
  return {
    x: amplitude * noise1D(`${ctx.seed}:x`, t),
    y: amplitude * noise1D(`${ctx.seed}:y`, t + 17.3),
    rotate: (amplitude / 30) * noise1D(`${ctx.seed}:r`, t + 41.7),
  };
}

export const hold = (): CameraTransform => cam({});

export function zoom(ctx: PresetContext, p: PresetParams<"zoom">): CameraTransform {
  const [fx, fy] = p.focus ?? [0.5, 0.5];
  return cam({
    scale: interpolate(progress(ctx), [0, 1], [p.from, p.to], { easing: Easing.inOut(Easing.cubic) }),
    originX: fx * 100,
    originY: fy * 100,
  });
}

export function pan(ctx: PresetContext, p: PresetParams<"pan">): CameraTransform {
  const eased = Easing.inOut(Easing.sin)(progress(ctx));
  return cam({ ...travel(ctx, p.direction, p.distance, eased), scale: 1 + p.distance / 2 });
}

export function shake(ctx: PresetContext, p: PresetParams<"shake">): CameraTransform {
  return cam({ ...shakeOffset(ctx, p.intensity * unit(ctx), p.frequency), scale: 1.03 });
}

export function actionTilt(ctx: PresetContext, p: PresetParams<"action_tilt">): CameraTransform {
  const hit = spring({ frame: ctx.frame, fps: ctx.fps, config: { damping: 11, stiffness: 190, mass: 0.6 } });
  return cam({
    scale: 1 + (p.punch - 1) * hit + 0.02 * progress(ctx),
    rotate: p.angle * hit,
  });
}

export function kineticJitter(ctx: PresetContext, p: PresetParams<"kinetic_jitter">): CameraTransform {
  // decay is the fraction left after one second; 0 kills the jitter after the first frame.
  const envelope = p.decay === 0 ? (ctx.frame === 0 ? 1 : 0) : Math.pow(p.decay, seconds(ctx));
  const jitter = shakeOffset(ctx, p.intensity * unit(ctx) * envelope, 18);
  return cam({ ...jitter, rotate: jitter.rotate * 2, scale: 1.02 + 0.06 * envelope });
}

export function panoramicFade(ctx: PresetContext, p: PresetParams<"panoramic_fade">): CameraTransform {
  const t = progress(ctx);
  const opacity = p.fade > 0 ? interpolate(t, [0, p.fade, 1 - p.fade, 1], [0, 1, 1, 0], clamp) : 1;
  return cam({ ...travel(ctx, p.direction, 0.12, t), scale: 1.08, opacity });
}

export function splitScreen(ctx: PresetContext, p: PresetParams<"split_screen">): CameraTransform {
  const local = Math.max(0, ctx.frame - Math.round(ctx.index * p.stagger * ctx.fps));
  const enter = spring({ frame: local, fps: ctx.fps, config: { damping: 15, stiffness: 120 } });
  const slide = (1 - enter) * 0.5;
  const offset =
    p.orientation === "vertical"
      ? { x: 0, y: (ctx.index % 2 === 0 ? 1 : -1) * slide * ctx.frameSize.height }
      : { x: (ctx.index % 2 === 0 ? -1 : 1) * slide * ctx.frameSize.width, y: 0 };
  return cam({ ...offset, opacity: interpolate(enter, [0, 0.4], [0, 1], clamp), scale: 1 + 0.03 * progress(ctx) });
}

export function hudPopup(ctx: PresetContext): CameraTransform {
  return cam({ scale: interpolate(progress(ctx), [0, 1], [1, 1.06]) });
}
