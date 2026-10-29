# ADR 0008: The narrator Mayu, hooks and the signature

Status: accepted (2026-10-29), Method 2

## Context

Manhwa and manga recap channels are crowded with deep, serious male voices reading flat play-by-play. Gemini writes
one chapter at a time and cannot see a whole one-hour episode, so a hook for the episode cannot come from one call.

## Decision

1. **The narrator is Mayu.** A soft-spoken young woman who has already read the chapter: warm, calm, a little
   teasing, dry understatement with the punch word last. The pattern interrupt is the contrast: brutal fights and big
   twists told calmly by a gentle voice. The persona is seasoning, not the meal: at most five persona sentences a
   chapter, always the last sentence of a line, never on two lines in a row, none on sad lines or the two after a
   death or real grief. One honest hint per chapter (यह ___ याद रखिए), the line मैं आगे पढ़ चुकी हूँ at most once,
   feminine first person, आप for the viewer, no greetings, kaomoji or laugh sounds. All of it lives in the System
   Instruction (`apps/desktop/src/method2/system_instruction.md`), with the writing-for-the-ear rules.
2. **Hooks come from each chapter, the choice is local.** Gemini's answer has a HOOKS section (one to three
   moments: panels that do not show the reveal, strength 1-5, type, spoiler low or high, a 10-15 s COLD open and a
   one-sentence TEASE), RECAP_IN (the story before the chapter, for an episode that starts there) and OPEN LOOPS in
   MEMORY (questions still open, carried into the next chapter's prompt). They are saved as `script/hooks.json`;
   an answer without them still works.
3. **An episode opens on the strongest low-spoiler hook of a later chapter** (ties to the later one, so the payoff
   comes inside the hour; a one-chapter episode may use its own), then one of five fixed bridge lines in Mayu's
   voice, the recap when the episode does not start the story, and the story. It ends on the tease of the chapter
   after it. `planEpisode` (packages/schema/src/hooks.ts) picks all of it with no model involved.
4. **The signature** "I am your dost or host, Mayu, signing off." is never written by Gemini and never translated.
   One recording (`<app config>/brand/signature.wav`) closes every episode of every channel as an audio logo, a
   second before the end, at the narration's loudness, so the music dips under it. The last part's hold grows to fit
   it (`endHold`), which renders that part again once when the clip is first set.
5. **Name slips** (a name one or two letters off a listed or more common one, "Everbank" beside "Evermark") are
   flagged when the answer is read, since the voice would say two different names.

## Not yet

The cold open, bridge, recap and tease are planned and shown on the Episodes screen but not voiced or rendered into
the video: that needs the new narrator voice and cloud runs, and waits for them.
