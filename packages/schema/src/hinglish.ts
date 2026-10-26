import type { ScriptDoc } from "./script";

/**
 * The script's writing rule: narration is Devanagari, or Mixed Hinglish (Devanagari with English words in Latin
 * script). Hindi written in Latin letters is refused, since Drama 3 reads it with a foreign accent (spike 4).
 * It is a rule of scripts meant for voicing (the Director's output, the approval gate, the voice job), not of the
 * ScriptDoc shape, so preview fixtures with English labels stay valid.
 * worker/jumbo_worker/contracts/rules.py mirrors this; fixtures/hinglish-cases.json is shared by both tests.
 */

/** Hindi words that, written in Latin letters, mark a line as Romanized Hindi rather than English. */
export const ROMANIZED_HINDI = new Set([
  "hai", "hain", "tha", "thi", "thay", "ka", "ki", "ke", "ko", "ne", "se", "mein", "aur", "nahi", "nahin", "kya",
  "kyun", "kyon", "toh", "bhi", "yeh", "ye", "woh", "wo", "ek", "lekin", "sirf", "abhi", "kuch", "raha", "rahi",
  "rahe", "hoga", "hogi", "gaya", "gayi", "uska", "uski", "uske", "apna", "apni", "mera", "meri", "tera", "teri",
]);

const CUE = /\[[^\]]*\]/g;
const DEVANAGARI = /[ऀ-ॿ]/;
const LATIN = /[A-Za-z]/;

/** Why `text` breaks the writing rule, or null when it is Devanagari or Mixed Hinglish. */
export function narrationProblem(text: string): string | null {
  // Bracketed cues such as [laughs] are directions to the voice, not words of the line.
  const words = text.replace(CUE, " ").split(/\s+/).filter((w) => DEVANAGARI.test(w) || LATIN.test(w));
  const devanagari = words.filter((w) => DEVANAGARI.test(w)).length;
  const romanized = words
    .map((w) => w.toLowerCase().replace(/[^a-z]/g, ""))
    .filter((w, i) => !DEVANAGARI.test(words[i]!) && ROMANIZED_HINDI.has(w));
  if (romanized.length > 0) return `narration has Hindi in Latin letters (${[...new Set(romanized)].join(", ")}); write it in Devanagari`;
  if (devanagari === 0 || devanagari * 2 < words.length) {
    return "narration must be Devanagari or Mixed Hinglish: at least half its words in Devanagari, English words may stay in Latin";
  }
  return null;
}

/** Every line of `doc` that breaks the writing rule, as zod-style issues. */
export function scriptProblems(doc: Pick<ScriptDoc, "lines">): { path: (string | number)[]; message: string }[] {
  return doc.lines.flatMap((line, i) => {
    const message = narrationProblem(line.narration);
    return message ? [{ path: ["lines", i, "narration"], message }] : [];
  });
}
