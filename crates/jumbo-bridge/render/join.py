"""Jumbo render, join: the __CHUNKS__ chunks rendered by the chunk machines (kernel sources), joined without re-encoding,
with the episode's soundtrack laid under them. Output: /kaggle/working/__NAME__.mp4.

The soundtrack puts each beat's clip at the beat's first frame, as the Episode composition plays it
(packages/motion/src/timeline.ts: a transition overlaps the previous beat, so a beat starts that much earlier)."""
import glob, json, os, shutil, subprocess, sys, tarfile, wave
from pathlib import Path

import numpy as np

W = Path("/kaggle/working")
JOB = Path("/kaggle/tmp/job")
CHUNKS, STAMP, NAME = __CHUNKS__, "__STAMP__", "__NAME__"


def sh(cmd):
    print("$", " ".join(cmd)[:300], flush=True)
    subprocess.run(cmd, check=True)


def unpack_job():
    stamp = glob.glob("/kaggle/input/**/STAMP", recursive=True)
    JOB.parent.mkdir(parents=True, exist_ok=True)
    if stamp:
        os.symlink(os.path.dirname(stamp[0]), JOB)
    else:
        with tarfile.open(glob.glob("/kaggle/input/**/job.tar", recursive=True)[0]) as tar:
            tar.extractall(JOB, filter="data")
    if (JOB / "STAMP").read_text().strip() != STAMP:
        print("stale render job", flush=True)
        sys.exit(3)


def soundtrack(spec, public, out):
    fps = spec["video"]["fps"]
    cursor, starts, prev = 0, [], None
    for i, b in enumerate(spec["beats"]):
        dur = max(1, round(b["duration"] * fps))
        want = 0 if i == 0 or b["transition_in"]["name"] == "cut" else round(b["transition_in"]["duration"] * fps)
        tin = min(want, prev - 1, dur - 1) if prev else 0
        start = cursor - tin
        starts.append(start)
        cursor, prev = start + dur, dur
    total = max(1, cursor)
    rate, mix = 44100, None
    for b, s in zip(spec["beats"], starts):
        if not b.get("audio"):
            continue
        with wave.open(str(public / b["audio"]["src"])) as w:
            r, pcm = w.getframerate(), np.frombuffer(w.readframes(w.getnframes()), "<i2").astype(np.int32)
        if mix is None:
            rate, mix = r, np.zeros(int(np.ceil(total / fps * r)) + r, np.int32)
        at = round(s / fps * rate)
        mix[at:at + len(pcm)] += pcm[: max(0, len(mix) - at)]
    n = int(round(total / fps * rate))
    mix = np.zeros(n, np.int32) if mix is None else mix[:n]
    with wave.open(str(out), "wb") as o:
        o.setnchannels(1)
        o.setsampwidth(2)
        o.setframerate(rate)
        o.writeframes(np.clip(mix, -32768, 32767).astype("<i2").tobytes())
    print(f"soundtrack: {total} frames, {total / fps:.2f} s, {rate} Hz", flush=True)


unpack_job()
ffmpeg = shutil.which("ffmpeg")
if not ffmpeg:
    subprocess.run("pip install -q imageio-ffmpeg==0.6.0", shell=True, check=True)
    import imageio_ffmpeg

    ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
chunks = []
for i in range(1, CHUNKS + 1):
    found = glob.glob(f"/kaggle/input/**/chunk{i}.mp4", recursive=True)
    if not found:
        raise SystemExit(f"chunk {i} is missing from the kernel sources")
    chunks.append(found[0])
(W / "chunks.txt").write_text("".join(f"file '{c}'\n" for c in chunks))
spec = json.loads((JOB / "episode.json").read_text())
soundtrack(spec, JOB / "public", W / "soundtrack.wav")
sh([ffmpeg, "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", str(W / "chunks.txt"), "-i", str(W / "soundtrack.wav"),
    "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart", str(W / f"{NAME}.mp4")])
for f in ("chunks.txt", "soundtrack.wav"):
    (W / f).unlink()
print(f"wrote {NAME}.mp4: {(W / f'{NAME}.mp4').stat().st_size / 1e6:.1f} MB", flush=True)
