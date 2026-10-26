# How to write each line's narration

The narration is read aloud by Fish Audio Drama 3 in a native Hindi voice. Write it so that voice sounds natural.

1. **Hindi words are written in Devanagari, always.** Never write Hindi in Latin letters.
   - Wrong: `Seoul ki raat hamesha shaant lagti thi.`
   - Right: `सियोल की रात हमेशा शांत लगती थी।`
2. **English words may stay in Latin letters**: names, game terms, words a Hindi speaker says in English.
   - Right: `System ने notification भेजा: Level up!`
   - Right: `Jinwoo ने तलवार उठाई। अब game शुरू होगा!`
3. **At least half the words of every line are Devanagari.** A line that is mostly English is rewritten in Hindi.
4. **Numbers are written as Hindi words:** `एक सौ बीस`, not `120`.
5. **Delivery belongs in the fields, not in the text.** Put the feeling in `emotion`, the manner in `voice.tone`
   (e.g. `whispered`, `menacing`) and the speed in `voice.pace`; the voice stage turns them into the model's cues.
   A short sound written where it happens is the only exception: `[laughs]`, `[sighs]`, `[gasps]`.
6. End sentences with `।`, `?` or `!` so the voice pauses where the panel does.

A line that breaks rules 1-3 fails validation (`contracts.rules.script_problems`) and is sent back for a rewrite.
