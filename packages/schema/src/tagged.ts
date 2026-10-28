import { Emotion, ScriptDoc } from "./script";
import { MULTI_PANEL_PRESETS, PresetSpec, type PresetName } from "./presets";

/**
 * Method 2: Gemini (Google AI Studio) answers in tagged text, not JSON, and this turns it into a ScriptDoc.
 *
 *   L3 [p_0005, p_0006] (tense) {split_screen} …Hindi narration…
 *
 * The cleanup is Director v3's Pass 3 (worker/jumbo_worker/pipeline/director/passes.py): every panel exactly once in
 * reading order, numbers as Hindi words, common English loanwords back in Latin letters, and a camera move that
 * matches the line's panel count. The MEMORY block is kept for the next chapter's prompt.
 */

export interface TaggedLine {
  n: number;
  panel_ids: string[];
  emotion: Emotion | null;
  move: string | null;
  narration: string;
}

export interface StoryMemory {
  /** STORY SO FAR: the story up to the end of this chapter. */
  story: string;
  /** CHARACTERS: one line per named character, "Name | role | he/she | visual tag". */
  characters: string[];
}

export interface TaggedScript {
  beats: string;
  lines: TaggedLine[];
  memory: StoryMemory;
}

const LINE_RE = /^\s*L(\d+)\s*\[([^\]]*)\]\s*(?:\(([^)]*)\))?\s*(?:\{([^}]*)\})?\s*(.+?)\s*$/;
const PANEL_RE = /p_\d{4}/g;
const SECTION_RE = /^\s*\**\s*(BEATS|NARRATION|MEMORY)\s*\**\s*:\s*\**\s*$/;

export function parseTagged(text: string): TaggedScript {
  const sections: Record<string, string[]> = {};
  let current = "NARRATION";
  for (const raw of text.replace(/\r\n/g, "\n").split("\n")) {
    const heading = SECTION_RE.exec(raw);
    if (heading) {
      current = heading[1]!;
      continue;
    }
    (sections[current] ??= []).push(raw);
  }
  const lines: TaggedLine[] = [];
  for (const raw of sections.NARRATION ?? []) {
    const m = LINE_RE.exec(raw);
    if (!m) continue;
    const emotion = Emotion.safeParse((m[3] ?? "").trim().toLowerCase());
    lines.push({
      n: Number(m[1]),
      panel_ids: m[2]!.match(PANEL_RE) ?? [],
      emotion: emotion.success ? emotion.data : null,
      move: m[4] ? m[4].trim().toLowerCase() : null,
      narration: m[5]!.trim(),
    });
  }
  const memory = (sections.MEMORY ?? []).map((l) => l.trim()).filter(Boolean);
  const storyAt = memory.findIndex((l) => /^STORY SO FAR\s*:/i.test(l));
  const charsAt = memory.findIndex((l) => /^CHARACTERS\s*:/i.test(l));
  const story = storyAt < 0 ? "" : [memory[storyAt]!.replace(/^STORY SO FAR\s*:\s*/i, ""), ...memory.slice(storyAt + 1, charsAt > storyAt ? charsAt : undefined)].join(" ").trim();
  const characters = charsAt < 0 ? [] : [memory[charsAt]!.replace(/^CHARACTERS\s*:\s*/i, ""), ...memory.slice(charsAt + 1)].map((l) => l.replace(/^[-*]\s*/, "").trim()).filter(Boolean);
  return { beats: (sections.BEATS ?? []).join("\n").trim(), lines, memory: { story, characters } };
}

/** Every panel on screen exactly once, in reading order, at most three per line. */
export function fixPanels(lines: TaggedLine[], order: string[]): { lines: TaggedLine[]; repairs: string[] } {
  const pos = new Map(order.map((id, i) => [id, i]));
  const repairs: string[] = [];
  const seen = new Set<string>();
  for (const line of lines) {
    let ids: string[] = [];
    for (const id of line.panel_ids) {
      if (!pos.has(id)) repairs.push(`L${line.n}: unknown panel ${id} dropped`);
      else if (seen.has(id)) repairs.push(`L${line.n}: ${id} already shown, dropped`);
      else {
        ids.push(id);
        seen.add(id);
      }
    }
    ids.sort((a, b) => pos.get(a)! - pos.get(b)!);
    if (ids.length > 3) {
      repairs.push(`L${line.n}: ${ids.length} panels, kept the first 3`);
      ids.slice(3).forEach((id) => seen.delete(id));
      ids = ids.slice(0, 3);
    }
    line.panel_ids = ids;
  }
  const kept = lines.filter((l) => l.panel_ids.length > 0 || (repairs.push(`L${l.n}: no valid panel, line dropped`), false));
  for (const id of order) {
    if (seen.has(id)) continue;
    // A missing panel joins the line holding its next or previous neighbour, if that line has room.
    const host = kept.find((l) => l.panel_ids.length < 3 && l.panel_ids.some((q) => Math.abs(pos.get(q)! - pos.get(id)!) === 1));
    if (host) {
      host.panel_ids = [...host.panel_ids, id].sort((a, b) => pos.get(a)! - pos.get(b)!);
      seen.add(id);
      repairs.push(`${id} was not narrated: joined L${host.n}`);
    } else {
      repairs.push(`${id} was not narrated and has no neighbour line with room`);
    }
  }
  kept.sort((a, b) => pos.get(a.panel_ids[0]!)! - pos.get(b.panel_ids[0]!)!);
  return { lines: kept, repairs };
}

const ONES = (
  "शून्य एक दो तीन चार पाँच छह सात आठ नौ दस ग्यारह बारह तेरह चौदह पंद्रह सोलह सत्रह अठारह उन्नीस बीस इक्कीस बाईस तेईस चौबीस " +
  "पच्चीस छब्बीस सत्ताईस अट्ठाईस उनतीस तीस इकतीस बत्तीस तैंतीस चौंतीस पैंतीस छत्तीस सैंतीस अड़तीस उनतालीस चालीस इकतालीस बयालीस " +
  "तैंतालीस चवालीस पैंतालीस छियालीस सैंतालीस अड़तालीस उनचास पचास इक्यावन बावन तिरेपन चौवन पचपन छप्पन सत्तावन अट्ठावन उनसठ साठ " +
  "इकसठ बासठ तिरेसठ चौंसठ पैंसठ छियासठ सड़सठ अड़सठ उनहत्तर सत्तर इकहत्तर बहत्तर तिहत्तर चौहत्तर पचहत्तर छिहत्तर सतहत्तर अठहत्तर " +
  "उन्यासी अस्सी इक्यासी बयासी तिरासी चौरासी पचासी छियासी सत्तासी अट्ठासी नवासी नब्बे इक्यानवे बानवे तिरानवे चौरानवे पचानवे छियानवे " +
  "सत्तानवे अट्ठानवे निन्यानवे"
).split(" ");

export function hindiNumber(n: number): string {
  if (n < 100) return ONES[n]!;
  for (const [size, word] of [
    [1e7, "करोड़"],
    [1e5, "लाख"],
    [1000, "हज़ार"],
    [100, "सौ"],
  ] as const) {
    if (n >= size) {
      const head = Math.floor(n / size);
      const rest = n % size;
      return `${hindiNumber(head)} ${word}${rest ? ` ${hindiNumber(rest)}` : ""}`;
    }
  }
  return String(n);
}

export function numbersToWords(text: string): string {
  return text
    .replace(/\b(\d{1,2}):(\d{2})\b(\s*बजे)?/g, (_, h: string, m: string) => `${hindiNumber(Number(h))} बजकर ${hindiNumber(Number(m))} मिनट`)
    .replace(/(\d+)%/g, (_, n: string) => `${hindiNumber(Number(n))} प्रतिशत`)
    .replace(/\d+/g, (d) => (d.length <= 9 ? hindiNumber(Number(d)) : d));
}

/** English words the model tends to write in Devanagari; the voice reads them better in Latin letters. */
const LOANWORDS: Record<string, string> = {
  ऑफिस: "office", ऑफ़िस: "office", बॉस: "boss", टीम: "team", सिस्टम: "system", लेवल: "level", स्किल: "skill",
  ओवरटाइम: "overtime", कंपनी: "company", मैनेजर: "manager", डिपार्टमेंट: "department", प्रोजेक्ट: "project",
  मीटिंग: "meeting", बीयर: "beer", अकाउंटिंग: "accounting", एम्प्लॉई: "employee", एम्प्लॉयी: "employee",
  रग्बी: "rugby", कॉलेज: "college", क्लब: "club", पार्टी: "party", प्रोडक्शन: "production", टीवी: "TV",
  फोन: "phone", फ़ोन: "phone", ट्रेन: "train", स्टेशन: "station", कंप्यूटर: "computer", डेस्क: "desk",
  टाई: "tie", सूट: "suit", ज़ॉम्बी: "zombie", जॉम्बी: "zombie", ज़ॉम्बीज़: "zombies", जॉम्बीज: "zombies",
  स्टाइल: "style", प्लान: "plan", बॉडी: "body", पावर: "power", फाइट: "fight", फ़ाइट: "fight",
  इंटरव्यू: "interview", रिपोर्टर: "reporter", बैंक: "bank",
};

export function loanwordsToLatin(text: string): string {
  return text.replace(/[ऀ-ॿ]+/g, (w) => LOANWORDS[w] ?? w);
}

const PRESET_NAMES = new Set<string>(PresetSpec.options.map((o) => o.shape.name.value));

/** The line's camera move, made to fit its panel count (the strict Pass 2 schema did this in Method 1). */
function presetFor(move: string | null, panels: number, note: (msg: string) => void): PresetSpec {
  const multi = panels > 1;
  let name = (move ?? "") as PresetName;
  if (!PRESET_NAMES.has(name)) {
    if (move) note(`unknown camera move {${move}}`);
    name = multi ? "split_screen" : "hold";
  } else if (multi !== MULTI_PANEL_PRESETS.has(name)) {
    note(`{${name}} does not fit ${panels} panel${panels === 1 ? "" : "s"}`);
    name = multi ? "split_screen" : "zoom";
  } else if (name === "hud_popup") {
    // A popup needs its text, which the tagged format does not carry.
    name = "hold";
  }
  return PresetSpec.parse({ name, params: {} });
}

export interface TaggedResult {
  doc: ScriptDoc;
  memory: StoryMemory;
  beats: string;
  /** Repairs and notes, for the review screen. */
  notes: string[];
}

/** Gemini's tagged answer -> a validated ScriptDoc for `chapter`, whose panels are `order` in reading order. */
export function taggedToScript(text: string, chapter: string, order: string[]): TaggedResult {
  const parsed = parseTagged(text);
  if (parsed.lines.length === 0) throw new Error("No narration lines found. Each line must look like: L1 [p_0001] (calm) {hold} …");
  const { lines, repairs } = fixPanels(parsed.lines, order);
  const notes = [...repairs];
  const doc = ScriptDoc.parse({
    schema_version: 1,
    chapter_id: chapter,
    lines: lines.map((l, i) => ({
      id: `l_${String(i + 1).padStart(4, "0")}`,
      panel_ids: l.panel_ids,
      narration: loanwordsToLatin(numbersToWords(l.narration)),
      emotion: l.emotion ?? "calm",
      voice: { speaker: "narrator", tone: "neutral", pace: "normal", intensity: 0.5 },
      preset: presetFor(l.move, l.panel_ids.length, (msg) => notes.push(`L${l.n}: ${msg}`)),
      transition_in: { name: "cut", duration: 0.3 },
    })),
  });
  return { doc, memory: parsed.memory, beats: parsed.beats, notes };
}
