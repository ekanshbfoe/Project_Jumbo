import { useMemo, useState } from "react";
import { api, ENTITY_TYPES, errorText, type Entity, type EntityType, type VerifyReport } from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

const TYPE_LABELS: Record<EntityType, string> = {
  character: "Characters",
  faction: "Factions",
  location: "Locations",
  item: "Items",
  relationship: "Relationships",
  plot_thread: "Plot threads",
};

function displayName(entity: Entity): string {
  const name = entity.data.name ?? entity.data.title ?? entity.data.summary;
  return typeof name === "string" ? name : entity.id;
}

function renderValue(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

function EntityCard({ entity }: { entity: Entity }) {
  const [showReceipts, setShowReceipts] = useState(false);
  const status = entity.data.status;
  return (
    <div className="card">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <strong>{displayName(entity)}</strong>
        {typeof status === "string" ? (
          <span className={`badge ${status === "open" ? "warn" : "ok"}`}>{status}</span>
        ) : null}
      </div>
      <div className="muted mono">{entity.id}</div>
      <dl className="kv">
        {Object.entries(entity.data)
          .filter(([k]) => k !== "name")
          .map(([k, v]) => (
            <div key={k} style={{ display: "contents" }}>
              <dt>{k}</dt>
              <dd>{renderValue(v)}</dd>
            </div>
          ))}
      </dl>
      <button onClick={() => setShowReceipts((s) => !s)}>
        {showReceipts ? "Hide" : "Show"} {entity.receipts.length} receipt{entity.receipts.length === 1 ? "" : "s"}
      </button>
      {showReceipts
        ? entity.receipts.map((r, i) => (
            <div key={i} className="receipt">
              <span className="mono">
                {r.chapter_id} · {r.panel_id}
              </span>
              {r.quote ? <div className="muted">“{r.quote}”</div> : null}
            </div>
          ))
        : null}
    </div>
  );
}

export function ReceiptBookScreen(_: ScreenProps) {
  const book = useAsync(api.bookOverview);
  const [tab, setTab] = useState<EntityType>("character");
  const [verify, setVerify] = useState<VerifyReport | string>();

  const byType = useMemo(() => {
    const groups = new Map<EntityType, Entity[]>(ENTITY_TYPES.map((t) => [t, []]));
    for (const e of book.data?.entities ?? []) groups.get(e.entity_type)?.push(e);
    return groups;
  }, [book.data]);

  const runVerify = () =>
    api
      .verifyBook()
      .then(setVerify)
      .catch((e) => setVerify(errorText(e)));

  if (book.error) return <p className="error">{book.error}</p>;
  if (!book.data) return <div className="empty">Loading…</div>;
  const b = book.data;

  return (
    <div>
      <div className="page-header">
        <h1>Receipt Book</h1>
        <div className="row">
          <span className={`badge ${b.locked ? "ok" : "warn"}`}>{b.locked ? "Locked" : "Session open"}</span>
          <button onClick={book.reload}>Refresh</button>
          <button onClick={runVerify}>Verify integrity</button>
        </div>
      </div>

      <div className="card row" style={{ gap: 28, marginBottom: 16 }}>
        <div>
          <div className="muted">Ledger entries</div>
          <strong>{b.ledger_len}</strong>
        </div>
        <div>
          <div className="muted">Head</div>
          <code>{b.ledger_head.slice(0, 16)}…</code>
        </div>
        <div>
          <div className="muted">Snapshots</div>
          <strong>{b.snapshots.length}</strong>
          {b.lock.snapshot ? <span className="muted"> (latest {b.lock.snapshot})</span> : null}
        </div>
        <div>
          <div className="muted">Entities</div>
          <strong>{b.entities.length}</strong>
        </div>
      </div>

      {b.repaired ? <p className="badge warn">Recovered after a crash: {b.repaired}</p> : null}
      {verify ? (
        typeof verify === "string" ? (
          <p className="error">{verify}</p>
        ) : verify.problems.length === 0 ? (
          <p className="badge ok">
            Verified: {verify.ledger_len} ledger entries and {verify.snapshots} snapshot(s) are intact.
          </p>
        ) : (
          <div className="card error">
            {verify.problems.map((p) => (
              <div key={p}>{p}</div>
            ))}
          </div>
        )
      ) : null}

      <div className="tabs">
        {ENTITY_TYPES.map((t) => (
          <button key={t} className={tab === t ? "active" : ""} onClick={() => setTab(t)}>
            {TYPE_LABELS[t]} <span className="muted">{byType.get(t)?.length ?? 0}</span>
          </button>
        ))}
      </div>
      {byType.get(tab)?.length ? (
        <div className="grid">
          {byType.get(tab)!.map((e) => (
            <EntityCard key={e.id} entity={e} />
          ))}
        </div>
      ) : (
        <div className="card empty">
          No {TYPE_LABELS[tab].toLowerCase()} yet. They appear here as approved chapters are committed.
        </div>
      )}

      {b.snapshots.length ? (
        <>
          <h3>Snapshots</h3>
          <div className="card">
            <table>
              <thead>
                <tr>
                  <th>Snapshot</th>
                  <th>Created</th>
                  <th>Label</th>
                  <th>Ledger</th>
                  <th>Tier files</th>
                </tr>
              </thead>
              <tbody>
                {[...b.snapshots].reverse().map((s) => (
                  <tr key={s.id}>
                    <td className="mono">{s.id}</td>
                    <td>{new Date(s.created_at).toLocaleString()}</td>
                    <td>{s.label ?? <span className="muted">—</span>}</td>
                    <td>{s.ledger_len}</td>
                    <td>{Object.keys(s.tiers).length}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      ) : null}
    </div>
  );
}
