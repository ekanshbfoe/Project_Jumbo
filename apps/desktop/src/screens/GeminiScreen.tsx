import { readGeminiAnswer } from "@jumbo/schema";
import { save } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";
import { api, errorText, type GeminiView } from "../api";
import { clearDraft, loadDraft, saveDraft } from "../drafts";
import type { ScreenProps } from "../hub/modules";
import instruction from "../method2/system_instruction.md?raw";
import { useAsync } from "../useAsync";

/** The AI Studio settings ADR 0006 settled on, shown next to the System Instruction. */
const SETTINGS = [
  ["Model", "Gemini 3.8 Flash"],
  ["Thinking level", "Medium (High if speakers get mixed up)"],
  ["Media resolution", "High"],
  ["Output length", "65536"],
  ["Safety settings", "all Off"],
  ["Tools", "all Off, Structured outputs too"],
] as const;

/** What to type into a new AI Studio chat for this chapter, under the System Instruction. */
function promptFor(view: GeminiView): string {
  const pack = view.pack!;
  const ctx = view.context;
  return [
    `PACK: ${pack.chapter} ${pack.pack}`,
    `STORY SO FAR: ${ctx?.story || "This is the first chapter."}`,
    "CHARACTERS:",
    ...(ctx?.characters.length ? ctx.characters : ["(none yet)"]),
    ...(ctx?.last_lines.length ? [`LAST NARRATED LINES: ${ctx.last_lines.join(" / ")}`] : []),
    "",
    `Narrate this chapter now: every labelled panel from ${pack.order[0]} to ${pack.order[pack.order.length - 1]} (${pack.order.length} panels), in the answer format, starting with the PACK line.`,
  ].join("\n");
}

/** Copies text, falling back to a hidden textarea where the async clipboard is not allowed. */
async function copy(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    document.body.appendChild(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

/**
 * Method 2's script step (ADR 0006): give Gemini the numbered PDF with the System Instruction and the prompt, paste
 * its answer back, check the story memory it wrote, and save the script for review on the Script screen.
 */
/** What the Gemini screen keeps for a chapter until its script is saved. */
interface GeminiDraft {
  answer: string;
  story: string;
  characters: string;
  /** The parsed memory the boxes were last filled from, so hand edits survive changes elsewhere in the answer. */
  memoryFrom: string;
}

export function GeminiScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const [chapter, setChapter] = useState<string>();
  const view = useAsync(() => (chapter ? api.geminiPack(chapter) : Promise.resolve(null)), [chapter]);
  const [answer, setAnswer] = useState("");
  const [story, setStory] = useState("");
  const [characters, setCharacters] = useState("");
  const [memoryFrom, setMemoryFrom] = useState("");
  const [copied, setCopied] = useState<string>();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState<string>();
  const current = chapters.data?.find((c) => c.id === chapter);

  useEffect(() => {
    const waiting = chapters.data?.find((c) => c.state === "analyzed");
    if (!chapter && chapters.data?.length) setChapter((waiting ?? chapters.data[0]!).id);
  }, [chapters.data, chapter]);

  // The chapter's unsaved answer and memory edits come back when the screen or the chapter is opened again.
  const [restored, setRestored] = useState<string>();
  useEffect(() => {
    const draft = chapter ? loadDraft<GeminiDraft>(project.root, chapter, "gemini") : undefined;
    setAnswer(draft?.answer ?? "");
    setStory(draft?.story ?? "");
    setCharacters(draft?.characters ?? "");
    setMemoryFrom(draft?.memoryFrom ?? "");
    setSaved(undefined);
    setError(undefined);
    setRestored(chapter);
  }, [chapter, project.root]);

  useEffect(() => {
    if (chapter && restored === chapter) saveDraft(project.root, chapter, "gemini", { answer, story, characters, memoryFrom } satisfies GeminiDraft);
  }, [chapter, restored, project.root, answer, story, characters, memoryFrom]);

  const pack = view.data?.pack ?? null;
  const read = useMemo(() => {
    if (!pack || !answer.trim()) return null;
    try {
      return readGeminiAnswer(answer, pack);
    } catch (e) {
      return { doc: null, memory: { story: "", characters: [] }, beats: "", errors: [`The answer could not be read: ${errorText(e)}`], notes: [] };
    }
  }, [answer, pack]);

  // A new answer brings its own memory. The boxes are refilled only when the parsed memory itself changes, so
  // hand edits survive fixing a narration line in the answer.
  const parsedMemory = JSON.stringify(read?.memory ?? null);
  useEffect(() => {
    if (restored !== chapter || parsedMemory === memoryFrom) return;
    setStory(read?.memory.story ?? "");
    setCharacters(read?.memory.characters.join("\n") ?? "");
    setMemoryFrom(parsedMemory);
  }, [parsedMemory, restored, chapter]);

  const flash = async (what: string, text: string) => {
    await copy(text);
    setCopied(what);
    setTimeout(() => setCopied((c) => (c === what ? undefined : c)), 2000);
  };

  const exportPdf = async () => {
    if (!chapter) return;
    setError(undefined);
    try {
      const dest = await save({ title: "Save the numbered PDF for Gemini", defaultPath: `${chapter}-numbered.pdf`, filters: [{ name: "PDF", extensions: ["pdf"] }] });
      if (!dest) return;
      await api.exportNumberedPdf(chapter, dest);
      setCopied("pdf");
    } catch (e) {
      setError(errorText(e));
    }
  };

  const saveScript = async () => {
    if (!chapter || !pack || !read?.doc) return;
    setError(undefined);
    try {
      await api.saveGeminiScript(chapter, pack.pack, read.doc, { story, characters: characters.split("\n").map((c) => c.trim()).filter(Boolean) });
      setSaved(`Saved ${read.doc.lines.length} lines. Review and approve the script on the Script screen.`);
      clearDraft(project.root, chapter, "gemini");
      chapters.reload();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const blocked = view.data?.context_error ?? null;
  const editable = current?.state === "analyzed" || current?.state === "directed";
  const canSave = !!read?.doc && !!story.trim() && !blocked && editable;
  const words = read?.doc?.lines.reduce((n, l) => n + l.narration.split(/\s+/).length, 0) ?? 0;

  return (
    <div>
      <div className="page-header">
        <h1>Gemini script</h1>
        <div className="row">
          <select value={chapter ?? ""} onChange={(e) => setChapter(e.target.value)} disabled={!chapters.data?.length}>
            {chapters.data?.map((c) => (
              <option key={c.id} value={c.id}>
                {c.id} · {c.title ?? c.original_name} ({c.state})
              </option>
            ))}
          </select>
        </div>
      </div>
      {error || view.error ? <p className="error">{error ?? view.error}</p> : null}
      {view.data && !pack ? (
        <div className="card empty">No numbered PDF yet. Press Make video on the Chapters screen: it finds the panels, then stops here for Gemini.</div>
      ) : null}
      {view.data && pack ? (
        <div className="stack">
          {blocked ? <div className="card error">{blocked}</div> : null}

          <div className="card stack">
            <strong>1. Give Gemini the chapter</strong>
            <span className="muted">
              {pack.chapter} · pack <span className="mono">{pack.pack}</span> · {pack.order.length} panels on {pack.pages} pages ·{" "}
              {(pack.bytes / 1e6).toFixed(1)} MB
            </span>
            <div className="row">
              <button onClick={exportPdf}>{copied === "pdf" ? "PDF saved" : "Save numbered PDF…"}</button>
              <button onClick={() => flash("instruction", instruction)}>{copied === "instruction" ? "Copied" : "Copy System Instruction"}</button>
              <button className="primary" disabled={!!blocked} onClick={() => flash("prompt", promptFor(view.data!))}>
                {copied === "prompt" ? "Copied" : "Copy prompt"}
              </button>
            </div>
            <span className="muted">
              In AI Studio: start a new chat, paste the System Instruction into "System instructions", attach the PDF, paste the prompt and run. Settings:{" "}
              {SETTINGS.map(([k, v]) => `${k}: ${v}`).join(" · ")}.
            </span>
          </div>

          <div className="card stack">
            <strong>2. Paste Gemini's answer</strong>
            <textarea
              className="answer"
              placeholder={`PACK: ${pack.chapter} ${pack.pack}\n\nBEATS:\n…\n\nNARRATION:\nL1 [${pack.order[0]}] (calm) {hold} …\n\nMEMORY:\nSTORY SO FAR: …\nCHARACTERS:\n…`}
              value={answer}
              onChange={(e) => {
                setAnswer(e.target.value);
                setSaved(undefined);
              }}
              rows={10}
            />
            {read?.errors.length ? (
              <ul className="error issues">
                {read.errors.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            ) : read?.doc ? (
              <span className="badge ok">
                {read.doc.lines.length} lines · {words} words · all {pack.order.length} panels narrated
              </span>
            ) : null}
            {read?.notes.length ? (
              <ul className="muted issues">
                {read.notes.map((n) => (
                  <li key={n}>{n}</li>
                ))}
              </ul>
            ) : null}
          </div>

          <div className="card stack">
            <strong>3. Check the story memory</strong>
            <span className="muted">The next chapter continues from this once the script is approved. Fix names or facts here if Gemini got them wrong.</span>
            <label className="stack">
              <span className="muted">Story so far</span>
              <textarea value={story} onChange={(e) => setStory(e.target.value)} rows={4} />
            </label>
            <label className="stack">
              <span className="muted">Characters, one per line: Name | role | he/she | look</span>
              <textarea value={characters} onChange={(e) => setCharacters(e.target.value)} rows={4} />
            </label>
            <div className="row" style={{ justifyContent: "flex-end" }}>
              {current && !editable ? (
                <span className="muted">
                  {chapter} is {current.state}. To paste a new answer, send it back to Directed on the Chapters screen first.
                </span>
              ) : null}
              {saved ? <span className="badge ok">{saved}</span> : null}
              <button className="primary" disabled={!canSave} onClick={saveScript} title={editable ? "" : `${chapter} is ${current?.state}; a pasted script goes onto an analyzed chapter`}>
                Save script
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
