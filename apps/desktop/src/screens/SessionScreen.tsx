import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
  bridgeApi,
  errorText,
  type BootState,
  type BridgeMode,
  type BridgeSettings,
  type BridgeTestReport,
  type SessionView,
} from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

const MODES: { id: BridgeMode; label: string; hint: string }[] = [
  { id: "kaggle_tpu", label: "Kaggle TPU v5e-8", hint: "Production. Uses TPU quota." },
  { id: "kaggle_cpu", label: "Kaggle CPU", hint: "Tests the full cloud path without spending TPU quota." },
  { id: "local", label: "Local worker", hint: "Runs the worker on this PC. No Kaggle, no tunnel." },
];

const STEPS: { id: BootState["state"]; label: string }[] = [
  { id: "pushing", label: "Push kernel" },
  { id: "queued", label: "Kaggle queue" },
  { id: "running", label: "Kernel running" },
  { id: "awaiting_rendezvous", label: "Tunnel + rendezvous" },
  { id: "connecting", label: "Connecting" },
  { id: "ready", label: "Ready" },
];

interface LogLine {
  ts: number;
  text: string;
}

function describe(event: Record<string, unknown>): string {
  switch (event.type) {
    case "job.progress":
      return `job ${String(event.job_id).slice(0, 8)} ${Math.round(Number(event.progress) * 100)}% ${event.message ?? ""}`;
    case "job.done":
      return `job ${String(event.job_id).slice(0, 8)} done`;
    case "job.failed":
      return `job ${String(event.job_id).slice(0, 8)} failed: ${event.error}`;
    case "tunnel.epoch":
      return `tunnel moved to epoch ${event.epoch}`;
    case "session.draining":
      return `worker draining (${event.reason})`;
    default:
      return String(event.type);
  }
}

function elapsed(since: number, now: number) {
  const s = Math.max(0, Math.round((now - since) / 1000));
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

function Timeline({ view }: { view: SessionView }) {
  const current = view.phase === "booting" ? view.boot.state : view.phase === "ready" ? "ready" : null;
  const reached = current ? STEPS.findIndex((s) => s.id === current) : -1;
  const local = (view.phase === "booting" || view.phase === "ready") && view.mode === "local";
  return (
    <div className="stack" style={{ gap: 6 }}>
      {STEPS.filter((s) => !local || !["pushing", "queued", "running"].includes(s.id)).map((step) => {
        const i = STEPS.findIndex((s) => s.id === step.id);
        const done = i < reached || current === "ready";
        const active = i === reached && current !== "ready";
        return (
          <div key={step.id} className="row">
            <span className={`badge ${done ? "ok" : active ? "warn" : ""}`} style={{ minWidth: 22, textAlign: "center" }}>
              {done ? "✓" : active ? "…" : "·"}
            </span>
            <span className={done || active ? "" : "muted"}>{step.label}</span>
          </div>
        );
      })}
    </div>
  );
}

export function SessionScreen(_: ScreenProps) {
  const settings = useAsync(bridgeApi.settings);
  const credentials = useAsync(bridgeApi.credentials);
  const [draft, setDraft] = useState<BridgeSettings>();
  const [view, setView] = useState<SessionView>({ phase: "idle" });
  const [log, setLog] = useState<LogLine[]>([]);
  const [report, setReport] = useState<BridgeTestReport>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [github, setGithub] = useState("");
  const [kaggleToken, setKaggleToken] = useState("");
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    if (settings.data && !draft) setDraft(settings.data);
  }, [settings.data, draft]);

  useEffect(() => {
    bridgeApi.view().then(setView).catch(() => {});
    const unlisten = [
      listen<SessionView>("bridge://state", (e) => setView(e.payload)),
      listen<Record<string, unknown>>("bridge://event", (e) =>
        setLog((lines) => [...lines.slice(-79), { ts: Date.now(), text: describe(e.payload) }]),
      ),
    ];
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      clearInterval(tick);
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, []);

  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      credentials.reload();
    }
  };

  const running = view.phase === "booting" || view.phase === "ready" || view.phase === "reconnecting";
  const local = draft?.mode === "local";

  return (
    <div>
      <div className="page-header">
        <h1>Session</h1>
        <div className="row">
          {!running ? (
            <button className="primary" disabled={busy || !draft} onClick={() => act(async () => {
              setLog([]);
              setReport(undefined);
              if (draft) await bridgeApi.saveSettings(draft);
              await bridgeApi.start();
            })}>
              Launch worker
            </button>
          ) : (
            <button disabled={busy} onClick={() => act(bridgeApi.stop)}>
              Stop session
            </button>
          )}
        </div>
      </div>
      {error ? <p className="error">{error}</p> : null}

      <div className="grid" style={{ gridTemplateColumns: "minmax(320px, 1fr) minmax(320px, 1fr)" }}>
        <div className="card stack">
          <strong>Status</strong>
          {view.phase === "failed" ? <div className="error">{view.error}</div> : null}
          {view.phase === "idle" || view.phase === "stopped" ? (
            <span className="muted">{view.phase === "stopped" ? "Session stopped." : "No worker running."}</span>
          ) : null}
          {view.phase === "reconnecting" ? <span className="badge warn">Tunnel dropped; reconnecting from epoch {view.from_epoch}…</span> : null}
          {running || view.phase === "failed" ? <Timeline view={view} /> : null}
          {view.phase === "booting" || view.phase === "ready" ? (
            <span className="muted">Elapsed {elapsed(view.since_ms, now)}</span>
          ) : null}
          {view.phase === "ready" ? (
            <>
              <dl className="kv">
                <dt>Epoch</dt>
                <dd>{view.epoch}</dd>
                {Object.entries(view.facts).map(([k, v]) => (
                  <div key={k} style={{ display: "contents" }}>
                    <dt>{k}</dt>
                    <dd>{String(v)}</dd>
                  </div>
                ))}
              </dl>
              <button disabled={busy} onClick={() => act(async () => setReport(await bridgeApi.test()))}>
                {busy ? "Testing…" : "Run bridge test (8 MiB round trip)"}
              </button>
              {report ? (
                <div className={`badge ${report.verified ? "ok" : "danger"}`}>
                  {report.verified ? "Verified" : "MISMATCH"}: {(report.bytes / 1048576).toFixed(0)} MiB · upload {report.upload_ms} ms · job{" "}
                  {report.job_ms} ms · download {report.download_ms} ms
                </div>
              ) : null}
            </>
          ) : null}
        </div>

        <div className="card stack">
          <strong>Worker events</strong>
          <div className="mono" style={{ maxHeight: 260, overflow: "auto" }}>
            {log.length ? (
              log.map((l, i) => (
                <div key={i}>
                  <span className="muted">{new Date(l.ts).toLocaleTimeString()}</span> {l.text}
                </div>
              ))
            ) : (
              <span className="muted">Events from the worker appear here.</span>
            )}
          </div>
        </div>
      </div>

      {draft ? (
        <div className="card stack" style={{ marginTop: 16 }}>
          <strong>Bridge settings</strong>
          <div className="row" style={{ flexWrap: "wrap" }}>
            {MODES.map((m) => (
              <button
                key={m.id}
                className={draft.mode === m.id ? "primary" : ""}
                disabled={running}
                title={m.hint}
                onClick={() => setDraft({ ...draft, mode: m.id })}
              >
                {m.label}
              </button>
            ))}
          </div>
          <span className="muted">{MODES.find((m) => m.id === draft.mode)?.hint}</span>
          {!local ? (
            <>
              <label className="stack" style={{ gap: 4 }}>
                <span className="muted">Kaggle kernel (owner/kernel-slug)</span>
                <input disabled={running} value={draft.kernel_slug} placeholder="yourname/jumbo-worker" onChange={(e) => setDraft({ ...draft, kernel_slug: e.target.value })} />
              </label>
              <label className="stack" style={{ gap: 4 }}>
                <span className="muted">Rendezvous gist id</span>
                <input disabled={running} value={draft.gist_id} placeholder="from gist.github.com/you/<id>" onChange={(e) => setDraft({ ...draft, gist_id: e.target.value })} />
              </label>
            </>
          ) : null}
          <div className="row">
            <button disabled={busy || running} onClick={() => act(() => bridgeApi.saveSettings(draft))}>
              Save settings
            </button>
          </div>
        </div>
      ) : null}

      {!local ? (
        <div className="card stack" style={{ marginTop: 16 }}>
          <strong>Credentials</strong>
          {credentials.data?.storage === "file" ? (
            <span className="muted">
              <span className="badge warn">Stored in a private file</span> No system keyring was found, so tokens are kept in a file only
              your user can read, in the app settings folder.
            </span>
          ) : (
            <span className="muted">Stored in the system credential store, never in the project folder.</span>
          )}
          <div className="row">
            <span className={`badge ${credentials.data?.kaggle ? "ok" : "warn"}`}>Kaggle {credentials.data?.kaggle ? "✓" : "missing"}</span>
            <button
              disabled={busy}
              onClick={() =>
                act(async () => {
                  const path = await open({ title: "Select kaggle.json", filters: [{ name: "kaggle.json", extensions: ["json"] }] });
                  if (typeof path === "string") await bridgeApi.importKaggleJson(path);
                })
              }
            >
              Import kaggle.json…
            </button>
            <input type="password" placeholder="or paste a Kaggle API token" value={kaggleToken} onChange={(e) => setKaggleToken(e.target.value)} style={{ flex: 1 }} />
            <button disabled={busy || !kaggleToken} onClick={() => act(async () => { await bridgeApi.setKaggleToken(kaggleToken); setKaggleToken(""); })}>
              Save
            </button>
          </div>
          <div className="row">
            <span className={`badge ${credentials.data?.github ? "ok" : "warn"}`}>GitHub gist {credentials.data?.github ? "✓" : "missing"}</span>
            <input type="password" placeholder="classic token with only the gist scope" value={github} onChange={(e) => setGithub(e.target.value)} style={{ flex: 1 }} />
            <button disabled={busy || !github} onClick={() => act(async () => { await bridgeApi.setGithubToken(github); setGithub(""); })}>
              Save
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
