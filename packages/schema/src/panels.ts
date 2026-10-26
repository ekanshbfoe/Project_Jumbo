import { z } from "zod";
import { AssetPath, BBox, BubbleId, ChapterId, ImageSize, PanelId, Polygon, SchemaVersion } from "./common";

export const BubbleKind = z.enum(["speech", "thought", "narration", "sfx"]);

export const Bubble = z.object({
  id: BubbleId,
  kind: BubbleKind,
  polygon: Polygon,
  text: z.string().optional(),
  /** Magi character cluster id of the speaker, when known. */
  speaker: z.string().nullable().optional(),
}).meta({ id: "Bubble" });

export const CharacterBox = z.object({
  /** Magi character cluster id, stable within a chapter. */
  cluster: z.string().min(1),
  bbox: BBox,
}).meta({ id: "CharacterBox" });

export const Panel = z.object({
  id: PanelId,
  /** Reading order within the chapter, starting at 0. */
  order: z.number().int().nonnegative(),
  /** Page or strip slice the panel was detected on. */
  page: AssetPath,
  page_size: ImageSize,
  bbox: BBox,
  /** Refined panel outline. Equals the bbox corners for rectangular panels. */
  polygon: Polygon,
  /** Image cropped to `bbox`. */
  crop: AssetPath.optional(),
  /** `crop` with speech bubbles inpainted away. */
  clean: AssetPath.optional(),
  bubbles: z.array(Bubble).default([]),
  characters: z.array(CharacterBox).default([]),
}).meta({ id: "Panel" });

export const PanelsDoc = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    layout: z.enum(["strip", "page"]),
    panels: z.array(Panel).min(1),
  })
  .superRefine((doc, ctx) => {
    const ids = new Set<string>();
    const orders = new Set<number>();
    doc.panels.forEach((panel, i) => {
      if (ids.has(panel.id)) ctx.addIssue({ code: "custom", path: ["panels", i, "id"], message: `duplicate panel id ${panel.id}` });
      if (orders.has(panel.order)) ctx.addIssue({ code: "custom", path: ["panels", i, "order"], message: `duplicate reading order ${panel.order}` });
      ids.add(panel.id);
      orders.add(panel.order);
    });
  });

export type BubbleKind = z.infer<typeof BubbleKind>;
export type Bubble = z.infer<typeof Bubble>;
export type CharacterBox = z.infer<typeof CharacterBox>;
export type Panel = z.infer<typeof Panel>;
export type PanelsDoc = z.infer<typeof PanelsDoc>;
