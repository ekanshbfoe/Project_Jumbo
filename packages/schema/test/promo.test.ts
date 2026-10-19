import { describe, expect, it } from "vitest";
import { SocialPromoSettings } from "../src";

describe("SocialPromoSettings", () => {
  it("defaults to Project_Jumbo on all three platforms, from 5% to 95% of the episode", () => {
    expect(SocialPromoSettings.parse({})).toEqual({
      enabled: true,
      username: "Project_Jumbo",
      headline: "Follow for more recaps",
      platforms: ["x", "youtube", "instagram"],
      position: "bottom-right",
      start_at: 0.05,
      end_at: 0.95,
    });
  });

  it("rejects handles with @ or spaces and repeated platforms", () => {
    expect(SocialPromoSettings.safeParse({ username: "@jumbo" }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ username: "project jumbo" }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ platforms: ["x", "x"] }).success).toBe(false);
  });
});

describe("SocialPromoSettings timing", () => {
  it("takes fractions of the episode and requires start before end", () => {
    expect(SocialPromoSettings.safeParse({ start_at: 0.1, end_at: 0.9 }).success).toBe(true);
    expect(SocialPromoSettings.safeParse({ start_at: 0.9, end_at: 0.1 }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ start_at: 300 }).success).toBe(false);
  });
});
