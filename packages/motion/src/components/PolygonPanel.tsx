import type { EpisodePanel } from "@jumbo/schema";
import type { CSSProperties } from "react";
import { Img } from "remotion";
import { useAssetResolver } from "../assets";
import { bubbleMask, polygonToClipPath, polygonToSvgPoints } from "../geometry";

export interface PolygonPanelProps {
  panel: EpisodePanel;
  /** Rendered size in pixels. Callers keep the bbox aspect ratio (see `fitContain`). */
  width: number;
  height: number;
  /** Mask speech bubbles out. Ignored when an inpainted `clean` image exists. */
  hideBubbles?: boolean;
  /** Colour that shows through masked bubbles. */
  bubbleFill?: string;
  border?: { color: string; width: number } | false;
  /** true for the default soft shadow, or a CSS drop-shadow() filter for a custom one. */
  shadow?: boolean | string;
  style?: CSSProperties;
}

/**
 * One manhwa panel cut to its true (possibly non-rectangular) outline with a CSS clip-path.
 * Uses the bbox crop when available, otherwise windows into the full page image.
 */
export function PolygonPanel({
  panel,
  width,
  height,
  hideBubbles = false,
  bubbleFill = "#f4f1ea",
  border = { color: "#ffffff", width: 3 },
  shadow = true,
  style,
}: PolygonPanelProps) {
  const resolve = useAssetResolver();
  const cropped = panel.clean ?? panel.crop;
  const mask = hideBubbles && !panel.clean ? bubbleMask(panel.bubbles, panel.bbox) : null;

  const image: CSSProperties = cropped
    ? { position: "absolute", inset: 0, width: "100%", height: "100%", display: "block" }
    : {
        position: "absolute",
        width: width / panel.bbox.w,
        height: height / panel.bbox.h,
        left: (-panel.bbox.x * width) / panel.bbox.w,
        top: (-panel.bbox.y * height) / panel.bbox.h,
        maxWidth: "none",
      };

  return (
    <div
      style={{
        position: "relative",
        width,
        height,
        // drop-shadow on the unclipped parent follows the clipped child's outline.
        filter: typeof shadow === "string" ? shadow : shadow ? "drop-shadow(0 18px 30px rgba(0, 0, 0, 0.55))" : undefined,
        ...style,
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          overflow: "hidden",
          clipPath: polygonToClipPath(panel.polygon, panel.bbox),
          backgroundColor: mask ? bubbleFill : undefined,
        }}
      >
        <div
          style={{
            position: "absolute",
            inset: 0,
            maskImage: mask ?? undefined,
            WebkitMaskImage: mask ?? undefined,
            maskSize: "100% 100%",
            WebkitMaskSize: "100% 100%",
          }}
        >
          <Img src={resolve(cropped ?? panel.page)} style={image} />
        </div>
      </div>
      {border ? (
        <svg
          viewBox="0 0 100 100"
          preserveAspectRatio="none"
          style={{ position: "absolute", inset: 0, width: "100%", height: "100%", overflow: "visible" }}
        >
          <polygon
            points={polygonToSvgPoints(panel.polygon, panel.bbox)}
            fill="none"
            stroke={border.color}
            strokeWidth={border.width}
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
      ) : null}
    </div>
  );
}
