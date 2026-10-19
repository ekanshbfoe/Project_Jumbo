import { AbsoluteFill, random } from "remotion";
import { wrap, type AmbientClock } from "./types";

export interface DustMote {
  x: number;
  y: number;
  size: number;
  opacity: number;
  blur: number;
}

/**
 * Slow floating dust: motes at three apparent depths that rise, sway and twinkle. Far motes are small
 * and faint; near ones are larger, brighter and softly out of focus. Motes wrap around off screen.
 */
export function dustMotes({ frame, fps, width, height }: AmbientClock, count = 48, seed = "dust"): DustMote[] {
  const u = height / 1080;
  const t = frame / fps;
  const pad = 60 * u;
  const span = height + pad * 2;
  return Array.from({ length: count }, (_, i) => {
    const r = (k: string) => random(`${seed}:${i}:${k}`);
    const depth = r("depth");
    const phase = r("phase") * Math.PI * 2;
    const size = (1.6 + depth * 6) * u;
    const speed = (6 + r("rise") * 10 + depth * 18) * u;
    const sway = (10 + r("sway") * 24) * u;
    const swayPeriod = 7 + r("swayPeriod") * 9;
    const twinklePeriod = 3 + r("twinkle") * 5;
    const twinkle = 0.6 + 0.4 * Math.sin((t / twinklePeriod) * Math.PI * 2 + phase * 1.7);
    return {
      x: r("x") * width + Math.sin((t / swayPeriod) * Math.PI * 2 + phase) * sway,
      y: wrap(r("y") * span - t * speed, span) - pad,
      size,
      opacity: (0.18 + depth * 0.5) * twinkle,
      blur: depth > 0.75 ? (depth - 0.75) * 14 * u : 0,
    };
  });
}

export function AmbientDust(clock: AmbientClock) {
  return (
    <AbsoluteFill style={{ pointerEvents: "none", mixBlendMode: "screen" }}>
      {dustMotes(clock).map((m, i) => (
        <div
          key={i}
          style={{
            position: "absolute",
            left: m.x - m.size / 2,
            top: m.y - m.size / 2,
            width: m.size,
            height: m.size,
            borderRadius: "50%",
            background:
              "radial-gradient(circle, rgba(255, 246, 228, 1) 0%, rgba(255, 236, 205, 0.55) 45%, rgba(255, 230, 200, 0) 72%)",
            opacity: m.opacity,
            filter: m.blur > 0 ? `blur(${m.blur}px)` : undefined,
          }}
        />
      ))}
    </AbsoluteFill>
  );
}
