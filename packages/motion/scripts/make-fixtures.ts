/** Regenerates public/fixtures: the synthetic page SVGs and placeholder narration WAVs. */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { SAMPLE_AUDIO } from "../src/fixtures/episodes";
import { FIXTURE_PAGES, pageSvg } from "../src/fixtures/pages";

const PUBLIC = join(dirname(fileURLToPath(import.meta.url)), "..", "public");

/** 16-bit mono PCM: a quiet tone with fades, standing in for a TTS clip of the same length. */
function toneWav(seconds: number, sampleRate: number, hz: number): Buffer {
  const samples = Math.round(seconds * sampleRate);
  const data = Buffer.alloc(samples * 2);
  const fade = sampleRate * 0.05;
  for (let i = 0; i < samples; i++) {
    const envelope = Math.min(1, i / fade, (samples - i) / fade);
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * hz * i) / sampleRate) * 3000 * envelope), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

for (const page of FIXTURE_PAGES) {
  const out = join(PUBLIC, page.file);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, pageSvg(page));
  console.log(`wrote public/${page.file}`);
}

SAMPLE_AUDIO.clips.forEach((clip, i) => {
  const out = join(PUBLIC, clip.src);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, toneWav(clip.duration, SAMPLE_AUDIO.sample_rate, 220 + i * 110));
  console.log(`wrote public/${clip.src}`);
});
