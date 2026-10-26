import { z } from "zod";

export const SocialPlatform = z.enum(["x", "youtube", "instagram"]);

export const PromoPosition = z.enum(["bottom-right", "bottom-left", "top-right", "top-left", "center"]);

/** The follow-us card shown over the middle of an episode. */
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
    /** Fraction of the episode's length at which the card starts animating in. */
    start_at: z.number().min(0).max(1).default(0.05),
    /** Fraction of the episode's length by which the card has finished animating out. */
    end_at: z.number().min(0).max(1).default(0.95),
  })
  .refine((s) => new Set(s.platforms).size === s.platforms.length, "platforms must not repeat")
  .refine((s) => s.start_at < s.end_at, "start_at must come before end_at")
  .meta({ id: "SocialPromoSettings" });

export type SocialPlatform = z.infer<typeof SocialPlatform>;
export type PromoPosition = z.infer<typeof PromoPosition>;
export type SocialPromoSettings = z.infer<typeof SocialPromoSettings>;
