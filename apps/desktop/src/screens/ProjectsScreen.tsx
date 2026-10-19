import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api, errorText, type ProjectInfo } from "../api";
import { useAsync } from "../useAsync";

async function pickFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" ? picked : null;
}

/** Shown when no project is open: open a folder, create a project, or reopen a recent one. */
export function ProjectsScreen({ onOpened }: { onOpened: (project: ProjectInfo) => void }) {
  const recent = useAsync(api.recentProjects);
  const [title, setTitle] = useState("");
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
      return path ? api.createProject(path, title) : null;
    });

  return (
    <div className="welcome stack">
      <div>
        <h1 style={{ marginBottom: 4 }}>Jumbo</h1>
        <p className="muted" style={{ marginTop: 0 }}>
          Turn manhwa chapters into narrated motion recaps. Projects live entirely in a folder on this PC.
        </p>
      </div>

      <div className="card stack">
        <strong>New project</strong>
        <div className="row">
          <input
            placeholder="Series title, e.g. Solo Leveling"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            style={{ flex: 1 }}
          />
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
                <div>{p.title}</div>
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
