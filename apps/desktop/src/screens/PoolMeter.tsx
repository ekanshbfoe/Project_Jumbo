import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { bridgeApi, type PoolStatus } from "../api";

const CHIP_GB = 16;
// Usable memory of a Kaggle T4, as the worker's pool counts it (gpu_hbm_gb).
const GPU_GB = 15;

/** Live view of the worker's model pool: who holds each TPU chip or GPU card, memory against the ceiling, CPU use, queues. */
export function PoolMeter() {
  const [pool, setPool] = useState<PoolStatus>();

  useEffect(() => {
    bridgeApi.pool().then(setPool).catch(() => {});
    const off = listen<Record<string, unknown>>("bridge://event", (e) => {
      if (e.payload.type === "pool.status") setPool({ enabled: true, ...(e.payload as Omit<PoolStatus, "enabled">) });
    });
    return () => {
      off.then((f) => f());
    };
  }, []);

  if (!pool) return <span className="muted">Loading pool…</span>;
  if (!pool.enabled) return <span className="muted">This worker has no model pool.</span>;
  const ceiling = pool.hbm_ceiling ?? 0.8;
  const cpu = pool.cpu;
  const queued = Object.entries(pool.queued ?? {});
  const broken = Object.entries(pool.broken ?? {});

  return (
    <div className="stack">
      <div className="row">
        <span className="badge">phase: {pool.phase ?? "idle"}</span>
        <span className="muted">HBM ceiling {Math.round(ceiling * 100)}% ({(ceiling * CHIP_GB).toFixed(1)} GB/chip)</span>
      </div>
      {pool.chips?.length ? (
        <div className="chips">
          {pool.chips.map((c) => (
            <div key={c.chip} className="chip" title={`${c.replica ?? "free"} · reserved ${c.reserved_gb} GB · peak ${Math.round(c.peak * 100)}%`}>
              <div className="chip-bar">
                <div className="chip-fill" style={{ height: `${(c.reserved_gb / CHIP_GB) * 100}%` }} />
                <div className="chip-peak" style={{ bottom: `${c.peak * 100}%` }} />
                <div className="chip-ceiling" style={{ bottom: `${ceiling * 100}%` }} />
              </div>
              <span className="mono">{c.chip}</span>
              <span className="muted chip-owner">{c.replica?.split("#")[0] ?? "—"}</span>
            </div>
          ))}
        </div>
      ) : pool.gpu_cards?.length ? (
        <div className="chips">
          {pool.gpu_cards.map((g) => {
            const busy = (pool.replicas ?? []).filter((r) => r.where === `gpu:${g.gpu}` && r.busy > 0).length;
            return (
              <div key={g.gpu} className="chip" title={`${g.replicas.join(", ") || "free"} · reserved ${g.reserved_gb} GB · in use ${g.used_gb} GB · peak ${Math.round(g.peak * 100)}%`}>
                <div className="chip-bar">
                  <div className="chip-fill" style={{ height: `${(g.reserved_gb / GPU_GB) * 100}%` }} />
                  <div className="chip-peak" style={{ bottom: `${g.peak * 100}%` }} />
                  <div className="chip-ceiling" style={{ bottom: `${ceiling * 100}%` }} />
                </div>
                <span className="mono">GPU {g.gpu}</span>
                <span className="muted chip-owner">
                  {g.replicas.length} repl · {busy} busy · {g.used_gb.toFixed(1)} GB
                </span>
              </div>
            );
          })}
        </div>
      ) : (
        <span className="muted">No TPU or GPU on this worker; models run on CPU.</span>
      )}
      {cpu ? (
        <span className="muted">
          CPU {cpu.used_cores}/{cpu.cores} cores · RAM {cpu.used_ram_gb}/{cpu.ram_gb} GB
        </span>
      ) : null}
      {pool.replicas?.length ? (
        <div className="muted mono">
          {pool.replicas.map((r) => (
            <div key={r.id}>
              {r.id} {r.where} {r.state}
              {r.busy ? ` · ${r.busy} busy` : ""}
              {r.throttled ? " · throttled" : ""}
            </div>
          ))}
        </div>
      ) : null}
      {queued.length ? <span className="muted">Queued: {queued.map(([m, n]) => `${m} ${n}`).join(", ")}</span> : null}
      {broken.map(([m, why]) => (
        <span key={m} className="badge danger">
          {m} cannot start: {why}
        </span>
      ))}
    </div>
  );
}
