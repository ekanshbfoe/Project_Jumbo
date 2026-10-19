import { useMemo } from "react";
import { AbsoluteFill, random } from "remotion";

export interface AmbientDustProps {
  /** Frame on the episode's clock, so the dust keeps flowing across beat cuts instead of restarting. */
  frame: number;
  fps: number;
  width: number;
  height: number;
  count?: number;
  seed?: string;
}

interface Mote {
  x: number;
  y: number;
  depth: number;
  rise: number;
  sway: number;
  swayPeriod: number;
  twinklePeriod: number;
  phase: number;
}

/**
 * Slow floating dust: deterministic motes at three apparent depths that rise, sway and twinkle.
 * Far motes are small and faint; near ones are larger, brighter and softly out of focus.
 */
export function AmbientDust({ frame, fps, width, height, count = 48, seed = "dust" }: AmbientDustProps) {
  const u = height / 1080;
  const motes = useMemo<Mote[]>(
    () =>
      Array.from({ length: count }, (_, i) => {
        const r = (k: string) => random(`${seed}:${i}:${k}`);
        return {
          x: r("x"),
          y: r("y"),
          depth: r("depth"),
          rise: 6 + r("rise") * 10,
          sway: 10 + r("sway") * 24,
          swayPeriod: 7 + r("swayPeriod") * 9,
          twinklePeriod: 3 + r("twinkle") * 5,
          phase: r("phase") * Math.PI * 2,
        };
      }),
    [count, seed],
  );
  const t = frame / fps;
  const pad = 60 * u;
  const span = height + pad * 2;

  return (
    <AbsoluteFill style={{ pointerEvents: "none", mixBlendMode: "screen" }}>
      {motes.map((m, i) => {
        const size = (1.6 + m.depth * 6) * u;
        const speed = (m.rise + m.depth * 18) * u;
        const y = ((((m.y * span - t * speed) % span) + span) % span) - pad;
        const x = m.x * width + Math.sin((t / m.swayPeriod) * Math.PI * 2 + m.phase) * m.sway * u;
        const twinkle = 0.6 + 0.4 * Math.sin((t / m.twinklePeriod) * Math.PI * 2 + m.phase * 1.7);
        const blur = m.depth > 0.75 ? (m.depth - 0.75) * 14 * u : 0;
        return (
          <div
            key={i}
            style={{
              position: "absolute",
              left: x - size / 2,
              top: y - size / 2,
              width: size,
              height: size,
              borderRadius: "50%",
              background: "radial-gradient(circle, rgba(255, 246, 228, 1) 0%, rgba(255, 236, 205, 0.55) 45%, rgba(255, 230, 200, 0) 72%)",
              opacity: (0.18 + m.depth * 0.5) * twinkle,
              filter: blur > 0 ? `blur(${blur}px)` : undefined,
            }}
          />
        );
      })}
    </AbsoluteFill>
  );
}
