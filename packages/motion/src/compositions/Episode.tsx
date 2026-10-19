import { EpisodeSpec, type TransitionSpec } from "@jumbo/schema";
import { linearTiming, TransitionSeries, type TransitionPresentation } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { slide } from "@remotion/transitions/slide";
import { wipe } from "@remotion/transitions/wipe";
import { useMemo, type ReactElement } from "react";
import { AbsoluteFill, Sequence, useVideoConfig, type CalculateMetadataFunction } from "remotion";
import { BeatView } from "../components/BeatView";
import { PROMO_ENTRY_SECONDS, PROMO_EXIT_SECONDS, SocialPromo } from "../components/SocialPromo";
import { episodeTimeline, overlayWindow } from "../timeline";

function presentation(name: Exclude<TransitionSpec["name"], "cut">): TransitionPresentation<Record<string, unknown>> {
  switch (name) {
    case "fade":
      return fade() as TransitionPresentation<Record<string, unknown>>;
    case "slide":
      return slide({ direction: "from-right" }) as TransitionPresentation<Record<string, unknown>>;
    case "wipe":
      return wipe({ direction: "from-left" }) as TransitionPresentation<Record<string, unknown>>;
  }
}

/**
 * Renders a whole chapter from an EpisodeSpec. Props must already be parsed (defaults applied):
 * `calculateEpisodeMetadata` does this for renders, and previews should call `EpisodeSpec.parse`.
 */
export function Episode(spec: EpisodeSpec) {
  const { durationInFrames, fps } = useVideoConfig();
  const timeline = useMemo(() => episodeTimeline(spec), [spec]);
  const panels = useMemo(() => new Map(spec.panels.map((p) => [p.id, p])), [spec.panels]);

  const children: ReactElement[] = [];
  spec.beats.forEach((beat, i) => {
    const timing = timeline.beats[i]!;
    if (timing.transitionInFrames > 0 && beat.transition_in.name !== "cut") {
      children.push(
        <TransitionSeries.Transition
          key={`t-${beat.line_id}`}
          presentation={presentation(beat.transition_in.name)}
          timing={linearTiming({ durationInFrames: timing.transitionInFrames })}
        />,
      );
    }
    children.push(
      <TransitionSeries.Sequence key={beat.line_id} durationInFrames={timing.durationInFrames} name={`${beat.line_id} · ${beat.preset.name}`}>
        <BeatView
          beat={beat}
          panels={panels}
          durationInFrames={timing.durationInFrames}
          hideBubbles={spec.hide_bubbles}
        />
      </TransitionSeries.Sequence>,
    );
  });

  // Scheduled against the rendered composition's length so it scales with any runtime.
  const promo = spec.social_promo;
  const promoWindow = promo.enabled
    ? overlayWindow(
        durationInFrames,
        promo.start_at,
        promo.end_at,
        Math.ceil((PROMO_ENTRY_SECONDS + PROMO_EXIT_SECONDS) * fps),
      )
    : null;

  return (
    <AbsoluteFill style={{ backgroundColor: spec.video.background }}>
      <TransitionSeries>{children}</TransitionSeries>
      {promoWindow ? (
        <Sequence from={promoWindow.from} durationInFrames={promoWindow.durationInFrames} name="SocialPromo">
          <SocialPromo settings={promo} durationInFrames={promoWindow.durationInFrames} />
        </Sequence>
      ) : null}
    </AbsoluteFill>
  );
}

/** Validates input props, applies defaults and sizes the composition from the spec. */
export const calculateEpisodeMetadata: CalculateMetadataFunction<EpisodeSpec> = ({ props }) => {
  const spec = EpisodeSpec.parse(props);
  return {
    props: spec,
    durationInFrames: episodeTimeline(spec).durationInFrames,
    fps: spec.video.fps,
    width: spec.video.width,
    height: spec.video.height,
  };
};
