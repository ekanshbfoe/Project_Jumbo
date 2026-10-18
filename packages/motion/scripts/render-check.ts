/**
 * Visual smoke test. Bundles the library, renders a still from the middle of every Gallery beat and a
 * few Episode frames into out/stills, and checks determinism by rendering sampled frames twice.
 *
 *   npm run render:check            # stills + determinism
 *   npm run render:check -- --video # also encodes out/sample-episode.mp4
 */
import { bundle } from "@remotion/bundler";
import { renderMedia, renderStill, selectComposition } from "@remotion/renderer";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { galleryEpisode, sampleEpisode } from "../src/fixtures/episodes";
import { episodeTimeline } from "../src/timeline";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "out", "stills");
mkdirSync(OUT, { recursive: true });

const sha = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

const serveUrl = await bundle({ entryPoint: join(ROOT, "src", "entry.ts"), publicDir: join(ROOT, "public") });

async function stills(id: string, inputProps: Record<string, unknown>, frames: { frame: number; name: string }[]) {
  const composition = await selectComposition({ serveUrl, id, inputProps });
  for (const { frame, name } of frames) {
    const output = join(OUT, `${id}-${name}.png`);
    await renderStill({ serveUrl, composition, inputProps, frame, output, imageFormat: "png" });
    console.log(`still ${id} frame ${frame} -> out/stills/${id}-${name}.png`);
  }
  return composition;
}

const gallery = galleryEpisode();
const galleryTimeline = episodeTimeline(gallery);
await stills(
  "Gallery",
  gallery,
  gallery.beats.map((beat, i) => {
    const t = galleryTimeline.beats[i]!;
    return { frame: t.from + Math.floor(t.durationInFrames / 2), name: `${String(i + 1).padStart(2, "0")}-${beat.preset.name}` };
  }),
);

const sample = sampleEpisode();
const sampleTimeline = episodeTimeline(sample);
const episode = await stills(
  "Episode",
  sample,
  sampleTimeline.beats.flatMap((t, i) => [
    { frame: t.from + Math.floor(t.durationInFrames * 0.6), name: `beat${i + 1}` },
    ...(t.transitionInFrames > 0 ? [{ frame: t.from + Math.floor(t.transitionInFrames / 2), name: `beat${i + 1}-transition` }] : []),
  ]),
);

// Determinism: seeded noise and pure presets must give byte-identical frames.
const composition = await selectComposition({ serveUrl, id: "Gallery", inputProps: gallery });
let mismatches = 0;
for (const frame of [5, 140, 333]) {
  const [a, b] = ["a", "b"].map((s) => join(OUT, `determinism-${frame}-${s}.png`));
  await renderStill({ serveUrl, composition, inputProps: gallery, frame, output: a!, imageFormat: "png" });
  await renderStill({ serveUrl, composition, inputProps: gallery, frame, output: b!, imageFormat: "png" });
  const same = sha(a!) === sha(b!);
  if (!same) mismatches++;
  console.log(`determinism frame ${frame}: ${same ? "identical" : "DIFFERENT"}`);
}

if (process.argv.includes("--video")) {
  const output = join(ROOT, "out", "sample-episode.mp4");
  await renderMedia({ serveUrl, composition: episode, inputProps: sample, codec: "h264", outputLocation: output });
  console.log(`video -> out/sample-episode.mp4 (${episode.durationInFrames} frames)`);
}

if (mismatches > 0) {
  console.error(`${mismatches} frame(s) were not deterministic`);
  process.exit(1);
}
