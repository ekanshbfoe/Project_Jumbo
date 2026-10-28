"""Jumbo render, chunk __CHUNK__ of __CHUNKS__: the motion library's Episode composition renders this chunk's frames
of the chapter in the render job dataset on a Kaggle CPU machine. Output: /kaggle/working/chunk__CHUNK__.mp4 (no audio).
Exits with code 3 when Kaggle mounted an older version of the job dataset, so the app pushes this machine again."""
import base64, glob, json, os, subprocess, sys, tarfile, threading, time
from pathlib import Path

T0 = time.time()
W = "/kaggle/working"
KIT = Path("/kaggle/tmp/kit")
JOB = Path("/kaggle/tmp/job")
CHUNK, CHUNKS, STAMP = __CHUNK__, __CHUNKS__, "__STAMP__"
NODE = "22.11.0"
FILES = __KIT__


def log(msg):
    print(f"[{time.time() - T0:7.1f}s] {msg}", flush=True)


def sh(cmd, check=True, **kw):
    log("$ " + cmd[:300])
    p = subprocess.run(cmd, shell=True, text=True, capture_output=True, **kw)
    print((p.stdout + p.stderr).strip()[-4000:], flush=True)
    if check and p.returncode:
        raise SystemExit(f"failed ({p.returncode}): {cmd}")


def watchdog():
    while time.time() - T0 < 80 * 60:
        time.sleep(10)
    log("self_stop: 80 minutes")
    os._exit(1)


def unpack_job():
    """The job dataset holds job.tar; Kaggle may or may not have unpacked it already."""
    stamp = glob.glob("/kaggle/input/**/STAMP", recursive=True)
    archive = glob.glob("/kaggle/input/**/job.tar", recursive=True)
    if not stamp and not archive:
        # Kaggle mounted the dataset while a new version was still being processed: same as a stale job.
        print("stale render job: nothing mounted yet", flush=True)
        sys.exit(3)
    JOB.parent.mkdir(parents=True, exist_ok=True)
    if stamp:
        os.symlink(os.path.dirname(stamp[0]), JOB)
    else:
        with tarfile.open(archive[0]) as tar:
            tar.extractall(JOB, filter="data")
    found = (JOB / "STAMP").read_text().strip()
    if found != STAMP:
        log(f"stale render job: {found}, want {STAMP}")
        sys.exit(3)


threading.Thread(target=watchdog, daemon=True).start()
unpack_job()
for rel, text in FILES.items():
    path = KIT / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")

# Node from PyPI, which Kaggle reaches reliably (nodejs.org once hung for 40 minutes).
sh(f"timeout 300 pip install -q nodejs-wheel-binaries=={NODE}")
sh(f"cd {KIT} && npm install --no-audit --no-fund --loglevel=error 2>&1 | tail -5")
# Libraries Chrome's headless shell links against.
sh("apt-get update -qq >/dev/null 2>&1; DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libnss3 libatk1.0-0 libatk-bridge2.0-0 libcups2 "
   "libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libpango-1.0-0 libcairo2 libasound2 >/dev/null 2>&1", check=False)

env = dict(os.environ, MOTION_DIR=str(KIT / "motion"), RENDER_CONCURRENCY=str(os.cpu_count() or 4))
t = time.time()
out = f"{W}/chunk{CHUNK}.mp4"
p = subprocess.run(f"cd {KIT} && ./node_modules/.bin/tsx render-episode.mts {JOB}/episode.json {JOB}/public {out} --chunk={CHUNK}/{CHUNKS}",
                   shell=True, env=env, text=True, capture_output=True)
print("\n".join(l for l in (p.stdout + p.stderr).splitlines() if not l.strip().startswith("at "))[-6000:], flush=True)
log(f"render exit {p.returncode} after {time.time() - t:.0f}s")
sys.exit(p.returncode)
