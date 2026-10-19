import { AbsoluteFill, random } from "remotion";
import { wrap, type AmbientClock } from "./types";

/** Night-street light colours: sodium lamps, tail lights, neon signs, headlights. */
const PALETTE = ["255, 178, 74", "255, 76, 92", "84, 206, 255", "255, 98, 206", "255, 238, 205"] as const;

export interface BokehLight {
  /** Centre of the disc. */
  x: number;
  y: number;
  size: number;
  /** "r, g, b" */
  color: string;
  opacity: number;
  blur: number;
}

/**
 * Out-of-focus city lights: coloured discs at three apparent depths drifting sideways in both
 * directions like passing traffic, bobbing gently and pulsing. Nearer lights are bigger, blurrier,
 * brighter and faster. Lights wrap around off screen.
 */
export function bokehLights({ frame, fps, width, height }: AmbientClock, count = 28, seed = "city"): BokehLight[] {
  const u = height / 1080;
  const t = frame / fps;
  return Array.from({ length: count }, (_, i) => {
    const r = (k: string) => random(`${seed}:${i}:${k}`);
    const depth = r("depth");
    const size = (40 + depth * 130) * u;
    const direction = r("direction") < 0.5 ? 1 : -1;
    const speed = (10 + r("speed") * 35) * (0.5 + depth) * u;
    const span = width + size * 2;
    const phase = r("phase") * Math.PI * 2;
    const bobPeriod = 6 + r("bob") * 6;
    const pulsePeriod = 2 + r("pulse") * 3;
    return {
      x: wrap(r("x") * span + direction * speed * t, span) - size,
      y: height * (0.22 + r("y") * 0.72) + Math.sin((t / bobPeriod) * Math.PI * 2 + phase) * 12 * u,
      size,
      color: PALETTE[Math.floor(r("color") * PALETTE.length)]!,
      opacity: (0.16 + depth * 0.3) * (0.75 + 0.25 * Math.sin((t / pulsePeriod) * Math.PI * 2 + phase)),
      blur: (4 + depth * 14) * u,
    };
  });
}

export function CityBokeh(clock: AmbientClock) {
  return (
    <AbsoluteFill style={{ pointerEvents: "none", mixBlendMode: "screen" }}>
      {bokehLights(clock).map((l, i) => (
        <div
          key={i}
          style={{
            position: "absolute",
            left: l.x - l.size / 2,
            top: l.y - l.size / 2,
            width: l.size,
            height: l.size,
            borderRadius: "50%",
            // Brighter rim like a real lens bokeh disc.
            background: `radial-gradient(circle, rgba(${l.color}, 0.35) 0%, rgba(${l.color}, 0.5) 55%, rgba(${l.color}, 0.8) 66%, rgba(${l.color}, 0) 71%)`,
            opacity: l.opacity,
            filter: `blur(${l.blur}px)`,
          }}
        />
      ))}
    </AbsoluteFill>
  );
}
