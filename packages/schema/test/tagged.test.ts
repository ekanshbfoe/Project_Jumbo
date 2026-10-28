import { describe, expect, it } from "vitest";
import { cleanLine, fixPanels, hindiNumber, numbersToWords, panelIds, parseTagged, readGeminiAnswer, scriptProblems } from "../src";

const PACK = { chapter: "ch_0001", pack: "7f3a", order: ["p_0001", "p_0002", "p_0003", "p_0004", "p_0005"] };

const ANSWER = `PACK: ch_0001 7f3a

**BEATS:**
- Sylvie gets a call from the CEO's office

NARRATION:
L1 [p_0001, p_0002] (calm) {pan} एक ऊँची इमारत पर The Financier का नाम चमकता है, जहाँ हर दिन 24 घंटे खबरें बनती हैं।
L2 [p_0003] (tense) {zoom} उसका फोन बजता है और Sylvie उसे उठाती है।
L3 [p_0004, p_0005] (shock) {split_screen} उधर से आवाज़ कहती है कि interview एक हफ़्ते के लिए टल गया है।

MEMORY:
STORY SO FAR: Sylvie Jaine, a reporter for The Financier, learns her interview with Stefan Yard is postponed.
She pushes for a meeting tonight.
CHARACTERS:
- Sylvie Jaine | reporter | she | long black hair
- Luke Chase | CEO's assistant | he | voice on the phone`;

describe("reading a pasted Gemini answer", () => {
  it("turns a clean answer into a valid ScriptDoc with its memory", () => {
    const r = readGeminiAnswer(ANSWER, PACK);
    expect(r.errors).toEqual([]);
    expect(r.doc!.lines.map((l) => [l.id, l.panel_ids, l.preset.name])).toEqual([
      ["l_0001", ["p_0001", "p_0002"], "split_screen"],
      ["l_0002", ["p_0003"], "zoom"],
      ["l_0003", ["p_0004", "p_0005"], "split_screen"],
    ]);
    // {pan} is a single-panel move on a two-panel line.
    expect(r.notes).toContain("L1: {pan} does not fit 2 panels");
    expect(r.doc!.lines[0]!.narration).toContain("चौबीस घंटे");
    expect(r.doc!.lines[1]!.narration).toContain("phone");
    expect(r.memory.story).toBe("Sylvie Jaine, a reporter for The Financier, learns her interview with Stefan Yard is postponed. She pushes for a meeting tonight.");
    expect(r.memory.characters).toEqual(["Sylvie Jaine | reporter | she | long black hair", "Luke Chase | CEO's assistant | he | voice on the phone"]);
    expect(scriptProblems(r.doc!)).toEqual([]);
  });

  it("reads the Markdown and list markers AI Studio adds when copied", () => {
    const messy = ANSWER.replace("L1 [p_0001, p_0002]", "**L1 [p_0001, p_0002]**")
      .replace("L2 [p_0003]", "- L2 \\[p\\_0003\\]")
      .replace("L3 [p_0004, p_0005]", "3. L3: 【P_4 - p5】")
      .replace("MEMORY:", "## MEMORY")
      .replace("STORY SO FAR:", "**STORY SO FAR:**");
    const r = readGeminiAnswer(messy, PACK);
    expect(r.errors).toEqual([]);
    expect(r.doc!.lines.map((l) => l.panel_ids)).toEqual([["p_0001", "p_0002"], ["p_0003"], ["p_0004", "p_0005"]]);
    expect(r.memory.story).toMatch(/^Sylvie Jaine/);
  });

  it("joins a narration that wrapped onto the next line", () => {
    const wrapped = ANSWER.replace("और Sylvie उसे उठाती है।", "और Sylvie\nउसे उठाती है।");
    expect(readGeminiAnswer(wrapped, PACK).doc!.lines[1]!.narration).toBe("उसका phone बजता है और Sylvie उसे उठाती है।");
  });

  it("reads an answer AI Studio joined into hard-wrapped paragraphs", () => {
    const joined = `PACK: ch_0001 7f3a

BEATS:

  - Sylvie gets a call from the
    CEO's office

NARRATION: L1 [p_0001, p_0002] (calm) {pan} एक ऊँची इमारत पर The Financier का
नाम चमकता है। L2 [p_0003] (tense) {zoom} उसका फोन बजता है। L3 [p_0004,
p_0005] (shock) {split_screen} उधर से आवाज़ कहती है कि interview टल गया है।

MEMORY: STORY SO FAR: Sylvie Jaine, a reporter, learns her interview is postponed. CHARACTERS: Sylvie Jaine | Reporter
at The Financier | she | long dark hair, white
coat Luke Chase | CEO's assistant | he | unseen caller Stephan Yard
| CEO of Everbank | he | tall businessman`;
    const r = readGeminiAnswer(joined, PACK);
    expect(r.errors).toEqual([]);
    expect(r.doc!.lines.map((l) => l.panel_ids)).toEqual([["p_0001", "p_0002"], ["p_0003"], ["p_0004", "p_0005"]]);
    expect(r.doc!.lines[0]!.narration).toBe("एक ऊँची इमारत पर The Financier का नाम चमकता है।");
    expect(r.memory.story).toBe("Sylvie Jaine, a reporter, learns her interview is postponed.");
    expect(r.memory.characters).toEqual([
      "Sylvie Jaine | Reporter at The Financier | she | long dark hair, white coat",
      "Luke Chase | CEO's assistant | he | unseen caller",
      "Stephan Yard | CEO of Everbank | he | tall businessman",
    ]);
  });

  it("refuses an answer that was cut off and says where it stops", () => {
    const cut = ANSWER.split("L3 ")[0]!;
    const r = readGeminiAnswer(cut, PACK);
    expect(r.doc).toBeNull();
    expect(r.errors).toContain("The answer stops before p_0004: p_0004, p_0005 have no narration. It was cut off; regenerate it (or lower the Thinking level).");
    expect(r.errors.some((e) => e.startsWith("The MEMORY block"))).toBe(true);
  });

  it("refuses an answer for another chapter or another export of this one", () => {
    expect(readGeminiAnswer(ANSWER.replace("ch_0001 7f3a", "ch_0004 7f3a"), PACK).errors[0]).toMatch(/written for ch_0004 pack 7f3a/);
    expect(readGeminiAnswer(ANSWER.replace("ch_0001 7f3a", "ch_0001 b201"), PACK).errors[0]).toMatch(/pack b201/);
    expect(readGeminiAnswer(ANSWER.replace("PACK: ch_0001 7f3a", ""), PACK).errors[0]).toMatch(/no PACK line/);
  });

  it("refuses two answers pasted together", () => {
    const twice = `${ANSWER}\n\nNARRATION:\nL1 [p_0001] (calm) {hold} दूसरा जवाब।`;
    expect(readGeminiAnswer(twice, PACK).errors[0]).toMatch(/Two answers are pasted/);
  });

  it("refuses empty, overlong and Romanized lines", () => {
    const empty = ANSWER.replace("उसका फोन बजता है और Sylvie उसे उठाती है।", "");
    expect(readGeminiAnswer(empty, PACK).errors).toContain("L2 has no narration.");
    const long = ANSWER.replace("उसका फोन", `${"बहुत ".repeat(450)}उसका फोन`);
    expect(readGeminiAnswer(long, PACK).errors[0]).toMatch(/^L2 is \d+ characters, over 2000/);
    const roman = ANSWER.replace("उसका फोन बजता है और Sylvie उसे उठाती है।", "Uska phone bajta hai aur Sylvie use uthati hai.");
    expect(readGeminiAnswer(roman, PACK).errors[0]).toMatch(/^L2: narration has Hindi in Latin letters/);
  });

  it("keeps Gemini's order and warns when it differs from the panels' reading order", () => {
    const swapped = ANSWER.replace("L2 [p_0003]", "L2 [p_0004, p_0005]").replace("L3 [p_0004, p_0005] (shock) {split_screen}", "L3 [p_0003] (shock) {zoom}");
    const r = readGeminiAnswer(swapped, PACK);
    expect(r.doc!.lines.map((l) => l.panel_ids[0])).toEqual(["p_0001", "p_0004", "p_0003"]);
    expect(r.notes).toContain("L3 narrates p_0003 after p_0004: check the reading direction.");
  });

  it("says what a usable answer looks like when nothing is tagged", () => {
    expect(readGeminiAnswer("Here is your script!", PACK).errors).toContain("No narration lines found. Each line must look like: L1 [p_0001] (calm) {hold} …");
  });
});

describe("tagged answer pieces", () => {
  it("cleans copy-paste noise from a line", () => {
    expect(cleanLine("  > **L1 \\[p\\_0001\\]** (calm)")).toBe("L1 [p_0001] (calm)");
    expect(cleanLine("12. L12 [p_0040]")).toBe("L12 [p_0040]");
  });

  it("reads panel ids however they are written", () => {
    expect(panelIds("p_0007, P_8, p0009")).toEqual(["p_0007", "p_0008", "p_0009"]);
    expect(panelIds("p_0003-p_0005")).toEqual(["p_0003", "p_0004", "p_0005"]);
  });

  it("joins only a single skipped panel to its neighbour's line", () => {
    const line = (n: number, ids: string[]) => ({ n, panel_ids: ids, emotion: null, move: null, narration: "x" });
    const fixed = fixPanels([line(1, ["p_0001"]), line(2, ["p_0001", "p_0042"]), line(3, ["p_0003"])], ["p_0001", "p_0002", "p_0003", "p_0004", "p_0005"]);
    expect(fixed.lines.map((l) => l.panel_ids)).toEqual([["p_0001", "p_0002"], ["p_0003"]]);
    expect(fixed.missing).toEqual(["p_0004", "p_0005"]);
    expect(fixed.repairs).toEqual(["L2: p_0001 already shown, dropped", "L2: unknown panel p_0042 dropped", "L2: no valid panel, line dropped", "p_0002 was not narrated: joined L1"]);
  });

  it("writes numbers as Hindi words, Devanagari digits and decimals included", () => {
    expect(numbersToWords("3:58 बजे")).toBe("तीन बजकर अट्ठावन मिनट");
    expect(numbersToWords("50% और 120")).toBe("पचास प्रतिशत और एक सौ बीस");
    expect(numbersToWords("३ लोग, 1,500 रुपये, 2.5 किलो")).toBe("तीन लोग, एक हज़ार पाँच सौ रुपये, दो दशमलव पाँच किलो");
    expect(hindiNumber(250000)).toBe("दो लाख पचास हज़ार");
  });

  it("parses a memory block whose keys share a line with the heading", () => {
    expect(parseTagged("MEMORY: STORY SO FAR: A happens.\nCHARACTERS: Ana | hero | she | red coat").memory).toEqual({
      story: "A happens.",
      characters: ["Ana | hero | she | red coat"],
    });
  });
});
