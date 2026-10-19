import type { AmbientEffect } from "@jumbo/schema";
import { AmbientDust } from "./dust";
import { AmbientSmoke } from "./smoke";
import type { AmbientClock } from "./types";

/** Renders the chosen ambient loop over a beat's blurred backdrop, on the episode's master clock. */
export function AmbientLayer({ effect, ...clock }: AmbientClock & { effect: AmbientEffect }) {
  switch (effect) {
    case "dust":
      return <AmbientDust {...clock} />;
    case "smoke":
      return <AmbientSmoke {...clock} />;
    case "none":
      return null;
  }
}
