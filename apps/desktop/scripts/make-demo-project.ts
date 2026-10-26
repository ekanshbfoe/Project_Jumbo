// Creates a project the Preview screen can play, from the motion fixtures (read only):
//   ch_0001 has a built render/episode.json,
//   ch_0002 has only script + panels + audio manifest, so the screen builds the spec itself.
//
//   npx tsx apps/desktop/scripts/make-demo-project.ts <empty folder>
//
// Uses the jumbo CLI for the project and chapter files (JUMBO_CLI overrides `cargo run -p jumbo-cli`).

import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SAMPLE_AUDIO, sampleEpisode } from "../../../packages/motion/src/fixtures/episodes";
import { panelsDoc } from "../../../packages/motion/src/fixtures/pages";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const fixtures = join(repo, "packages/motion/public/fixtures");
const target = process.argv[2];
if (!target) throw new Error("usage: make-demo-project.ts <empty folder>");
const root = resolve(target);
mkdirSync(root, { recursive: true });
if (readdirSync(root).length) throw new Error(`${root} is not empty`);

const jumbo = (...args: string[]) => {
  const [cmd, ...base] = process.env.JUMBO_CLI ? [process.env.JUMBO_CLI] : ["cargo", "run", "-q", "-p", "jumbo-cli", "--"];
  execFileSync(cmd!, [...base, "-C", root, ...args], { cwd: repo, stdio: "inherit" });
};

const write = (path: string, data: unknown) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);
};

/** Moves fixture paths into the chapter folder and stamps the chapter id, as the pipeline would write them. */
const relocate = <T>(value: T, chapter: string): T =>
  JSON.parse(JSON.stringify(value), (key, v) => {
    if (key === "chapter_id") return chapter;
    if (typeof v !== "string") return v;
    if (v.startsWith("fixtures/audio/")) return `chapters/${chapter}/audio/${v.slice("fixtures/audio/".length)}`;
    if (v.startsWith("fixtures/")) return `chapters/${chapter}/pages/${v.slice("fixtures/".length)}`;
    return v;
  });

jumbo("init", "Jumbo demo");
const pdf = join(root, ".demo-source.pdf");
writeFileSync(pdf, "%PDF-1.4\n% placeholder: the demo chapters ship their panels already analysed\n");
jumbo("import", pdf, "--title", "Pehla Darwaza (built episode)");
jumbo("import", pdf, "--title", "Pehla Darwaza (script + panels)");
rmSync(pdf);

for (const chapter of ["ch_0001", "ch_0002"]) {
  const dir = join(root, "chapters", chapter);
  cpSync(join(fixtures, "audio"), join(dir, "audio"), { recursive: true });
  for (const file of readdirSync(fixtures).filter((f) => f.endsWith(".svg"))) {
    cpSync(join(fixtures, file), join(dir, "pages", file));
  }
}

const episode = sampleEpisode();
write(join(root, "chapters/ch_0001/render/episode.json"), relocate(episode, "ch_0001"));

// The approved script the sample episode was built from, recovered from its beats.
const script = {
  schema_version: 1,
  chapter_id: episode.chapter_id,
  title: episode.title,
  lines: episode.beats.map((b) => ({
    id: b.line_id,
    panel_ids: b.panel_ids,
    narration: b.narration,
    emotion: b.emotion,
    preset: b.preset,
    transition_in: b.transition_in,
  })),
};
write(join(root, "chapters/ch_0002/script/approved.json"), relocate(script, "ch_0002"));
write(join(root, "chapters/ch_0002/panels/panels.json"), relocate(panelsDoc(), "ch_0002"));
write(join(root, "chapters/ch_0002/audio/audio.manifest.json"), relocate(SAMPLE_AUDIO, "ch_0002"));

console.log(`demo project ready: ${root}`);
