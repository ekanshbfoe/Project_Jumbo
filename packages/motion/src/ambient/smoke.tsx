import { AbsoluteFill, random } from "remotion";
import { wrap, type AmbientClock } from "./types";

export interface SmokePuff {
  /** Centre of the puff. */
  x: number;
  y: number;
  width: number;
  height: number;
  rotate: number;
  opacity: number;
  blur: number;
}

/**
 * Slow, wispy rising haze. Each puff lives on its own long cycle: it appears below the frame, swells,
 * sways and turns as it climbs, and thins to nothing near the top, so the loop never pops.
 */
export function smokePuffs({ frame, fps, width, height }: AmbientClock, count = 14, seed = "smoke"): SmokePuff[] {
  const u = height / 1080;
  const t = frame / fps;
  return Array.from({ length: count }, (_, i) => {
    const r = (k: string) => random(`${seed}:${i}:${k}`);
    const period = 16 + r("period") * 14;
    const age = wrap(t + r("offset") * period, period) / period;
    const base = (260 + r("size") * 360) * u;
    const grow = 0.7 + age * 0.9;
    const swayPeriod = 8 + r("swayPeriod") * 6;
    return {
      x: r("x") * width + (r("drift") - 0.5) * width * 0.25 * age + Math.sin((t / swayPeriod) * Math.PI * 2 + r("phase") * 6) * 40 * u,
      y: height * (1.15 - age * 0.95),
      width: base * grow * 1.7,
      height: base * grow,
      rotate: (r("tilt") - 0.5) * 40 + age * 25,
      // sin(pi * age) is zero at both ends of the cycle, where the puff restarts below the frame.
      opacity: Math.sin(Math.PI * age) * (0.18 + r("opacity") * 0.17),
      blur: (38 + r("blur") * 30) * u,
    };
  });
}

export function AmbientSmoke(clock: AmbientClock) {
  const t = clock.frame / clock.fps;
  return (
    <AbsoluteFill style={{ pointerEvents: "none", mixBlendMode: "screen" }}>
      {/* Low fog bank that breathes slowly. */}
      <div
        style={{
          position: "absolute",
          inset: 0,
          background: "linear-gradient(0deg, rgba(196, 202, 218, 0.28) 0%, rgba(196, 202, 218, 0.1) 32%, transparent 60%)",
          opacity: 0.8 + 0.2 * Math.sin((t / 11) * Math.PI * 2),
        }}
      />
      {smokePuffs(clock).map((p, i) => (
        <div
          key={i}
          style={{
            position: "absolute",
            left: p.x - p.width / 2,
            top: p.y - p.height / 2,
            width: p.width,
            height: p.height,
            transform: `rotate(${p.rotate}deg)`,
            borderRadius: "50%",
            background: "radial-gradient(ellipse at center, rgba(214, 220, 232, 1) 0%, rgba(214, 220, 232, 0.45) 40%, rgba(214, 220, 232, 0) 70%)",
            opacity: p.opacity,
            filter: `blur(${p.blur}px)`,
          }}
        />
      ))}
    </AbsoluteFill>
  );
}
