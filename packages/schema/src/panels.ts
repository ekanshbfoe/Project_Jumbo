import { z } from "zod";
import { AssetPath, BBox, BubbleId, ChapterId, ImageSize, PanelId, Polygon, SchemaVersion } from "./common";

export const BubbleKind = z.enum(["speech", "thought", "narration", "sfx"]);

export const Bubble = z.object({
  id: BubbleId,
  kind: BubbleKind,
  polygon: Polygon,
  text: z.string().optional(),
  /** Cluster of the speaker (see CharacterBox.cluster), when known. */
  speaker: z.string().nullable().optional(),
}).meta({ id: "Bubble" });

/** Character box id, unique in the chapter: k_0001 is row 0 of the chapter's fingerprints. */
export const CharacterBoxId = z.string().regex(/^k_\d{4,}$/, "expected k_0001");

export const CharacterBox = z.object({
  id: CharacterBoxId.optional(),
  /**
   * Magi's grouping of the characters on one detected image (a page, slice or tile): the same
   * cluster is the same character on that image. Unique in the chapter, but never linked across
   * images; who a character is across pages comes from the fingerprints.
   */
  cluster: z.string().min(1),
  bbox: BBox,
}).meta({ id: "CharacterBox" });

/** One appearance fingerprint per character box, from Magi's crop encoder (the whole box, not just the face). */
export const Fingerprints = z.object({
  /** Little-endian float16 rows of `dim` values; row n belongs to character box k_{n+1}. */
  path: AssetPath,
  dtype: z.literal("float16"),
  dim: z.number().int().positive(),
  count: z.number().int().nonnegative(),
  /** Encoder that made them: fingerprints of different encoders cannot be compared. */
  model: z.string().min(1),
}).meta({ id: "Fingerprints" });

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
  /**
   * `crop` small, blurred and darkened: the backdrop drawn behind the panel. Made once by the vision stage so the
   * renderer does not blur the whole frame every frame; without it the renderer blurs the panel image itself.
   */
  backdrop: AssetPath.optional(),
  bubbles: z.array(Bubble).default([]),
  characters: z.array(CharacterBox).default([]),
}).meta({ id: "Panel" });

export const PanelsDoc = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    layout: z.enum(["strip", "page"]),
    panels: z.array(Panel).min(1),
    fingerprints: Fingerprints.optional(),
  })
  .superRefine((doc, ctx) => {
    const ids = new Set<string>();
    const orders = new Set<number>();
    const boxes = new Set<string>();
    doc.panels.forEach((panel, i) => {
      if (ids.has(panel.id)) ctx.addIssue({ code: "custom", path: ["panels", i, "id"], message: `duplicate panel id ${panel.id}` });
      if (orders.has(panel.order)) ctx.addIssue({ code: "custom", path: ["panels", i, "order"], message: `duplicate reading order ${panel.order}` });
      ids.add(panel.id);
      orders.add(panel.order);
      panel.characters.forEach((c, j) => {
        const path = ["panels", i, "characters", j, "id"];
        if (c.id && boxes.has(c.id)) ctx.addIssue({ code: "custom", path, message: `duplicate character box id ${c.id}` });
        if (c.id) boxes.add(c.id);
        if (doc.fingerprints && !c.id) ctx.addIssue({ code: "custom", path, message: "a chapter with fingerprints needs an id on every character box" });
        if (doc.fingerprints && c.id && Number(c.id.slice(2)) > doc.fingerprints.count) {
          ctx.addIssue({ code: "custom", path, message: `${c.id} has no fingerprint (the file has ${doc.fingerprints.count} rows)` });
        }
      });
    });
  });

export type BubbleKind = z.infer<typeof BubbleKind>;
export type Bubble = z.infer<typeof Bubble>;
export type CharacterBox = z.infer<typeof CharacterBox>;
export type Fingerprints = z.infer<typeof Fingerprints>;
export type Panel = z.infer<typeof Panel>;
export type PanelsDoc = z.infer<typeof PanelsDoc>;
