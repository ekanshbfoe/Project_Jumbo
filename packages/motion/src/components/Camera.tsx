import type { CSSProperties, ReactNode } from "react";

/** Virtual camera move applied to one panel. Offsets are in output pixels. */
export interface CameraTransform {
  x: number;
  y: number;
  scale: number;
  /** Degrees, clockwise in the image plane. */
  rotate: number;
  /** Degrees of 3D tilt around the horizontal axis (top away from the viewer is positive). */
  rotateX: number;
  /** Degrees of 3D turn around the vertical axis. */
  rotateY: number;
  opacity: number;
  /** Pivot for scale and rotation, as percentages of the panel. */
  originX: number;
  originY: number;
}

export const IDENTITY_CAMERA: Readonly<CameraTransform> = Object.freeze({
  x: 0,
  y: 0,
  scale: 1,
  rotate: 0,
  rotateX: 0,
  rotateY: 0,
  opacity: 1,
  originX: 50,
  originY: 50,
});

export function cameraStyle(t: CameraTransform): CSSProperties {
  // Perspective only when the move is actually 3D, so flat presets render exactly as before.
  const depth = t.rotateX !== 0 || t.rotateY !== 0 ? `perspective(1800px) ` : "";
  const tilt = depth ? ` rotateX(${t.rotateX}deg) rotateY(${t.rotateY}deg)` : "";
  return {
    transform: `${depth}translate(${t.x}px, ${t.y}px)${tilt} rotate(${t.rotate}deg) scale(${t.scale})`,
    transformOrigin: `${t.originX}% ${t.originY}%`,
    opacity: t.opacity,
  };
}

export interface CameraProps {
  transform: CameraTransform;
  /** Where the panel sits in the frame before the camera moves. */
  left: number;
  top: number;
  width: number;
  height: number;
  /** Stacking order among the beat's panels; higher draws in front. */
  zIndex?: number;
  children: ReactNode;
}

/** Positions a panel in the frame and applies a camera transform to it. */
export function Camera({ transform, left, top, width, height, zIndex, children }: CameraProps) {
  return (
    <div
      style={{ position: "absolute", left, top, width, height, zIndex, willChange: "transform", ...cameraStyle(transform) }}
    >
      {children}
    </div>
  );
}
