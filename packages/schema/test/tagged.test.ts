import { describe, expect, it } from "vitest";
import { fixPanels, hindiNumber, numbersToWords, parseTagged, scriptProblems, taggedToScript } from "../src";

const ANSWER = `**BEATS:**
- Sylvie gets a call from the CEO's office

NARRATION:
L1 [p_0001, p_0002] (calm) {pan} एक ऊँची इमारत पर The Financier का नाम चमकता है, जहाँ हर दिन 24 घंटे खबरें बनती हैं।
L2 [p_0003] (tense) {zoom} उसका फोन बजता है और Sylvie उसे उठाती है।
this line is not tagged
L3 [p_0004, p_0099] (shock) {split_screen} उधर से आवाज़ कहती है कि interview एक हफ़्ते के लिए टल गया है।

MEMORY:
STORY SO FAR: Sylvie Jaine, a reporter for The Financier, learns her interview with Stefan Yard is postponed.
She pushes for a meeting tonight.
CHARACTERS:
- Sylvie Jaine | reporter | she | long black hair
- Luke Chase | CEO's assistant | he | voice on the phone`;

const ORDER = ["p_0001", "p_0002", "p_0003", "p_0004", "p_0005"];

describe("tagged Gemini answers", () => {
  it("reads lines, moves and the memory block", () => {
    const t = parseTagged(ANSWER);
    expect(t.lines.map((l) => [l.n, l.panel_ids, l.emotion, l.move])).toEqual([
      [1, ["p_0001", "p_0002"], "calm", "pan"],
      [2, ["p_0003"], "tense", "zoom"],
      [3, ["p_0004", "p_0099"], "shock", "split_screen"],
    ]);
    expect(t.beats).toBe("- Sylvie gets a call from the CEO's office");
    expect(t.memory.story).toBe(
      "Sylvie Jaine, a reporter for The Financier, learns her interview with Stefan Yard is postponed. She pushes for a meeting tonight.",
    );
    expect(t.memory.characters).toEqual(["Sylvie Jaine | reporter | she | long black hair", "Luke Chase | CEO's assistant | he | voice on the phone"]);
  });

  it("shows every panel once and drops lines left without one", () => {
    const lines = [
      { n: 1, panel_ids: ["p_0001"], emotion: null, move: null, narration: "a" },
      { n: 2, panel_ids: ["p_0001", "p_0042"], emotion: null, move: null, narration: "b" },
      { n: 3, panel_ids: ["p_0003"], emotion: null, move: null, narration: "c" },
    ];
    const fixed = fixPanels(lines, ["p_0001", "p_0002", "p_0003"]);
    expect(fixed.lines.map((l) => l.panel_ids)).toEqual([["p_0001", "p_0002"], ["p_0003"]]);
    expect(fixed.repairs).toEqual([
      "L2: p_0001 already shown, dropped",
      "L2: unknown panel p_0042 dropped",
      "L2: no valid panel, line dropped",
      "p_0002 was not narrated: joined L1",
    ]);
  });

  it("writes numbers as Hindi words", () => {
    expect(numbersToWords("3:58 बजे")).toBe("तीन बजकर अट्ठावन मिनट");
    expect(numbersToWords("50% और 120")).toBe("पचास प्रतिशत और एक सौ बीस");
    expect(hindiNumber(250000)).toBe("दो लाख पचास हज़ार");
  });

  it("becomes a valid ScriptDoc whose moves fit the panel counts", () => {
    const { doc, memory, notes } = taggedToScript(ANSWER, "ch_0001", ORDER);
    expect(doc.lines.map((l) => [l.id, l.panel_ids, l.preset.name])).toEqual([
      ["l_0001", ["p_0001", "p_0002"], "split_screen"],
      ["l_0002", ["p_0003"], "zoom"],
      ["l_0003", ["p_0004", "p_0005"], "split_screen"],
    ]);
    // {pan} is a single-panel move on a two-panel line; the missing p_0005 joins its neighbour's line.
    expect(notes).toContain("L1: {pan} does not fit 2 panels");
    expect(notes).toContain("p_0005 was not narrated: joined L3");
    expect(doc.lines[0]!.narration).toContain("चौबीस घंटे");
    expect(doc.lines[1]!.narration).toContain("phone");
    expect(scriptProblems(doc)).toEqual([]);
    expect(memory.characters).toHaveLength(2);
  });

  it("says what a usable answer looks like when nothing is tagged", () => {
    expect(() => taggedToScript("Here is your script!", "ch_0001", ORDER)).toThrow(/L1 \[p_0001\]/);
  });
});
