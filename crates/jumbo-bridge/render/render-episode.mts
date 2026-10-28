/**
 * Renders one chunk of an EpisodeSpec with the motion library's Episode composition, on a Kaggle CPU machine.
 *
 *   tsx render-episode.mts EPISODE_JSON PUBLIC_DIR OUT_MP4 --chunk=i/n
 *
 * Asset paths in the spec are project-relative and PUBLIC_DIR holds them, served as Remotion's public dir. Chunk i
 * renders the i-th of n equal frame ranges (1-based) without audio; the join machine lays the soundtrack under the
 * joined video, so no audio seam falls on a chunk boundary.
 */
import { bundle } from "@remotion/bundler";
import { renderMedia, selectComposition } from "@remotion/renderer";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { EpisodeSpec } from "@jumbo/schema";

const [specPath, publicDir, out] = process.argv.slice(2).filter((a) => !a.startsWith("--"));
const chunkArg = process.argv.find((a) => a.startsWith("--chunk="));
if (!specPath || !publicDir || !out || !chunkArg) throw new Error("usage: render-episode.mts EPISODE_JSON PUBLIC_DIR OUT_MP4 --chunk=i/n");
const spec = EpisodeSpec.parse(JSON.parse(readFileSync(specPath, "utf8")));

const serveUrl = await bundle({ entryPoint: join(process.env.MOTION_DIR!, "src", "entry.ts"), publicDir });
const composition = await selectComposition({ serveUrl, id: "Episode", inputProps: spec });
const [i, n] = chunkArg.slice(8).split("/").map(Number) as [number, number];
const size = Math.ceil(composition.durationInFrames / n);
const frameRange: [number, number] = [(i - 1) * size, Math.min(composition.durationInFrames, i * size) - 1];
console.log(`${composition.width}x${composition.height} @ ${composition.fps} fps, ${composition.durationInFrames} frames; chunk ${i}/${n}: ${frameRange[0]}-${frameRange[1]}`);

let last = -1;
await renderMedia({
  serveUrl,
  composition,
  inputProps: spec,
  codec: "h264",
  outputLocation: out,
  concurrency: Number(process.env.RENDER_CONCURRENCY ?? 2),
  frameRange,
  muted: true,
  onProgress: ({ progress }) => {
    const p = Math.floor(progress * 10);
    if (p !== last) {
      last = p;
      console.log(`render ${p * 10}%`);
    }
  },
});
console.log(`wrote ${out}`);
