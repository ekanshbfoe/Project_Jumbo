import { EpisodeSpec } from "@jumbo/schema";
import { Composition } from "remotion";
import { calculateEpisodeMetadata, Episode } from "./compositions/Episode";
import { galleryEpisode, sampleEpisode } from "./fixtures/episodes";

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
      <Composition
        id="Gallery"
        component={Episode}
        schema={EpisodeSpec}
        defaultProps={galleryEpisode()}
        calculateMetadata={calculateEpisodeMetadata}
      />
    </>
  );
}
