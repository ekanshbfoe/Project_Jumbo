import { interpolate, spring } from "remotion";
import type { PresetOverlayProps } from "./types";

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

/** Game-system style message box that pops in and types its text on. */
export function HudPopupOverlay({ ctx, params }: PresetOverlayProps<"hud_popup">) {
  const { frame, fps, frameSize } = ctx;
  const u = frameSize.height / 1080;
  const appear = Math.round(0.15 * fps);
  const pop = spring({ frame: Math.max(0, frame - appear), fps, config: { damping: 12, stiffness: 160 } });
  const typeStart = appear + Math.round(0.2 * fps);
  const chars = Math.floor(
    interpolate(frame, [typeStart, typeStart + params.text.length * fps * 0.035 + 1], [0, params.text.length], clamp),
  );
  const cursorOn = Math.floor(frame / (fps * 0.25)) % 2 === 0;
  const top = { top: "10%", center: "50%", bottom: "78%" }[params.position];

  return (
    <div
      style={{
        position: "absolute",
        left: "50%",
        top,
        transform: `translate(-50%, -50%) scale(${interpolate(pop, [0, 1], [0.6, 1])})`,
        opacity: interpolate(pop, [0, 0.3], [0, 1], clamp),
        minWidth: 520 * u,
        maxWidth: frameSize.width * 0.7,
        padding: `${22 * u}px ${36 * u}px`,
        background: "linear-gradient(180deg, rgba(8, 16, 32, 0.92), rgba(4, 10, 22, 0.88))",
        border: `${2 * u}px solid ${params.accent}`,
        borderRadius: 6 * u,
        boxShadow: `0 0 ${28 * u}px ${params.accent}88, inset 0 0 ${18 * u}px ${params.accent}33`,
        color: "#e8f7ff",
        fontFamily: "'JetBrains Mono', 'Consolas', 'Courier New', monospace",
        fontSize: 40 * u,
        fontWeight: 700,
        letterSpacing: 2 * u,
        textTransform: "uppercase",
        textAlign: "center",
        textShadow: `0 0 ${10 * u}px ${params.accent}`,
      }}
    >
      <div style={{ fontSize: 18 * u, letterSpacing: 6 * u, color: params.accent, marginBottom: 8 * u }}>[ SYSTEM ]</div>
      {params.text.slice(0, chars)}
      <span style={{ opacity: cursorOn ? 1 : 0, color: params.accent }}>▌</span>
    </div>
  );
}
