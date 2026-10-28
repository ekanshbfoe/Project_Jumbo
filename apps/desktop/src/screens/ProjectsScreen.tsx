import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api, errorText, type Method, type ProjectInfo } from "../api";
import { useAsync } from "../useAsync";

async function pickFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" ? picked : null;
}

/** The two ways a manga or manhwa project gets its scripts (ADR 0005, ADR 0006). */
export const METHODS: { id: Method; title: string; subtitle: string; detail: string }[] = [
  {
    id: "method1",
    title: "Method 1",
    subtitle: "Fully automatic on Kaggle",
    detail: "Qwen writes the script on the T4s (about 1 min a panel). One button runs everything.",
  },
  {
    id: "method2",
    title: "Method 2",
    subtitle: "Script from Gemini in Google AI Studio",
    detail: "You give Gemini a numbered PDF and paste its answer back. Panels, voice and render stay automatic.",
  },
];

export const methodLabel = (m: Method) => METHODS.find((x) => x.id === m)!.title;

/** Manga / Manhwa projects: create one (choosing its method once), open a folder, or reopen a recent one. */
export function ProjectsScreen({ onOpened, onBack }: { onOpened: (project: ProjectInfo) => void; onBack: () => void }) {
  const recent = useAsync(api.recentProjects);
  const [title, setTitle] = useState("");
  const [method, setMethod] = useState<Method>("method2");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const run = async (action: () => Promise<ProjectInfo | null>) => {
    setBusy(true);
    setError(undefined);
    try {
      const project = await action();
      if (project) onOpened(project);
    } catch (e) {
      setError(errorText(e));
      recent.reload();
    } finally {
      setBusy(false);
    }
  };

  const openExisting = () =>
    run(async () => {
      const path = await pickFolder("Open a Jumbo project");
      return path ? api.openProject(path) : null;
    });

  const createNew = () =>
    run(async () => {
      const path = await pickFolder("Choose an empty folder for the new project");
      return path ? api.createProject(path, title, method) : null;
    });

  return (
    <div className="welcome stack">
      <div>
        <button className="link" onClick={onBack}>
          ← All content
        </button>
        <h1 style={{ marginBottom: 4 }}>Manga / Manhwa</h1>
        <p className="muted" style={{ marginTop: 0 }}>
          Turn chapters into narrated motion recaps.
        </p>
      </div>

      <div className="card stack">
        <strong>New project</strong>
        <input placeholder="Series title, e.g. Solo Leveling" value={title} onChange={(e) => setTitle(e.target.value)} />
        <span className="muted">How should its scripts be written? This is chosen once, before the first script.</span>
        <div className="method-cards" role="radiogroup">
          {METHODS.map((m) => (
            <button
              key={m.id}
              role="radio"
              aria-checked={method === m.id}
              className={`method-card ${method === m.id ? "selected" : ""}`}
              title={m.detail}
              onClick={() => setMethod(m.id)}
            >
              <strong>{m.title}</strong>
              <span>{m.subtitle}</span>
              <span className="muted">{m.detail}</span>
            </button>
          ))}
        </div>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <button className="primary" disabled={busy || !title.trim()} onClick={createNew}>
            Create in folder…
          </button>
        </div>
      </div>

      <div className="card stack">
        <div className="row" style={{ justifyContent: "space-between" }}>
          <strong>Recent projects</strong>
          <button disabled={busy} onClick={openExisting}>
            Open folder…
          </button>
        </div>
        {recent.data?.length ? (
          recent.data.map((p) => (
            <div key={p.path} className="row" style={{ justifyContent: "space-between" }}>
              <div>
                <div>
                  {p.title} <span className="badge">{methodLabel(p.method)}</span>
                </div>
                <div className="muted mono">{p.path}</div>
              </div>
              <div className="row">
                <button disabled={busy} onClick={() => run(() => api.openProject(p.path))}>
                  Open
                </button>
                <button
                  disabled={busy}
                  title="Remove from this list (the project folder is untouched)"
                  onClick={() => api.forgetRecentProject(p.path).then(recent.reload)}
                >
                  ✕
                </button>
              </div>
            </div>
          ))
        ) : (
          <span className="muted">No recent projects yet.</span>
        )}
      </div>

      {error ? <div className="error">{error}</div> : null}
    </div>
  );
}
