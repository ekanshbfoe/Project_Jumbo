import { z } from "zod";

/** Bumped on any breaking change to a persisted or wire contract. */
export const SCHEMA_VERSION = 1 as const;
export const SchemaVersion = z.literal(SCHEMA_VERSION);

export const ChapterId = z.string().regex(/^ch_\d{4}$/, "expected ch_0001 style id");
export const PanelId = z.string().regex(/^p_\d{4}$/, "expected p_0001 style id");
export const BubbleId = z.string().regex(/^b_\d{4}$/, "expected b_0001 style id");
export const LineId = z.string().regex(/^l_\d{4}$/, "expected l_0001 style id");

/** Coordinate normalized to the source image: 0 = left/top edge, 1 = right/bottom edge. */
const Unit = z.number().min(0).max(1);

/** [x, y] in normalized source-image coordinates. */
export const Point = z.tuple([Unit, Unit]);

/** Closed polygon, listed in drawing order. The last point connects back to the first. */
export const Polygon = z.array(Point).min(3);

/** Axis-aligned box in normalized source-image coordinates. */
export const BBox = z
  .object({ x: Unit, y: Unit, w: Unit.positive(), h: Unit.positive() })
  .refine((b) => b.x + b.w <= 1 + 1e-6 && b.y + b.h <= 1 + 1e-6, "bbox extends past the image");

export const ImageSize = z.object({
  width: z.number().int().positive(),
  height: z.number().int().positive(),
});

/** Project-relative POSIX path, or an absolute http(s)/data URL. */
export const AssetPath = z
  .string()
  .min(1)
  .refine((p) => !p.includes("\\") && !p.startsWith("/") && !/^[a-z]:/i.test(p), "use a project-relative POSIX path");

export const HexColor = z.string().regex(/^#[0-9a-fA-F]{6}$/, "expected #rrggbb");

export type Point = z.infer<typeof Point>;
export type Polygon = z.infer<typeof Polygon>;
export type BBox = z.infer<typeof BBox>;
export type ImageSize = z.infer<typeof ImageSize>;
