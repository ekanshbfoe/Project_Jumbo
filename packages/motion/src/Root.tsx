import { EpisodeSpec, SocialPromoSettings } from "@jumbo/schema";
import { Composition, Folder } from "remotion";
import { calculateEpisodeMetadata, Episode } from "./compositions/Episode";
import { ambientPreviewEpisode, galleryEpisode, sampleEpisode } from "./fixtures/episodes";
import { PROMO_PREVIEW_SECONDS, SocialPromoPreview } from "./fixtures/SocialPromoPreview";

/**
 * Compositions for Remotion Studio and the worker's renderer. The worker renders "Episode" with the
 * chapter's EpisodeSpec as input props; the defaults here are the development fixtures.
 */
export function Root() {
  return (
    <>
      <Composition
        id="Episode"
        component={Episode}
        schema={EpisodeSpec}
        defaultProps={sampleEpisode()}
        calculateMetadata={calculateEpisodeMetadata}
      />
      <Folder name="Gallery">
        <Composition
          id="Gallery"
          component={Episode}
          schema={EpisodeSpec}
          defaultProps={galleryEpisode()}
          calculateMetadata={calculateEpisodeMetadata}
        />
        {(["dust", "smoke", "city_bokeh"] as const).map((effect) => (
          <Composition
            key={effect}
            id={`Ambient-${effect.replace("_", "-")}`}
            component={Episode}
            schema={EpisodeSpec}
            defaultProps={ambientPreviewEpisode(effect)}
            calculateMetadata={calculateEpisodeMetadata}
          />
        ))}
        <Composition
          id="SocialPromo"
          component={SocialPromoPreview}
          schema={SocialPromoSettings}
          defaultProps={SocialPromoSettings.parse({})}
          durationInFrames={PROMO_PREVIEW_SECONDS * 30}
          fps={30}
          width={1920}
          height={1080}
        />
      </Folder>
    </>
  );
}
