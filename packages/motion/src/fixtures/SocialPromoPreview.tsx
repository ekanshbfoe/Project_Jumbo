import { PresetSpec, type Beat, type SocialPromoSettings } from "@jumbo/schema";
import { useMemo } from "react";
import { AbsoluteFill, useVideoConfig } from "remotion";
import { BeatView } from "../components/BeatView";
import { SocialPromo } from "../components/SocialPromo";
import { panelsDoc } from "./pages";

/** Seconds the standalone preview runs: entry, a short hold with one light sweep, then exit. */
export const PROMO_PREVIEW_SECONDS = 5;

const BACKDROP_PANEL = "p_0001";

/**
 * Gallery preview of the follow card over a real fixture panel, so the frosted glass has something to
 * blur. The card fills the whole composition, so Studio's loop replays its entry and exit back to back.
 */
export function SocialPromoPreview(settings: SocialPromoSettings) {
  const { durationInFrames } = useVideoConfig();
  const panels = useMemo(() => new Map(panelsDoc().panels.map((p) => [p.id, p])), []);
  const beat: Beat = {
    line_id: "l_0001",
    panel_ids: [BACKDROP_PANEL],
    narration: "Social promo preview",
    emotion: "calm",
    preset: PresetSpec.parse({ name: "zoom", params: { to: 1.08 } }),
    transition_in: { name: "cut", duration: 0 },
    card: null,
    sfx: null,
    audio: null,
    duration: PROMO_PREVIEW_SECONDS,
  };

  return (
    <AbsoluteFill style={{ backgroundColor: "#0b0b10" }}>
      <BeatView beat={beat} panels={panels} durationInFrames={durationInFrames} hideBubbles={false} />
      <SocialPromo settings={settings} durationInFrames={durationInFrames} />
    </AbsoluteFill>
  );
}
