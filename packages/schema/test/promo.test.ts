import { describe, expect, it } from "vitest";
import { SocialPromoSettings } from "../src";

describe("SocialPromoSettings", () => {
  it("defaults to Project_Jumbo on all three platforms, 5 minutes in from each end", () => {
    expect(SocialPromoSettings.parse({})).toEqual({
      enabled: true,
      username: "Project_Jumbo",
      headline: "Follow for more recaps",
      platforms: ["x", "youtube", "instagram"],
      position: "bottom-right",
      start_after: 300,
      end_before: 300,
    });
  });

  it("rejects handles with @ or spaces and repeated platforms", () => {
    expect(SocialPromoSettings.safeParse({ username: "@jumbo" }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ username: "project jumbo" }).success).toBe(false);
    expect(SocialPromoSettings.safeParse({ platforms: ["x", "x"] }).success).toBe(false);
  });
});
