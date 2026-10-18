import type { CSSProperties, ReactNode } from "react";

/** Virtual camera move applied to one panel. Offsets are in output pixels. */
export interface CameraTransform {
  x: number;
  y: number;
  scale: number;
  /** Degrees, clockwise. */
  rotate: number;
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
  opacity: 1,
  originX: 50,
  originY: 50,
});

export function cameraStyle(t: CameraTransform): CSSProperties {
  return {
    transform: `translate(${t.x}px, ${t.y}px) rotate(${t.rotate}deg) scale(${t.scale})`,
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
  children: ReactNode;
}

/** Positions a panel in the frame and applies a camera transform to it. */
export function Camera({ transform, left, top, width, height, children }: CameraProps) {
  return (
    <div style={{ position: "absolute", left, top, width, height, willChange: "transform", ...cameraStyle(transform) }}>
      {children}
    </div>
  );
}
