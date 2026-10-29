import { describe, expect, it } from "vitest";
import { SocialPromoSettings } from "../src";

describe("SocialPromoSettings", () => {
  it("defaults to Project_Jumbo on all three platforms, 8 s from 0:20 and a 6 s outro", () => {
    expect(SocialPromoSettings.parse({})).toEqual({
      enabled: true,
      username: "Project_Jumbo",
      headline: "Follow for more recaps",
      platforms: ["x", "youtube", "instagram"],
      position: "bottom-right",
      early: true,
      start_s: 20,
      duration_s: 8,
      outro_s: 6,
    });
  });

  it("rejects handles with @ or spaces and repeated platforms", () => {
    expect(SocialPromoSettings.safeParse({ username: "@jumbo" }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ username: "project jumbo" }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ platforms: ["x", "x"] }).success).toBe(false);
  });
});

describe("SocialPromoSettings timing", () => {
  it("is in seconds; the outro can be turned off", () => {
    expect(SocialPromoSettings.safeParse({ start_s: 45, duration_s: 10, outro_s: 0 }).success).toBe(true);
    expect(SocialPromoSettings.safeParse({ start_s: -1 }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ duration_s: 0.5 }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ outro_s: 90 }).success).toBe(false);
  });
});
