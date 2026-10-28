# ADR 0006: Method 2, the script from Gemini in Google AI Studio

**Status:** accepted 2026-10-28 (project lead). Built on the `method-two` branch; Method 1 (ADR 0005) is unchanged.

## Decision
Method 2 replaces only the Director. Panels, voice and render are Method 1's.

1. **Panels:** Magi v2 on the T4s, as in Method 1 (~3–5 min a chapter). The vision job also writes
   `panels/numbered.pdf`: every page with each panel outlined and labelled `p_0007`.
2. **Script:** the person drops the numbered PDF into Google AI Studio with the saved System Instruction
   (`apps/desktop/src/method2/system_instruction.md`) and the prompt the app copies for them (the story so far and
   the characters from the previous chapter), then pastes Gemini's answer back into the app.
3. **Answer format:** tagged plain text, not JSON: `L3 [p_0005, p_0006] (tense) {split_screen} …Hindi…`, then a
   MEMORY block. `packages/schema/src/tagged.ts` turns it into a ScriptDoc with Director v3's Pass 3 cleanup
   (panel coverage, numbers in Hindi words, loanwords in Latin letters, camera moves fitted to the panel count).
4. **Voice and render:** the approved script goes through Drama 3 and the Kaggle CPU render exactly as in Method 1.

## Why
- **Magi stays for the visuals.** The renderer needs exact crops, polygons, reading order and bubble outlines.
  Gemini can return boxes, but their accuracy on comic panels is unmeasured; Magi costs minutes. The labelled
  PDF gives Gemini and the renderer the same panel ids, so Gemini never locates panels itself.
- **No structured output.** Constrained JSON measurably lowers writing quality, the AI Studio schema editor has
  known bugs (`$defs`), and an answer cut off by the output limit is unusable JSON but still parseable text.
- **Memory lives in the project, not in AI Studio.** Each answer ends with STORY SO FAR and CHARACTERS; the app
  stores them per chapter (`script/memory.json`) and puts them in the next chapter's prompt. Any Google account,
  any new chat, gets the same context.

## AI Studio settings
Gemini 3.8 Flash; Thinking Medium (High if speakers get mixed up); Media resolution High (1120 tokens a page, small
bubble text); Output length 65536; Safety all Off; every tool Off; Structured output Off; one new chat per chapter.
Gemini 4 Argon (Google AI Pro) only to compare on hard chapters.

## Limits (2026-10-28)
- AI Studio web: free, per-model daily caps since 2026-09-29, numbers unpublished; Pro raises them.
- Gemini API free tier, 3.8 Flash: 5 requests/min, 20/day per Google project, 250K tokens/min. One chapter is one
  request, so a later "Generate with Gemini" button could do 20 chapters a day per project.

## UI
Home shows content types (Manga/Manhwa; Book Explanation and Novels say "Coming soon"). Manga/Manhwa lists projects
with their method; a new project picks Method 1 or Method 2 once, stored in `jumbo.project.toml`. A Method 2 project's
sidebar has "Gemini script" (export the numbered PDF, copy the system instruction and prompt, paste the answer)
in place of Method 1's Director.

## Hardening (self-audit, 2026-10-28)
- **Pack stamp:** every numbered PDF page says `ch_0003 . pack 7f3a`; the answer's first line repeats it, and the
  app refuses an answer for another chapter or an older export (the pack id hashes the panel ids and outlines).
- **Strict answers:** a cut-off answer (panels left unnarrated), two answers pasted together, an empty or overlong
  line, Romanized Hindi, or a missing MEMORY block is refused with what to do. Copy-paste noise (Markdown, list
  markers, escapes, wrapped lines, loose panel ids and ranges) is cleaned first. Gemini's line order is kept and a
  reading-order mismatch is flagged instead of silently re-sorted.
- **Readable labels:** pages taller than 2.2 widths become overlapping PDF pages; labels are sized for Gemini's
  scaling to 3072 px, placed over the gutter, semi-transparent and never overlapping; the PDF stays under 40 MB.
- **Memory chain:** memory counts once the script is approved; a chapter continues from exactly the chapter before
  it (otherwise it waits); characters are merged over all earlier chapters; changing an approved story marks later
  scripted chapters stale. One format for both methods (`story`, `characters`, `last_lines`, `method`).
- **Guards:** the project's method is stored (old projects are Method 1) and fixed once a chapter has a script;
  Method 2's Make video never runs the Director or approves on its own; approval checks the Devanagari rule before
  any voice session is spent.
