// Regenerates everything derived from packages/schema: the JSON Schema exports, then the worker's
// pydantic models. CI runs this and fails if the committed output changes.
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", shell: process.platform === "win32" && command === "npm" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("npm", ["run", "schema:export", "-w", "@jumbo/schema"]);

const python =
  process.env.JUMBO_PYTHON ??
  [join(root, "worker", ".venv", "bin", "python"), join(root, "worker", ".venv", "Scripts", "python.exe")].find(existsSync);
if (!python) {
  console.error('No worker venv. Create it with: python -m venv worker/.venv && worker/.venv/bin/pip install -e "worker[dev]"');
  process.exit(1);
}
run(python, [join("worker", "scripts", "codegen.py")]);
