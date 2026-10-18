import { interpolate } from "remotion";

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

export interface CaptionsProps {
  text: string;
  frame: number;
  durationInFrames: number;
  fps: number;
  /** Seconds the narration is spoken for. Words are highlighted evenly across it. */
  spokenFor: number;
  height: number;
}

/**
 * Burned-in narration caption with a karaoke-style highlight. Without word timestamps from the voice
 * engine, the highlight assumes evenly paced words.
 */
export function Captions({ text, frame, durationInFrames, fps, spokenFor, height }: CaptionsProps) {
  const u = height / 1080;
  const words = text.split(/\s+/).filter(Boolean);
  const fadeFrames = Math.min(Math.round(0.2 * fps), Math.floor(durationInFrames / 2));
  const opacity =
    fadeFrames > 0
      ? interpolate(frame, [0, fadeFrames, durationInFrames - fadeFrames, durationInFrames], [0, 1, 1, 0], clamp)
      : 1;
  const spoken = interpolate(frame, [0, Math.max(1, spokenFor * fps)], [0, words.length], clamp);

  return (
    <div
      style={{
        position: "absolute",
        left: "8%",
        right: "8%",
        bottom: 56 * u,
        display: "flex",
        justifyContent: "center",
        opacity,
      }}
    >
      <div
        style={{
          padding: `${14 * u}px ${28 * u}px`,
          borderRadius: 14 * u,
          background: "rgba(0, 0, 0, 0.62)",
          fontFamily: "'Poppins', 'Segoe UI', 'Noto Sans', sans-serif",
          fontSize: 46 * u,
          fontWeight: 700,
          lineHeight: 1.25,
          textAlign: "center",
          color: "rgba(255, 255, 255, 0.55)",
          textShadow: `0 ${2 * u}px ${6 * u}px rgba(0, 0, 0, 0.8)`,
        }}
      >
        {words.map((word, i) => (
          <span key={i} style={{ color: i < spoken ? "#ffffff" : undefined }}>
            {word}
            {i < words.length - 1 ? " " : ""}
          </span>
        ))}
      </div>
    </div>
  );
}
