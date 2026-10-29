# ADR 0007: Episodes, one long video from many chapters

**Status:** accepted 2026-10-29 (project lead). No intro, outro, hook or chapter title cards for now: they are
planned together with the narrator's personality. Episode parts join straight, chapter into chapter.

## Decision
1. **Make video holds at Voiced.** A voiced chapter waits to become part of an episode; the Render screen still
   renders one chapter on its own.
2. **An episode** is a run of chapters in story order with none skipped, each chapter in one episode only
   (`episodes/<ep_NNNN>/episode.json`).
3. **Each chapter renders as its own part** on the Kaggle CPU machines, exactly like a chapter render, with:
   - the follow card shown early only in the first part (`social_promo.early`) and at the end only in the last;
   - its soundtrack downloaded beside the video (`render/part.mp4`, `render/part.wav`).
4. **Parts are fingerprinted:** the part's EpisodeSpec, every file it plays, and the renderer (render kit, chunk and
   join scripts). A part is current only while all of them are unchanged, so an unchanged chapter is never rendered
   twice and a re-voiced or edited one is never joined stale.
5. **The join runs on this PC** with a pinned static ffmpeg (6.1.1, SHA-256 checked, downloaded once into the
   app's folder). The video is copied without re-encoding; the parts' soundtracks are joined and encoded once.
6. **Brand music** is one app-wide track for every project and channel (`<app config>/brand`). It loops under
   the whole episode at a set level (default -18 dB), fades in over 2 s and out over 3 s, and dips under the
   narration (sidechain compression), so it swells in the pauses.
7. **Character cards** show a character's name and short English role once in the whole series, on the line
   where the character first appears (`ScriptLine.card`, carried onto the line's first beat):
   - Gemini marks the line with `<card: Name>`; the role comes from its CHARACTERS list.
   - Whether it is the first appearance is decided by the app, from the approved memory of every earlier chapter,
     never by Gemini, so a new Gemini account or chat cannot bring a card back.
   - The Gemini screen drops a card for a known character and notes repeats, unlisted names, names like an earlier
     one ("Jin" and "Kang Jin") and new characters without a card. The Script screen edits, removes or adds cards.
   - Approving refuses a card for a character an earlier chapter knows, a second card for one character and an
     unnamed card, and adds every carded character missing from the chapter's memory to it.

## Why
- **Not one render of the whole hour.** A chunk machine stops itself after 80 minutes, one failure would lose
  the whole render, and any fix would re-render every chapter. Per-chapter parts fail and resume alone.
- **The join is local.** The parts are already on this PC; joining on Kaggle would upload about 2 GB again.
  Copying video and re-encoding only the sound takes minutes on a small PC.
- **Soundtracks, not AAC tracks.** Joining the parts' WAVs avoids AAC priming gaps between parts and lets the
  music run continuously across chapter boundaries.

## Consequences
- 10 chapters render one after another (one render at a time shares the Kaggle job dataset), about 3 hours, on
  CPU quota only.
- A part is a chapter's render too: rendering a part moves the chapter to Rendered.
