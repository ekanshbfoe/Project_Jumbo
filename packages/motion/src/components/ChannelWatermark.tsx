import { loadFont } from "@remotion/google-fonts/Inter";
import { AbsoluteFill, useVideoConfig } from "remotion";

const { fontFamily } = loadFont("normal", { weights: ["600"], subsets: ["latin"] });

/** Space the watermark takes above the bottom edge (margin, mark and a gap), in 1080p pixels. */
export const WATERMARK_RESERVED_HEIGHT = 76;

export interface ChannelWatermarkProps {
  text: string;
  /** 0-1; kept low so it never competes with the panels. */
  opacity: number;
}

/** Logo placeholder: the slanted-panel motif from the app icon, as an outline. */
function Mark({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M3.5 4.5h17v11.2L3.5 19.5z" stroke="#ffffff" strokeWidth={2} strokeLinejoin="round" />
      <path d="M7 15.5L17 8.5" stroke="#ffffff" strokeWidth={2} strokeLinecap="round" />
    </svg>
  );
}

/** Permanent, semi-translucent channel mark anchored to the bottom-right corner. */
export function ChannelWatermark({ text, opacity }: ChannelWatermarkProps) {
  const { height } = useVideoConfig();
  const u = height / 1080;
  return (
    <AbsoluteFill style={{ pointerEvents: "none" }}>
      <div
        style={{
          position: "absolute",
          right: 40 * u,
          bottom: 36 * u,
          display: "flex",
          alignItems: "center",
          gap: 10 * u,
          opacity,
          fontFamily,
          fontWeight: 600,
          fontSize: 20 * u,
          letterSpacing: 3.5 * u,
          textTransform: "uppercase",
          color: "#ffffff",
          textShadow: `0 ${1 * u}px ${6 * u}px rgba(0, 0, 0, 0.6)`,
          filter: `drop-shadow(0 ${1 * u}px ${4 * u}px rgba(0, 0, 0, 0.5))`,
        }}
      >
        <Mark size={26 * u} />
        {text}
      </div>
    </AbsoluteFill>
  );
}
