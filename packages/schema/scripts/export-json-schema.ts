/**
 * Writes JSON Schema for every top-level contract to json/. The Python worker (pydantic codegen)
 * and the Rust crates (typify) consume these files, so they are committed and checked for drift in CI.
 *
 * Cross-field rules written as zod refinements (unique ids, panel counts, receipt chapters) are not
 * expressible in JSON Schema; consumers re-validate them or round-trip through this package.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  AudioManifest,
  EpisodeSpec,
  LedgerEvent,
  MemoryDelta,
  MULTI_PANEL_PRESETS,
  PanelsDoc,
  PRESET_CATALOG,
  SCHEMA_VERSION,
  ScriptDoc,
  SessionTicket,
} from "../src";

export const CONTRACTS = { PanelsDoc, ScriptDoc, AudioManifest, EpisodeSpec, MemoryDelta, LedgerEvent, SessionTicket } as const;

export const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "json");

type Json = { [key: string]: unknown };

/**
 * `prefault({})` makes zod parse an omitted object as `{}`, which still fails when the object has
 * required fields (e.g. hud_popup's `text`). JSON Schema would call such a field optional with an
 * invalid default, so producers (and constrained decoding) would leave it out. Make it required.
 */
function requireUnsatisfiableDefaults(schema: Json): Json {
  const defs = (schema.$defs ?? {}) as Record<string, Json>;
  const target = (node: Json): Json => {
    const ref = node.$ref;
    return typeof ref === "string" && ref.startsWith("#/$defs/") ? (defs[ref.slice(8)] ?? node) : node;
  };
  const visit = (node: unknown): void => {
    if (Array.isArray(node)) return node.forEach(visit);
    if (!node || typeof node !== "object") return;
    const obj = node as Json;
    const props = obj.properties as Record<string, Json> | undefined;
    for (const [key, prop] of Object.entries(props ?? {})) {
      const isEmptyDefault = prop.default !== undefined && JSON.stringify(prop.default) === "{}";
      if (isEmptyDefault && ((target(prop).required as string[] | undefined) ?? []).length > 0) {
        delete prop.default;
        obj.required = [...new Set([...((obj.required as string[] | undefined) ?? []), key])];
      }
    }
    Object.values(obj).forEach(visit);
  };
  visit(schema);
  return schema;
}

/** zod exports `z.literal(1)` as `"type": "number"`, so typed consumers would read 1.0. Say integer. */
function integerConsts(node: unknown): void {
  if (Array.isArray(node)) return node.forEach(integerConsts);
  if (!node || typeof node !== "object") return;
  const obj = node as Json;
  if (obj.type === "number" && Number.isInteger(obj.const)) obj.type = "integer";
  Object.values(obj).forEach(integerConsts);
}

export function renderContract(name: keyof typeof CONTRACTS): string {
  // "input" describes what producers may send: fields with defaults are optional.
  const schema = requireUnsatisfiableDefaults(z.toJSONSchema(CONTRACTS[name], { io: "input", unrepresentable: "any" }) as Json);
  integerConsts(schema);
  return `${JSON.stringify({ $id: `https://jumbo.local/schema/${name}.schema.json`, title: name, ...schema }, null, 2)}\n`;
}

export const CATALOG_FILE = "presets.catalog.json";

/** Constants other languages need alongside the schemas: the preset vocabulary and its panel counts. */
export function renderCatalog(): string {
  const catalog = {
    schema_version: SCHEMA_VERSION,
    multi_panel_presets: [...MULTI_PANEL_PRESETS].sort(),
    presets: PRESET_CATALOG,
  };
  return `${JSON.stringify(catalog, null, 2)}\n`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  mkdirSync(OUT_DIR, { recursive: true });
  for (const name of Object.keys(CONTRACTS) as (keyof typeof CONTRACTS)[]) {
    writeFileSync(join(OUT_DIR, `${name}.schema.json`), renderContract(name));
    console.log(`wrote json/${name}.schema.json`);
  }
  writeFileSync(join(OUT_DIR, CATALOG_FILE), renderCatalog());
  console.log(`wrote json/${CATALOG_FILE}`);
}
