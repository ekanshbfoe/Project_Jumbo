import { scriptProblems } from "./hinglish";
import { Emotion, ScriptDoc, SoundEffect, type CharacterCard } from "./script";
import { MULTI_PANEL_PRESETS, PresetSpec, type PresetName } from "./presets";

/**
 * Method 2: Gemini (Google AI Studio) answers in tagged text, not JSON, and this turns it into a ScriptDoc.
 *
 *   PACK: ch_0003 7f3a
 *   L3 [p_0005, p_0006] (tense) {split_screen} …Hindi narration…
 *   L4 [p_0007] (calm) {zoom} <card: Kang Jin> …Hindi narration…   (a character's first appearance)
 *   L5 [p_0008] (action) {action_tilt} <sfx: punch> …Hindi narration…  (one of the brand sound effects)
 *
 * The answer is pasted by hand, so it is read forgivingly (Markdown, list markers, escaped brackets, wrapped lines,
 * loose panel ids) but checked strictly: an answer for another chapter or pack, a cut-off answer that leaves panels
 * unnarrated, an empty or overlong line, or a missing MEMORY block is refused with a message that says what to do.
 * The cleanup is Director v3's Pass 3 (worker/jumbo_worker/pipeline/director/passes.py): numbers as Hindi words,
 * common English loanwords back in Latin letters, and a camera move that fits the line's panel count.
 */

export interface TaggedLine {
  n: number;
  panel_ids: string[];
  emotion: Emotion | null;
  /** What Gemini wrote in the parentheses, kept to say so when it is not a known emotion. */
  emotion_text: string | null;
  move: string | null;
  /** The name in a `<card: Name>` mark: the character Gemini says first appears on this line. */
  card: string | null;
  /** What a `<sfx: name>` mark names: the sound effect played as the line starts. */
  sfx: string | null;
  narration: string;
}

export interface StoryMemory {
  /** STORY SO FAR: the story up to the end of this chapter. */
  story: string;
  /** CHARACTERS: one line per named character, "Name | role | he/she | visual tag". */
  characters: string[];
}

export interface TaggedScript {
  /** The PACK line: the chapter and pack the answer was written for. */
  pack: { chapter: string; id: string } | null;
  beats: string;
  lines: TaggedLine[];
  memory: StoryMemory;
  /** How many NARRATION blocks the text holds; more than one means two answers were pasted. */
  answers: number;
}

/** Strips what copying from AI Studio adds around a line: Markdown, list markers, escapes, full-width brackets. */
export function cleanLine(raw: string): string {
  return raw
    .replace(/\\([[\]_*(){}#.\-|])/g, "$1")
    .replace(/\*+|__|`/g, "")
    .replace(/[【［]/g, "[")
    .replace(/[】］]/g, "]")
    .replace(/（/g, "(")
    .replace(/）/g, ")")
    .replace(/｛/g, "{")
    .replace(/｝/g, "}")
    .replace(/^\s*(?:[>#•\-–]+\s*|\d+[.)]\s+)+/, "")
    .trim();
}

const LINE_RE = /^L\s*(\d+)\s*[:.\-–]?\s*\[([^\]]*)\]\s*(?:\(([^)]*)\))?\s*(?:\{([^}]*)\})?\s*(.*)$/i;
const SECTION_RE = /^(BEATS|NARRATION|MEMORY)\s*:?\s*(.*)$/i;
const PACK_RE = /^PACK\s*:?\s*(ch_\d{4})\W*(?:pack\W*)?([0-9a-f]{4})\b/i;
const STORY_RE = /^STORY SO FAR\s*:?\s*/i;
const CHARACTERS_RE = /^CHARACTERS\s*:?\s*/i;

const MARKER_RE = /\s+(?=(?:PACK|BEATS|NARRATION|MEMORY|STORY SO FAR|CHARACTERS)\s*:|L\s*\d+\s*[:.\-–]?\s*\[)/g;
const PRONOUN_RE = /^(?:he|she|they|it)(?:\s*\/\s*\w+)?$/i;
/** `<card: Name>` anywhere in a line, also with Gemini's other brackets. */
const CARD_RE = /[<‹«]\s*card\s*[:：]\s*([^>›»]+?)\s*[>›»]/i;
/** `<sfx: name>` anywhere in a line; every mark is taken out of the narration, the first one counts. */
const SFX_RE = /[<‹«]\s*sfx\s*[:：]\s*([^>›»]+?)\s*[>›»]/gi;

/**
 * The answer one item per line, however it was copied. AI Studio's Markdown view can join the whole answer into
 * hard-wrapped paragraphs ("NARRATION: L1 […] … L2 […] …"), wrapping even inside a line's panel brackets, so every
 * heading and L-tag starts its own line again.
 */
function itemLines(text: string): string[] {
  const cleaned = text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map(cleanLine)
    .join("\n")
    .replace(/\[[^\]\n]*(?:\n[^\]\n]*)+\]/g, (m) => m.replace(/\s*\n\s*/g, " "));
  return cleaned
    .replace(MARKER_RE, "\n")
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
}

const panelId = (n: number) => `p_${String(n).padStart(4, "0")}`;

/** Panel ids in a line's brackets, however Gemini wrote them: p_0007, P_7, p0007, and ranges p_0003-p_0005. */
export function panelIds(text: string): string[] {
  const out: string[] = [];
  const re = /p[\s_-]?0*(\d{1,4})(?:\s*(?:-|–|—|to)\s*p?[\s_-]?0*(\d{1,4}))?/gi;
  for (const m of text.matchAll(re)) {
    const from = Number(m[1]);
    const to = m[2] === undefined ? from : Number(m[2]);
    if (to >= from && to - from <= 10) for (let n = from; n <= to; n++) out.push(panelId(n));
    else out.push(panelId(from));
  }
  return out;
}

export function parseTagged(text: string): TaggedScript {
  const sections: Record<string, string[]> = { BEATS: [], NARRATION: [], MEMORY: [] };
  let current = "NARRATION";
  let answers = 0;
  let pack: TaggedScript["pack"] = null;
  const lines: TaggedLine[] = [];
  for (const line of itemLines(text)) {
    const packMatch = PACK_RE.exec(line);
    if (packMatch) {
      pack ??= { chapter: packMatch[1]!.toLowerCase(), id: packMatch[2]!.toLowerCase() };
      continue;
    }
    const heading = SECTION_RE.exec(line);
    if (heading) {
      current = heading[1]!.toUpperCase();
      if (current === "NARRATION") answers++;
      if (heading[2]) sections[current]!.push(heading[2]);
      continue;
    }
    const m = current === "MEMORY" ? null : LINE_RE.exec(line);
    if (m) {
      const emotion = Emotion.safeParse((m[3] ?? "").trim().toLowerCase());
      const card = CARD_RE.exec(m[5]!);
      const sfx = [...m[5]!.matchAll(SFX_RE)][0];
      lines.push({
        n: Number(m[1]),
        panel_ids: panelIds(m[2]!),
        emotion: emotion.success ? emotion.data : null,
        emotion_text: m[3]?.trim() || null,
        move: m[4] ? m[4].trim().toLowerCase() : null,
        card: card ? card[1]!.trim() : null,
        sfx: sfx ? sfx[1]!.trim() : null,
        narration: m[5]!.replace(CARD_RE, " ").replace(SFX_RE, " ").replace(/\s+/g, " ").trim(),
      });
      current = "NARRATION";
    } else if (current === "NARRATION" && lines.length) {
      // A narration wrapped onto the next line when it was copied.
      const last = lines[lines.length - 1]!;
      last.narration = `${last.narration} ${line}`.trim();
    } else {
      sections[current]!.push(line);
    }
  }
  return {
    pack,
    beats: sections.BEATS!.join("\n"),
    lines,
    memory: readMemory(sections.MEMORY!),
    answers: Math.max(answers, lines.length ? 1 : 0),
  };
}

function readMemory(lines: string[]): StoryMemory {
  const story: string[] = [];
  const characters: string[] = [];
  let into: string[] | null = null;
  for (const line of lines) {
    if (STORY_RE.test(line)) {
      into = story;
      const rest = line.replace(STORY_RE, "");
      if (rest) story.push(rest);
    } else if (CHARACTERS_RE.test(line)) {
      into = characters;
      const rest = line.replace(CHARACTERS_RE, "");
      if (rest) characters.push(rest);
    } else {
      into?.push(line);
    }
  }
  return { story: story.join(" ").trim(), characters: characterEntries(characters) };
}

/**
 * "Name | role | he/she | look" entries. Lines that already hold one entry each are kept; otherwise (entries
 * wrapped over lines or glued into one paragraph) the fields are re-cut around the pronoun field, and the
 * capitalised words that end a look start the next entry's name.
 */
function characterEntries(lines: string[]): string[] {
  if (lines.every((l) => (l.match(/\|/g) ?? []).length === 3)) return lines;
  const fields = lines.join(" ").split("|").map((f) => f.trim());
  const pronouns = fields.flatMap((f, i) => (PRONOUN_RE.test(f) ? [i] : []));
  if (pronouns.length < 2 || pronouns.some((p, k) => p !== 2 + 3 * k)) return lines;
  const entries: string[] = [];
  let name = fields[0]!;
  pronouns.forEach((p, k) => {
    const lastEntry = k === pronouns.length - 1;
    const words = lastEntry ? [] : (fields[p + 1] ?? "").split(/\s+/);
    let cut = words.length;
    while (cut > 0 && words.length - cut < 3 && /^\p{Lu}/u.test(words[cut - 1]!)) cut--;
    const look = lastEntry ? fields.slice(p + 1).join(" | ") : words.slice(0, cut).join(" ");
    entries.push([name, fields[p - 1]!, fields[p]!, look].join(" | "));
    name = words.slice(cut).join(" ");
  });
  return entries.every((e) => !e.startsWith(" |")) ? entries : lines;
}

/**
 * Every panel on screen once, at most three per line, in the order Gemini narrated them. A single skipped panel
 * between narrated ones joins its neighbour's line; any other unnarrated panel is reported as missing.
 */
export function fixPanels(lines: TaggedLine[], order: string[]): { lines: TaggedLine[]; repairs: string[]; missing: string[] } {
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
  const narrated = new Set(seen);
  const missing: string[] = [];
  order.forEach((id, i) => {
    if (narrated.has(id)) return;
    const isolated = i > 0 && narrated.has(order[i - 1]!) && (i === order.length - 1 || narrated.has(order[i + 1]!));
    const host = isolated && kept.find((l) => l.panel_ids.length < 3 && l.panel_ids.some((q) => Math.abs(pos.get(q)! - i) === 1));
    if (host) {
      host.panel_ids = [...host.panel_ids, id].sort((a, b) => pos.get(a)! - pos.get(b)!);
      repairs.push(`${id} was not narrated: joined L${host.n}`);
    } else {
      missing.push(id);
    }
  });
  return { lines: kept, repairs, missing };
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

const DEVANAGARI_DIGITS = "०१२३४५६७८९";

export function numbersToWords(text: string): string {
  return text
    .replace(/[०-९]/g, (d) => String(DEVANAGARI_DIGITS.indexOf(d)))
    .replace(/(?<=\d),(?=\d{2,3}(?:\D|$))/g, "")
    .replace(/\b(\d{1,2}):(\d{2})\b(\s*बजे)?/g, (_, h: string, m: string) => `${hindiNumber(Number(h))} बजकर ${hindiNumber(Number(m))} मिनट`)
    .replace(/(\d+)%/g, (_, n: string) => `${hindiNumber(Number(n))} प्रतिशत`)
    .replace(/(\d+)\.(\d+)/g, (_, a: string, b: string) => `${hindiNumber(Number(a))} दशमलव ${[...b].map((d) => ONES[Number(d)]).join(" ")}`)
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
const MAX_NARRATION = 2000;

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

/** The pack a chapter's numbered PDF was made as (panels/numbered.json): the answer must name the same one. */
export interface GeminiPack {
  chapter: string;
  pack: string;
  /** Panel ids in reading order. */
  order: string[];
}

export interface GeminiResult {
  /** The script, or null while there are errors. */
  doc: ScriptDoc | null;
  memory: StoryMemory;
  beats: string;
  /** Problems that block saving, each saying what to do. */
  errors: string[];
  /** Repairs and warnings for the review screen. */
  notes: string[];
}

/** The key a character is known by: the name before the first "|", lowercased, spaces collapsed (memory.rs). */
export function characterKey(line: string): string {
  return (line.split("|")[0] ?? "").trim().split(/\s+/).join(" ").toLowerCase();
}

/** True when one name's words are all in the other ("Jin" and "Kang Jin"): perhaps the same person. */
function similar(a: string, b: string): boolean {
  if (!a || !b || a === b) return false;
  const [short, long] = a.length < b.length ? [a, b] : [b, a];
  const words = new Set(long.split(" "));
  return short.split(" ").every((w) => words.has(w));
}

/**
 * The cards a chapter keeps (ADR 0007): a card shows only on a character's first appearance in the whole series.
 * `known` are the characters of every earlier chapter's approved memory, `characters` this answer's CHARACTERS
 * (where a card's role comes from). Returns each marked line's card, or null where it was dropped, with notes.
 */
export function chapterCards(marks: { n: number; card: string | null }[], characters: string[], known: string[]): { cards: Map<number, CharacterCard>; notes: string[] } {
  const cards = new Map<number, CharacterCard>();
  const notes: string[] = [];
  const earlier = new Map(known.map((c) => [characterKey(c), (c.split("|")[0] ?? "").trim()]));
  const listed = new Map(characters.map((c) => [characterKey(c), c]));
  const carded = new Set<string>();
  for (const { n, card } of marks) {
    if (!card) continue;
    const key = characterKey(card);
    if (earlier.has(key)) {
      notes.push(`L${n}: no card for ${card}: ${earlier.get(key)} appeared in an earlier chapter`);
      continue;
    }
    if (carded.has(key)) {
      notes.push(`L${n}: ${card} has a card on an earlier line already, this one is dropped`);
      continue;
    }
    carded.add(key);
    const line = listed.get(key);
    const role = (line?.split("|")[1] ?? "").trim();
    if (!line) notes.push(`L${n}: ${card} is not in CHARACTERS, so the card has no role; add one on the Script screen`);
    const like = [...earlier.entries()].find(([k]) => similar(k, key));
    if (like) notes.push(`L${n}: ${card} looks like ${like[1]} from an earlier chapter; if it is the same person, remove the card`);
    cards.set(n, { name: card, role });
  }
  for (const [key, line] of listed) {
    if (!earlier.has(key) && !carded.has(key)) notes.push(`No card for ${(line.split("|")[0] ?? "").trim()}, who is new in this chapter; add one on the Script screen if it should show`);
  }
  return { cards, notes };
}

/** Ids for a message: a short list, or "first–last (n panels)". */
function describe(ids: string[]): string {
  return ids.length > 4 ? `${ids[0]}–${ids[ids.length - 1]} (${ids.length} panels)` : ids.join(", ");
}

/**
 * Gemini's pasted answer, checked against the pack the chapter was exported as. `known` are the characters of the
 * earlier chapters (the prompt's CHARACTERS), so a card is kept only for a character's first appearance.
 */
export function readGeminiAnswer(text: string, pack: GeminiPack, known: string[] = []): GeminiResult {
  const parsed = parseTagged(text);
  const errors: string[] = [];
  const notes: string[] = [];
  const result = (doc: ScriptDoc | null): GeminiResult => ({ doc: errors.length ? null : doc, memory: parsed.memory, beats: parsed.beats, errors, notes });

  if (parsed.answers > 1 || new Set(parsed.lines.map((l) => l.n)).size < parsed.lines.length) {
    errors.push("Two answers are pasted (the narration starts again). Paste only the one you want to keep.");
  }
  if (!parsed.pack) {
    errors.push(`The answer has no PACK line. Its first line must be "PACK: ${pack.chapter} ${pack.pack}"; copy the prompt again and regenerate.`);
  } else if (parsed.pack.chapter !== pack.chapter || parsed.pack.id !== pack.pack) {
    errors.push(
      `This answer was written for ${parsed.pack.chapter} pack ${parsed.pack.id}, but this chapter's numbered PDF is ${pack.chapter} pack ${pack.pack}. ` +
        "Paste the answer for this chapter, or give Gemini the current numbered PDF.",
    );
  }
  if (parsed.lines.length === 0) {
    errors.push("No narration lines found. Each line must look like: L1 [p_0001] (calm) {hold} …");
    return result(null);
  }
  for (const l of parsed.lines) {
    // Pass 3 first, so the length checked is the one that is spoken (numbers grow when written as words).
    l.narration = loanwordsToLatin(numbersToWords(l.narration));
    if (!/[ऀ-ॿA-Za-z]/.test(l.narration)) errors.push(`L${l.n} has no narration.`);
    else if (l.narration.length > MAX_NARRATION) errors.push(`L${l.n} is ${l.narration.length} characters, over ${MAX_NARRATION}. Ask Gemini to split it.`);
    if (!l.emotion && l.emotion_text) notes.push(`L${l.n}: (${l.emotion_text}) is not one of the emotions, read as calm`);
    if (l.sfx && !SoundEffect.safeParse(l.sfx.toLowerCase()).success) {
      notes.push(`L${l.n}: <sfx: ${l.sfx}> is not one of the sound effects, so the line has none`);
    }
  }
  const { lines, repairs, missing } = fixPanels(parsed.lines, pack.order);
  notes.push(...repairs);
  if (missing.length) {
    const first = pack.order.indexOf(missing[0]!);
    const tail = missing.length === pack.order.length - first;
    errors.push(
      tail
        ? `The answer stops before ${missing[0]}: ${describe(missing)} have no narration. It was cut off; regenerate it (or lower the Thinking level).`
        : `No narration for ${describe(missing)}. Ask Gemini to narrate every labelled panel.`,
    );
  }
  if (!parsed.memory.story) errors.push("The MEMORY block (STORY SO FAR) is missing, so the next chapter would start without the story. Regenerate the answer.");
  for (let i = 1; i < lines.length; i++) {
    const before = pack.order.indexOf(lines[i - 1]!.panel_ids[0]!);
    const now = pack.order.indexOf(lines[i]!.panel_ids[0]!);
    if (now < before) notes.push(`L${lines[i]!.n} narrates ${lines[i]!.panel_ids[0]} after ${lines[i - 1]!.panel_ids[0]}: check the reading direction.`);
  }
  if (errors.length || lines.length === 0) return result(null);

  const { cards, notes: cardNotes } = chapterCards(lines, parsed.memory.characters, known);
  notes.push(...cardNotes);
  const built = ScriptDoc.safeParse({
    schema_version: 1,
    chapter_id: pack.chapter,
    lines: lines.map((l, i) => ({
      id: `l_${String(i + 1).padStart(4, "0")}`,
      panel_ids: l.panel_ids,
      narration: l.narration,
      emotion: l.emotion ?? "calm",
      voice: { speaker: "narrator", tone: "neutral", pace: "normal", intensity: 0.5 },
      preset: presetFor(l.move, l.panel_ids.length, (msg) => notes.push(`L${l.n}: ${msg}`)),
      // A short fade between lines: 59 hard cuts in five minutes play like a slideshow.
      transition_in: { name: i === 0 ? "cut" : "fade", duration: 0.3 },
      card: cards.get(l.n) ?? null,
      sfx: SoundEffect.safeParse(l.sfx?.toLowerCase()).data ?? null,
    })),
  });
  if (!built.success) {
    errors.push(`The script does not fit the format: ${built.error.issues.map((i) => `${i.path.join(".")}: ${i.message}`).join("; ")}`);
    return result(null);
  }
  const doc = built.data;
  for (const p of scriptProblems(doc)) errors.push(`L${lines[Number(p.path[1])]!.n}: ${p.message}`);
  return result(doc);
}
