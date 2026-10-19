import { invoke } from "@tauri-apps/api/core";

// Mirrors the DTOs in src-tauri/src/commands.rs and the serde types in jumbo-core / jumbo-ledger.

export interface ProjectInfo {
  root: string;
  id: string;
  title: string;
  language: string;
  created_at: string;
}

export interface RecentProject {
  path: string;
  title: string;
  opened_at: number;
}

export const CHAPTER_STATES = [
  "imported",
  "rasterized",
  "analyzed",
  "directed",
  "librarian",
  "approved",
  "voiced",
  "rendered",
  "consolidated",
] as const;
export type ChapterState = (typeof CHAPTER_STATES)[number];

export interface ChapterManifest {
  id: string;
  title?: string;
  original_name: string;
  layout?: "strip" | "page";
  state: ChapterState;
  stale: boolean;
  imported_at: string;
  updated_at: string;
  history: { from: ChapterState; to: ChapterState; at: string }[];
}

export const ENTITY_TYPES = ["character", "faction", "location", "item", "relationship", "plot_thread"] as const;
export type EntityType = (typeof ENTITY_TYPES)[number];

export interface Receipt {
  chapter_id: string;
  panel_id: string;
  quote?: string;
}

export interface Entity {
  entity_type: EntityType;
  id: string;
  data: Record<string, unknown>;
  receipts: Receipt[];
  provenance: Record<string, string>;
  created_seq: number;
  updated_seq: number;
}

export interface LockState {
  locked: boolean;
  snapshot?: string;
  ledger_len: number;
  ledger_head: string;
  at?: string;
}

export interface SnapshotManifest {
  id: string;
  created_at: string;
  label?: string;
  session?: string;
  ledger_len: number;
  ledger_head: string;
  state_hash: string;
  tiers: Record<string, string>;
  prev?: string;
  hash: string;
}

export interface BookOverview {
  locked: boolean;
  lock: LockState;
  ledger_len: number;
  ledger_head: string;
  entities: Entity[];
  snapshots: SnapshotManifest[];
  repaired: string | null;
}

export interface VerifyReport {
  ledger_len: number;
  snapshots: number;
  problems: string[];
}

export const api = {
  createProject: (path: string, title: string) => invoke<ProjectInfo>("create_project", { path, title }),
  openProject: (path: string) => invoke<ProjectInfo>("open_project", { path }),
  closeProject: () => invoke<void>("close_project"),
  currentProject: () => invoke<ProjectInfo | null>("current_project"),
  recentProjects: () => invoke<RecentProject[]>("recent_projects"),
  forgetRecentProject: (path: string) => invoke<void>("forget_recent_project", { path }),
  listChapters: () => invoke<ChapterManifest[]>("list_chapters"),
  importChapters: (paths: string[]) => invoke<ChapterManifest[]>("import_chapters", { paths }),
  bookOverview: () => invoke<BookOverview>("book_overview"),
  verifyBook: () => invoke<VerifyReport>("verify_book"),
};

/** Tauri rejects with the Rust error string; normalise anything else. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
