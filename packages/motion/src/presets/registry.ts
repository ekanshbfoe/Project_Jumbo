import type { PresetName, PresetSpec } from "@jumbo/schema";
import type { CameraTransform } from "../components/Camera";
import * as cameras from "./cameras";
import { floatingLayout, floatingPanels, floatingShadow } from "./floating";
import { HudPopupOverlay } from "./HudPopupOverlay";
import type { PresetContext, PresetImpl, PresetRegistry } from "./types";

/** Every preset in the schema catalog must have an implementation here, which the mapped type enforces. */
export const PRESETS: PresetRegistry = {
  hold: { name: "hold", camera: cameras.hold },
  zoom: { name: "zoom", camera: cameras.zoom },
  pan: { name: "pan", camera: cameras.pan },
  shake: { name: "shake", camera: cameras.shake },
  action_tilt: { name: "action_tilt", camera: cameras.actionTilt },
  kinetic_jitter: { name: "kinetic_jitter", camera: cameras.kineticJitter },
  panoramic_fade: { name: "panoramic_fade", camera: cameras.panoramicFade },
  split_screen: { name: "split_screen", camera: cameras.splitScreen },
  floating_panels: {
    name: "floating_panels",
    camera: floatingPanels,
    layout: floatingLayout,
    panelShadow: floatingShadow,
  },
  hud_popup: { name: "hud_popup", camera: cameras.hudPopup, Overlay: HudPopupOverlay },
};

export function getPreset<N extends PresetName>(name: N): PresetImpl<N> {
  return PRESETS[name] as PresetImpl<N>;
}

/** Evaluates a parsed preset spec's camera at one frame. */
export function evaluateCamera(spec: PresetSpec, ctx: PresetContext): CameraTransform {
  const impl = PRESETS[spec.name] as PresetImpl<PresetName>;
  return impl.camera(ctx, spec.params as never);
}
