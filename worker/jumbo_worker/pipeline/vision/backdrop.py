"""The blurred backdrop behind a beat's panel, made once per panel instead of by the renderer on every frame.

The motion library used to draw a beat's background as the panel's own image under a CSS
`blur(40px) brightness(0.4) saturate(1.2)` at 120% of a 1920x1080 frame. A full-screen 40 px blur on every frame was the
largest part of render time (video test 1: 2.2 frames a second on 4 CPU cores). The same look is made here once, at a
small size: blurred with the radius the CSS blur has at that size, darkened and saturated alike. Upscaled by the browser,
a blurred image loses nothing.

Backfill an analyzed chapter:  python -m jumbo_worker.pipeline.vision.backdrop CHAPTER_DIR
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import cv2
import numpy as np

WIDTH = 480
JPEG_QUALITY = 85
# The renderer's backdrop box: 120% of the 1920x1080 frame, filled ("cover").
BOX = (2304, 1296)
CSS_BLUR_PX = 40
BRIGHTNESS = 0.4
SATURATE = 1.2


def make_backdrop(rgb: np.ndarray) -> np.ndarray:
    """The panel image as the renderer's blurred, darkened backdrop, `WIDTH` pixels wide (RGB uint8)."""
    h, w = rgb.shape[:2]
    small = cv2.resize(rgb, (WIDTH, max(1, round(h * WIDTH / w))), interpolation=cv2.INTER_AREA)
    sh, sw = small.shape[:2]
    # The CSS blur radius is in frame pixels; the small image is drawn this many times larger.
    scale = max(BOX[0] / sw, BOX[1] / sh)
    blurred = cv2.GaussianBlur(small.astype(np.float32), (0, 0), sigmaX=CSS_BLUR_PX / scale, borderType=cv2.BORDER_REFLECT)
    # CSS saturate(): mix with the luminance; brightness(): scale.
    lum = (blurred @ np.array([0.2126, 0.7152, 0.0722], np.float32))[..., None]
    out = (lum + (blurred - lum) * SATURATE) * BRIGHTNESS
    return np.clip(np.round(out), 0, 255).astype(np.uint8)


def encode_jpeg(rgb: np.ndarray) -> bytes:
    ok, buf = cv2.imencode(".jpg", cv2.cvtColor(rgb, cv2.COLOR_RGB2BGR), [cv2.IMWRITE_JPEG_QUALITY, JPEG_QUALITY])
    if not ok:
        raise RuntimeError("jpeg encoding failed")
    return buf.tobytes()


def backfill(chapter_dir: Path) -> int:
    """Adds a backdrop to every panel of an analyzed chapter that has a crop and no backdrop yet; returns how many."""
    project = chapter_dir.parent.parent
    doc_path = chapter_dir / "panels" / "panels.json"
    doc = json.loads(doc_path.read_text())
    made = 0
    for panel in doc["panels"]:
        if panel.get("backdrop") or not panel.get("crop"):
            continue
        bgr = cv2.imread(str(project / panel["crop"]), cv2.IMREAD_COLOR)
        if bgr is None:
            continue
        rel = f"panels/backdrops/{panel['id']}.jpg"
        (chapter_dir / rel).parent.mkdir(parents=True, exist_ok=True)
        (chapter_dir / rel).write_bytes(encode_jpeg(make_backdrop(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))))
        panel["backdrop"] = f"chapters/{chapter_dir.name}/{rel}"
        made += 1
    doc_path.write_text(json.dumps(doc, ensure_ascii=False))
    return made


if __name__ == "__main__":
    print(backfill(Path(sys.argv[1])), "backdrops")
