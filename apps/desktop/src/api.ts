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

/** Raw preview inputs for one chapter (`chapter_preview`); the screen validates them with @jumbo/schema. */
export interface PreviewSources {
  episode: unknown | null;
  script: unknown | null;
  script_file: string | null;
  panels: unknown | null;
  audio: unknown | null;
}

/** Panel detector for the vision stage. Magi v2 on Kaggle is the production path; OpenCV only tests the plumbing. */
export type Detector = "magi" | "opencv";

/** `vision://progress` events while a chapter is analyzed. */
export interface VisionProgress {
  chapter: string;
  progress: number;
  message: string;
}

/** panels/vision.report.json: how pages were stitched and what was found. */
export interface VisionReport {
  pages: number;
  detector: string;
  joins: { after_page: number; kind: "vertical" | "horizontal" | "none"; reason: string; cross: number; inner: number; correlation: number }[];
  canvases: { pages: number[]; kind: "page" | "strip" | "spread"; size: [number, number] }[];
  images: number;
  panels: number;
}

/** `script://progress` events while the Director writes a chapter. */
export interface ScriptProgress {
  chapter: string;
  progress: number;
  message: string;
}

/** script/director.report.json: one entry per scene the Director wrote. */
export interface DirectorReport {
  scenes: {
    scene: string;
    panels: string[];
    summary?: { lines: number; panels: number; covered: number; words: number; words_per_panel: number; quote_marks: number };
    pass2_mode?: string;
    panel_repairs?: string[];
    cleanup_notes?: string[];
    wall_s?: number;
    error?: string;
  }[];
  failed: string[];
  problems: string[];
}

/** The newest script of a chapter (`chapter_script`), with what the screen needs to review it. */
export interface ScriptView {
  script: unknown | null;
  script_file: string | null;
  report: DirectorReport | null;
  panels: unknown | null;
}

/** `voice://progress` events while a chapter is voiced. */
export type VoiceProgress = ScriptProgress;

/** audio/voice.report.json: how each line was spoken. */
export interface VoiceReport {
  lines: { line_id: string; voice: string; emotion_clip: string; duration: number; attempts: number; seed: number; plain?: boolean; problem?: string }[];
  total_s: number;
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
  chapterPreview: (chapter: string) => invoke<PreviewSources>("chapter_preview", { chapter }),
  analyzeChapter: (chapter: string, detector: Detector) => invoke<ChapterManifest>("analyze_chapter", { chapter, detector }),
  chapterPanels: (chapter: string) => invoke<{ panels: unknown; report: VisionReport | null }>("chapter_panels", { chapter }),
  writeScript: (chapter: string) => invoke<ChapterManifest>("write_script", { chapter }),
  chapterScript: (chapter: string) => invoke<ScriptView>("chapter_script", { chapter }),
  approveScript: (chapter: string, script: unknown) => invoke<ChapterManifest>("approve_script", { chapter, script }),
  voiceLibrary: () => invoke<Record<string, string[]>>("voice_library"),
  importVoice: (speaker: string, emotion: string, path: string) => invoke<void>("import_voice", { speaker, emotion, path }),
  recordVoice: (chapter: string) => invoke<ChapterManifest>("record_voice", { chapter }),
  chapterAudio: (chapter: string) => invoke<{ audio: unknown; report: VoiceReport | null }>("chapter_audio", { chapter }),
};

/** Tauri rejects with the Rust error string; normalise anything else. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

// Bridge (src-tauri/src/bridge.rs).

export type BridgeMode = "kaggle_tpu" | "kaggle_gpu" | "kaggle_cpu" | "local";

export interface BridgeSettings {
  mode: BridgeMode;
  kernel_slug: string;
  gist_id: string;
}

export interface CredentialStatus {
  kaggle: boolean;
  github: boolean;
  /** "file" when no OS credential store was available and secrets fell back to a private file. */
  storage: "os" | "file";
}

export type BootState =
  | { state: "pushing" | "queued" | "running" | "awaiting_rendezvous" }
  | { state: "connecting" | "authenticating" | "warming"; epoch: number }
  | { state: "retrying"; reason: string }
  | { state: "ready"; epoch: number; boot: Record<string, unknown> }
  | { state: "reconnecting"; from_epoch: number };

export type SessionView =
  | { phase: "idle" | "stopped" }
  | { phase: "booting"; boot: BootState; mode: BridgeMode; since_ms: number }
  | { phase: "ready"; epoch: number; facts: Record<string, unknown>; mode: BridgeMode; since_ms: number }
  | { phase: "reconnecting"; from_epoch: number }
  | { phase: "failed"; error: string };

export interface BridgeTestReport {
  bytes: number;
  upload_ms: number;
  job_ms: number;
  download_ms: number;
  verified: boolean;
}

/** Worker model pool (`GET /v1/pool` and `pool.status` events; worker/jumbo_worker/pool/manager.py). */
export interface PoolStatus {
  enabled: boolean;
  phase?: string | null;
  hbm_ceiling?: number;
  chips?: { chip: number; replica: string | null; reserved_gb: number; peak: number }[];
  gpus?: number;
  gpu_cards?: { gpu: number; reserved_gb: number; used_gb: number; peak: number; replicas: string[] }[];
  cpu?: { cores: number; ram_gb: number; used_cores: number; used_ram_gb: number };
  replicas?: { id: string; model: string; where: string; state: string; busy: number; throttled: boolean }[];
  queued?: Record<string, number>;
  broken?: Record<string, string>;
}

export const bridgeApi = {
  settings: () => invoke<BridgeSettings>("bridge_settings"),
  saveSettings: (settings: BridgeSettings) => invoke<void>("save_bridge_settings", { settings }),
  credentials: () => invoke<CredentialStatus>("credential_status"),
  importKaggleJson: (path: string) => invoke<void>("import_kaggle_json", { path }),
  setKaggleToken: (token: string) => invoke<void>("set_kaggle_token", { token }),
  setGithubToken: (token: string) => invoke<void>("set_github_token", { token }),
  view: () => invoke<SessionView>("session_view"),
  start: () => invoke<void>("start_session"),
  test: () => invoke<BridgeTestReport>("run_bridge_test"),
  stop: () => invoke<void>("stop_session"),
  pool: () => invoke<PoolStatus>("pool_status"),
};
