import type { BBox, Bubble, EpisodePanel, Point, Polygon } from "@jumbo/schema";

export interface Size {
  width: number;
  height: number;
}

export interface Rect extends Size {
  left: number;
  top: number;
}

const pct = (v: number) => `${Math.round(v * 100_000) / 1000}%`;

/** Maps a point in normalized page coordinates into fractions of `bbox` (0..1 inside the box). */
export function toBoxSpace([x, y]: Point, bbox: BBox): Point {
  return [(x - bbox.x) / bbox.w, (y - bbox.y) / bbox.h];
}

/**
 * CSS `clip-path` for a panel outline. Percentages are relative to the element showing the bbox crop,
 * so the mask stays correct at any rendered size.
 */
export function polygonToClipPath(polygon: Polygon, bbox: BBox): string {
  return `polygon(${polygon.map((p) => toBoxSpace(p, bbox).map(pct).join(" ")).join(", ")})`;
}

/** SVG `points` attribute in a 0..100 viewBox spanning the bbox. */
export function polygonToSvgPoints(polygon: Polygon, bbox: BBox): string {
  return polygon
    .map((p) => toBoxSpace(p, bbox).map((v) => Math.round(v * 100_000) / 1000).join(","))
    .join(" ");
}

/**
 * CSS `mask-image` that keeps the panel and cuts holes where speech bubbles are, using an even-odd path.
 * Returns null when there is nothing to cut.
 */
export function bubbleMask(bubbles: readonly Bubble[], bbox: BBox): string | null {
  if (bubbles.length === 0) return null;
  const holes = bubbles
    .map((b) => `M${b.polygon.map((p) => toBoxSpace(p, bbox).map((v) => (v * 100).toFixed(3)).join(" ")).join("L")}Z`)
    .join("");
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" preserveAspectRatio="none">` +
    `<path fill-rule="evenodd" fill="#000" d="M0 0H100V100H0Z${holes}"/></svg>`;
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
}

/** Pixel size of the panel's bbox on its source page. */
export function panelSize(panel: Pick<EpisodePanel, "bbox" | "page_size">): Size {
  return { width: panel.bbox.w * panel.page_size.width, height: panel.bbox.h * panel.page_size.height };
}

/** Largest rect with the content's aspect ratio that fits inside `frame` minus `margin` (fraction per side), centred. */
export function fitContain(content: Size, frame: Size, margin = 0): Rect {
  const availW = frame.width * (1 - 2 * margin);
  const availH = frame.height * (1 - 2 * margin);
  const scale = Math.min(availW / content.width, availH / content.height);
  const width = content.width * scale;
  const height = content.height * scale;
  return { width, height, left: (frame.width - width) / 2, top: (frame.height - height) / 2 };
}

/** Splits a frame into `count` equal slots side by side ("vertical" dividers) or stacked ("horizontal"). */
export function splitFrame(frame: Size, count: number, orientation: "vertical" | "horizontal", gap = 0): Rect[] {
  const vertical = orientation === "vertical";
  const total = vertical ? frame.width : frame.height;
  const slot = (total - gap * (count - 1)) / count;
  return Array.from({ length: count }, (_, i) => {
    const offset = i * (slot + gap);
    return vertical
      ? { left: offset, top: 0, width: slot, height: frame.height }
      : { left: 0, top: offset, width: frame.width, height: slot };
  });
}
