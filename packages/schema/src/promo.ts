import { z } from "zod";

export const SocialPlatform = z.enum(["x", "youtube", "instagram"]);

export const PromoPosition = z.enum(["bottom-right", "bottom-left", "top-right", "top-left", "center"]);

/** The follow-us card: shown once early in an episode for a few seconds, and again as an outro at its end. */
export const SocialPromoSettings = z
  .object({
    enabled: z.boolean().default(true),
    /** Handle without the @, shared across platforms. */
    username: z
      .string()
      .min(1)
      .max(30)
      .regex(/^[A-Za-z0-9_.]+$/, "letters, digits, _ and . only")
      .default("Project_Jumbo"),
    headline: z.string().min(1).max(40).default("Follow for more recaps"),
    platforms: z.array(SocialPlatform).min(1).max(3).default(["x", "youtube", "instagram"]),
    position: PromoPosition.default("bottom-right"),
    /** Whether the card shows early on. An episode made of several parts shows it only in the first. */
    early: z.boolean().default(true),
    /** Seconds into the episode at which the card starts animating in. */
    start_s: z.number().min(0).default(20),
    /** Seconds the card is on screen, its entry and exit animations included. */
    duration_s: z.number().min(1).max(60).default(8),
    /** Seconds the card is on screen again at the very end of the episode; 0 for no outro. */
    outro_s: z.number().min(0).max(60).default(6),
  })
  .refine((s) => new Set(s.platforms).size === s.platforms.length, "platforms must not repeat")
  .meta({ id: "SocialPromoSettings" });

export type SocialPlatform = z.infer<typeof SocialPlatform>;
export type PromoPosition = z.infer<typeof PromoPosition>;
export type SocialPromoSettings = z.infer<typeof SocialPromoSettings>;
