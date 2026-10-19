import { z } from "zod";
import { HexColor, Point } from "./common";

const Direction = z.enum(["left", "right", "up", "down"]);

const preset = <N extends string, P extends z.ZodObject>(name: N, description: string, params: P) =>
  // Omitted params are parsed as {}, so every field's default applies (and required fields still fail).
  z.object({ name: z.literal(name), params: params.prefault({} as z.input<P>) }).describe(description);

export const HoldPreset = preset("hold", "Static panel. Use for quiet exposition or dialogue-heavy beats.", z.object({}));

export const ZoomPreset = preset(
  "zoom",
  "Smooth push in (to > from) or pull out (to < from), optionally centred on a focus point such as a face.",
  z.object({
    from: z.number().min(0.5).max(3).default(1),
    to: z.number().min(0.5).max(3).default(1.15),
    /** Normalized point inside the panel to zoom toward. Defaults to the centre. */
    focus: Point.optional(),
  }),
);

export const PanPreset = preset(
  "pan",
  "Slow camera drift across the panel. Use for establishing shots and wide panels.",
  z.object({
    direction: Direction.default("right"),
    /** Travel as a fraction of the frame size. */
    distance: z.number().min(0).max(0.5).default(0.08),
  }),
);

export const ShakePreset = preset(
  "shake",
  "Continuous camera shake. Use for rumbling, explosions or sustained tension.",
  z.object({
    /** Peak offset in pixels at 1080p. */
    intensity: z.number().min(0).max(60).default(12),
    /** Shake oscillations per second. */
    frequency: z.number().min(1).max(30).default(12),
  }),
);

export const ActionTiltPreset = preset(
  "action_tilt",
  "Fast punch-in with a dutch tilt that settles. Use for hits, reveals and sudden action.",
  z.object({
    /** Tilt in degrees; negative tilts counter-clockwise. */
    angle: z.number().min(-20).max(20).default(6),
    /** Peak scale of the punch-in. */
    punch: z.number().min(1).max(1.5).default(1.12),
  }),
);

export const KineticJitterPreset = preset(
  "kinetic_jitter",
  "Violent jitter that decays over the beat. Use for impacts, shock and screams.",
  z.object({
    intensity: z.number().min(0).max(60).default(18),
    /** Fraction of jitter remaining after one second. */
    decay: z.number().min(0).max(1).default(0.35),
  }),
);

export const PanoramicFadePreset = preset(
  "panoramic_fade",
  "Wide pan that fades in and out. Use for scene changes, flashbacks and time skips.",
  z.object({
    direction: Direction.default("right"),
    /** Fraction of the beat spent fading in, and again fading out. */
    fade: z.number().min(0).max(0.5).default(0.2),
  }),
);

export const SplitScreenPreset = preset(
  "split_screen",
  "Two or three panels share the frame and slide in one after another. Use for confrontations and parallel reactions.",
  z.object({
    orientation: z.enum(["vertical", "horizontal"]).default("vertical"),
    /** Delay between panel entrances, in seconds. */
    stagger: z.number().min(0).max(2).default(0.25),
  }),
);

export const FloatingPanelsPreset = preset(
  "floating_panels",
  "Two or three tall panels float over the background like physical photos layered in 3D, with heavy shadows and a slow drift. Use for character introductions, line-ups and reflective moments.",
  z.object({
    /** Resting in-plane rotation of the outermost panels, in degrees. */
    tilt: z.number().min(0).max(12).default(4),
    /** Peak 3D drift (tilt and turn), in degrees. */
    drift: z.number().min(0).max(15).default(6),
    /** Seconds per drift cycle. */
    period: z.number().min(3).max(30).default(10),
  }),
);

export const HorizontalSlicesPreset = preset(
  "horizontal_slices",
  "Two or three ultra-wide, short panels stacked as strips that drift slowly in opposite directions. Use for intimate close-ups, eye contact and tense stare-downs.",
  z.object({
    /** Horizontal travel of each strip over the beat, as a fraction of the frame width. */
    drift: z.number().min(0).max(0.2).default(0.05),
    /** Direction the top strip drifts; the strips below alternate. */
    lead: z.enum(["left", "right"]).default("left"),
  }),
);

export const HudPopupPreset = preset(
  "hud_popup",
  "Game-style info panel types onto the screen over the panel. Use for system messages, stats and level-ups.",
  z.object({
    text: z.string().min(1).max(120),
    position: z.enum(["top", "center", "bottom"]).default("top"),
    accent: HexColor.default("#3ad6ff"),
  }),
);

export const PresetSpec = z.discriminatedUnion("name", [
  HoldPreset,
  ZoomPreset,
  PanPreset,
  ShakePreset,
  ActionTiltPreset,
  KineticJitterPreset,
  PanoramicFadePreset,
  SplitScreenPreset,
  FloatingPanelsPreset,
  HorizontalSlicesPreset,
  HudPopupPreset,
]);

export type PresetSpec = z.infer<typeof PresetSpec>;
export type PresetName = PresetSpec["name"];
export type PresetParams<N extends PresetName> = Extract<PresetSpec, { name: N }>["params"];

/** Presets that lay out more than one panel. Every other preset takes exactly one. */
export const MULTI_PANEL_PRESETS: ReadonlySet<PresetName> = new Set<PresetName>([
  "split_screen",
  "floating_panels",
  "horizontal_slices",
]);

/** Preset vocabulary with descriptions, used to build the Director's system prompt. */
export const PRESET_CATALOG: ReadonlyArray<{ name: PresetName; description: string }> = PresetSpec.options.map((o) => ({
  name: o.shape.name.value,
  description: o.description ?? "",
}));

export const TransitionSpec = z.object({
  name: z.enum(["cut", "fade", "slide", "wipe"]).default("cut"),
  /** Overlap with the previous beat, in seconds. Ignored for cuts. */
  duration: z.number().min(0).max(1.5).default(0.4),
});

export type TransitionSpec = z.infer<typeof TransitionSpec>;
