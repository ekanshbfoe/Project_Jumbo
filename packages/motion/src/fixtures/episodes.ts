import {
  AudioManifest,
  buildEpisodeSpec,
  EpisodeSpec,
  PRESET_CATALOG,
  ScriptDoc,
  type PresetName,
  type TransitionSpec,
} from "@jumbo/schema";
import { panelsDoc } from "./pages";

/** One representative panel per outline type the renderer has to handle. */
export const GALLERY_SHAPES = [
  { label: "rectangle", panel: "p_0001" },
  { label: "trapezoid", panel: "p_0004" },
  { label: "irregular + bubbles", panel: "p_0006" },
  { label: "borderless strip", panel: "p_0007" },
] as const;

const SPLIT_PAIRS = [
  ["p_0002", "p_0003"],
  ["p_0005", "p_0006"],
  ["p_0007", "p_0008"],
  ["p_0002", "p_0003", "p_0004"],
] as const;

const TRANSITIONS: TransitionSpec["name"][] = ["cut", "fade", "slide", "wipe"];

const presetParams = (name: PresetName, i: number): Record<string, unknown> => {
  switch (name) {
    case "hud_popup":
      return { text: "Daily quest complete", position: (["top", "center", "bottom"] as const)[i % 3] };
    case "zoom":
      return { to: 1.3, focus: [0.5, 0.35] };
    case "pan":
      return { direction: (["right", "left", "down", "up"] as const)[i % 4] };
    case "split_screen":
      return { orientation: i === 1 ? "horizontal" : "vertical" };
    default:
      return {};
  }
};

/** Every preset on every panel shape, cycling through transitions. Labels are shown as captions. */
export function galleryEpisode(): EpisodeSpec {
  let n = 0;
  const lines = PRESET_CATALOG.flatMap(({ name }) =>
    GALLERY_SHAPES.map((shape, i) => {
      n += 1;
      const multi = name === "split_screen";
      return {
        id: `l_${String(n).padStart(4, "0")}`,
        panel_ids: multi ? [...SPLIT_PAIRS[i]!] : [shape.panel],
        narration: `${name} · ${multi ? `${SPLIT_PAIRS[i]!.length} panels` : shape.label}`,
        emotion: "calm" as const,
        preset: { name, params: presetParams(name, i) },
        transition_in: { name: TRANSITIONS[n % TRANSITIONS.length]!, duration: 0.3 },
      };
    }),
  );
  const script = ScriptDoc.parse({ schema_version: 1, chapter_id: "ch_0001", title: "Preset gallery", lines });
  // Bubble masking is on so the bubble-heavy panels exercise it.
  const spec = buildEpisodeSpec(script, panelsDoc(), undefined, { hideBubbles: true });
  // Fixed 2.5 s beats so every cell of the gallery gets the same screen time.
  return EpisodeSpec.parse({ ...spec, beats: spec.beats.map((b) => ({ ...b, duration: 2.5 })) });
}

const SAMPLE_SCRIPT = ScriptDoc.parse({
  schema_version: 1,
  chapter_id: "ch_0001",
  title: "Pehla Darwaza",
  lines: [
    {
      id: "l_0001",
      panel_ids: ["p_0001"],
      narration: "Seoul ki raat hamesha shaant lagti thi, lekin aaj kuch alag tha.",
      emotion: "mysterious",
      preset: { name: "pan", params: { direction: "right" } },
    },
    {
      id: "l_0002",
      panel_ids: ["p_0002", "p_0003"],
      narration: "Do purane dost, aaj ek dusre ke saamne khade the.",
      emotion: "tense",
      preset: { name: "split_screen" },
      transition_in: { name: "fade", duration: 0.4 },
    },
    {
      id: "l_0003",
      panel_ids: ["p_0004"],
      narration: "Aur ek hi pal mein, talwaar ne hawa ko cheer diya!",
      emotion: "action",
      preset: { name: "action_tilt", params: { angle: -8 } },
      voice: { tone: "excited", pace: "fast", intensity: 0.9 },
    },
    {
      id: "l_0004",
      panel_ids: ["p_0007"],
      narration: "Uski aankhon mein ek neeli roshni jag uthi.",
      emotion: "shock",
      preset: { name: "kinetic_jitter" },
      transition_in: { name: "wipe", duration: 0.3 },
    },
    {
      id: "l_0005",
      panel_ids: ["p_0008"],
      narration: "System ne awaaz di. Naya gate khul chuka tha.",
      emotion: "triumphant",
      preset: { name: "hud_popup", params: { text: "New gate detected: Rank S" } },
      transition_in: { name: "slide", duration: 0.4 },
    },
  ],
});

/** Placeholder narration clips (soft tones written by scripts/make-fixtures.ts) for two lines. */
export const SAMPLE_AUDIO = AudioManifest.parse({
  schema_version: 1,
  chapter_id: "ch_0001",
  sample_rate: 24000,
  clips: [
    { line_id: "l_0001", src: "fixtures/audio/l_0001.wav", duration: 3.2 },
    { line_id: "l_0003", src: "fixtures/audio/l_0003.wav", duration: 2.4 },
  ],
});

/** A short end-to-end chapter: mixed shapes, transitions, audio and unvoiced beats. */
export function sampleEpisode(): EpisodeSpec {
  return buildEpisodeSpec(SAMPLE_SCRIPT, panelsDoc(), SAMPLE_AUDIO);
}
