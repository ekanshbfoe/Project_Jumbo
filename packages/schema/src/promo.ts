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
    /** Seconds from the start of the episode until the card animates in. */
    start_after: z.number().nonnegative().default(300),
    /** Seconds before the end of the episode by which the card has animated out. */
    end_before: z.number().nonnegative().default(300),
  })
  .refine((s) => new Set(s.platforms).size === s.platforms.length, "platforms must not repeat");

export type SocialPlatform = z.infer<typeof SocialPlatform>;
export type PromoPosition = z.infer<typeof PromoPosition>;
export type SocialPromoSettings = z.infer<typeof SocialPromoSettings>;
