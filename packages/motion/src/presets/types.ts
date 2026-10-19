import type { PresetName, PresetParams } from "@jumbo/schema";
import type { ComponentType } from "react";
import type { CameraTransform } from "../components/Camera";
import type { Rect, Size } from "../geometry";

export interface PresetContext {
  /** Frame within the beat, starting at 0. */
  frame: number;
  durationInFrames: number;
  fps: number;
  frameSize: Size;
  /** Stable per beat, so randomness differs between beats but not between renders. */
  seed: string;
  /** Which panel of a multi-panel beat this camera is for. */
  index: number;
  count: number;
}

export interface PresetOverlayProps<N extends PresetName> {
  ctx: PresetContext;
  params: PresetParams<N>;
}

/** Where one panel of a beat rests before its camera moves, and its stacking order. */
export interface PanelSlot extends Rect {
  zIndex?: number;
}

export interface PresetImpl<N extends PresetName> {
  name: N;
  /** Camera move for one panel at one frame. Must be a pure function of its inputs. */
  camera: (ctx: PresetContext, params: PresetParams<N>) => CameraTransform;
  /** Custom resting layout for the beat's panels (pixel sizes of each bbox), instead of the default fit/split. */
  layout?: (panels: Size[], frame: Size, params: PresetParams<N>) => PanelSlot[];
  /** CSS drop-shadow() filter for this preset's panels, given the 1080p pixel unit. */
  panelShadow?: (u: number) => string;
  /** Optional layer drawn above the panels, e.g. the HUD box. */
  Overlay?: ComponentType<PresetOverlayProps<N>>;
}

export type PresetRegistry = { [N in PresetName]: PresetImpl<N> };
