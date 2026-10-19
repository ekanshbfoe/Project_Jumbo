import { useEffect, useState } from "react";
import { api, type ProjectInfo } from "./api";
import { MODULES } from "./hub/modules";
import { ProjectsScreen } from "./screens/ProjectsScreen";

const firstScreen = () => {
  const module = MODULES[0]!;
  return { module: module.id, screen: module.screens[0]!.id };
};

/** Hub shell: project chip, per-module navigation, and the active screen. */
export function App() {
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [ready, setReady] = useState(false);
  const [active, setActive] = useState(firstScreen);

  useEffect(() => {
    api
      .currentProject()
      .then(setProject)
      .finally(() => setReady(true));
  }, []);

  if (!ready) return null;
  if (!project) {
    return (
      <ProjectsScreen
        onOpened={(p) => {
          setProject(p);
          setActive(firstScreen());
        }}
      />
    );
  }

  const module = MODULES.find((m) => m.id === active.module);
  const screen = module?.screens.find((s) => s.id === active.screen);

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">Jumbo</div>
        <div className="project-chip">
          <div className="title">{project.title}</div>
          <div className="path mono" title={project.root}>
            {project.root}
          </div>
          <button
            style={{ marginTop: 8, width: "100%" }}
            onClick={() => api.closeProject().then(() => setProject(null))}
          >
            Switch project
          </button>
        </div>
        {MODULES.map((m) => (
          <nav key={m.id} className="nav-group">
            <h4>{m.title}</h4>
            {m.screens.map((s) => (
              <button
                key={s.id}
                className={`nav-item ${active.module === m.id && active.screen === s.id ? "active" : ""}`}
                onClick={() => setActive({ module: m.id, screen: s.id })}
              >
                {s.label}
              </button>
            ))}
          </nav>
        ))}
      </aside>
      <main className="main">{screen ? <screen.Component key={project.root} project={project} /> : null}</main>
    </div>
  );
}
