import type { PromoPosition, SocialPromoSettings } from "@jumbo/schema";
import { loadFont } from "@remotion/google-fonts/Inter";
import type { CSSProperties } from "react";
import { AbsoluteFill, Easing, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { PLATFORM_LABELS, PLATFORM_TILES, PlatformGlyph } from "./SocialIcons";

const { fontFamily } = loadFont("normal", { weights: ["500", "600", "800"], subsets: ["latin"] });

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

/** Seconds the exit animation takes; it ends exactly on the promo's last frame. */
export const PROMO_EXIT_SECONDS = 0.75;
/** Seconds until the entry spring has settled. Promos shorter than entry + exit are not shown. */
export const PROMO_ENTRY_SECONDS = 1.2;
/** A light sweep crosses the glass this often while the card is idle. */
const SWEEP_PERIOD_SECONDS = 7;

export interface SocialPromoProps {
  settings: Pick<SocialPromoSettings, "username" | "headline" | "platforms" | "position">;
  /** Length of the promo's Sequence; the exit animation finishes on its last frame. */
  durationInFrames: number;
  /** Extra space kept clear below the card in bottom placements (1080p pixels), e.g. for a watermark. */
  bottomClearance?: number;
}

function placement(position: PromoPosition, margin: number, bottom: number): CSSProperties {
  switch (position) {
    case "bottom-right":
      return { right: margin, bottom };
    case "bottom-left":
      return { left: margin, bottom };
    case "top-right":
      return { right: margin, top: margin };
    case "top-left":
      return { left: margin, top: margin };
    case "center":
      return { left: "50%", top: "50%", translate: "-50% -50%" };
  }
}

/**
 * Premium follow-us card: dark frosted glass that flips in on a snappy spring, cascades its contents,
 * floats while visible with a periodic light sweep, and flips away at the end of its Sequence.
 */
export function SocialPromo({ settings, durationInFrames, bottomClearance = 0 }: SocialPromoProps) {
  const frame = useCurrentFrame();
  const { fps, height } = useVideoConfig();
  const u = height / 1080;
  const s = (seconds: number) => Math.round(seconds * fps);
  const delayed = (delay: number, config = { damping: 14, stiffness: 190, mass: 0.7 }) =>
    spring({ frame: Math.max(0, frame - s(delay)), fps, config });

  // Entry: 3D flip up from below with an aggressive scale-in and a small overshoot.
  const enter = spring({ frame, fps, config: { damping: 13, stiffness: 150, mass: 0.9 } });
  // Exit: a short anticipation swell, then flip back and away with motion blur.
  const exitStart = Math.max(s(PROMO_ENTRY_SECONDS) / 2, durationInFrames - s(PROMO_EXIT_SECONDS));
  const exit = interpolate(frame, [exitStart, durationInFrames - 1], [0, 1], {
    ...clamp,
    easing: Easing.bezier(0.55, 0, 0.75, 0.1),
  });

  const t = frame / fps;
  const float = Math.sin((t / 5) * Math.PI * 2) * enter;
  const card: CSSProperties = {
    transform: [
      `perspective(${1600 * u}px)`,
      `translateY(${(1 - enter) * 160 * u + float * 5 * u + exit * 90 * u}px)`,
      `rotateX(${interpolate(enter, [0, 1], [-95, 0]) + float * 1.2 + exit * 70}deg)`,
      `rotateY(${interpolate(enter, [0, 1], [-24, 0])}deg)`,
      `scale(${interpolate(enter, [0, 1], [0.55, 1]) * interpolate(exit, [0, 0.25, 1], [1, 1.04, 0.6])})`,
    ].join(" "),
    opacity: interpolate(enter, [0, 0.3], [0, 1], clamp) * interpolate(exit, [0.35, 1], [1, 0], clamp),
    filter: exit > 0 ? `blur(${exit * 14 * u}px)` : undefined,
  };

  // First sweep right after landing, then one every SWEEP_PERIOD_SECONDS.
  const sweepFrames = s(0.9);
  const firstSweep = s(0.45);
  const sinceSweep = frame < firstSweep ? -1 : (frame - firstSweep) % s(SWEEP_PERIOD_SECONDS);
  const sweep = sinceSweep >= 0 && sinceSweep < sweepFrames ? sinceSweep / sweepFrames : null;

  const accent = delayed(0.2);
  const handle = delayed(0.18, { damping: 16, stiffness: 170, mass: 0.8 });
  const divider = delayed(0.3, { damping: 20, stiffness: 120, mass: 1 });

  return (
    <AbsoluteFill style={{ pointerEvents: "none" }}>
      <div style={{ position: "absolute", width: 620 * u, fontFamily, ...placement(settings.position, 64 * u, Math.max(64, bottomClearance) * u) }}>
        {/* Ambient glow behind the glass. */}
        <div
          style={{
            position: "absolute",
            inset: `${-40 * u}px`,
            borderRadius: 60 * u,
            background: "radial-gradient(ellipse at 30% 40%, rgba(124, 92, 255, 0.45), rgba(58, 214, 255, 0.12) 55%, transparent 75%)",
            filter: `blur(${36 * u}px)`,
            opacity: enter * (1 - exit),
          }}
        />
        <div
          style={{
            position: "relative",
            overflow: "hidden",
            borderRadius: 30 * u,
            padding: `${28 * u}px ${34 * u}px ${30 * u}px`,
            background: "linear-gradient(150deg, rgba(40, 40, 58, 0.78) 0%, rgba(16, 16, 26, 0.82) 55%, rgba(10, 10, 18, 0.88) 100%)",
            backdropFilter: `blur(${26 * u}px) saturate(160%)`,
            WebkitBackdropFilter: `blur(${26 * u}px) saturate(160%)`,
            border: `${1.2 * u}px solid rgba(255, 255, 255, 0.16)`,
            boxShadow: [
              `inset 0 ${1.5 * u}px 0 rgba(255, 255, 255, 0.28)`,
              `inset 0 ${-1 * u}px 0 rgba(0, 0, 0, 0.4)`,
              `0 ${40 * u}px ${90 * u}px ${-20 * u}px rgba(0, 0, 0, 0.75)`,
              `0 0 0 ${1 * u}px rgba(124, 92, 255, 0.18)`,
            ].join(", "),
            color: "#ffffff",
            transformStyle: "preserve-3d",
            ...card,
          }}
        >
          {sweep !== null ? (
            <div
              style={{
                position: "absolute",
                top: "-50%",
                bottom: "-50%",
                width: "40%",
                left: `${interpolate(sweep, [0, 1], [-60, 160], { easing: Easing.inOut(Easing.cubic) })}%`,
                transform: "rotate(18deg)",
                background: "linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.16), transparent)",
              }}
            />
          ) : null}

          <div style={{ display: "flex", alignItems: "center", gap: 14 * u }}>
            <div
              style={{
                width: 34 * u * accent,
                height: 4 * u,
                borderRadius: 4 * u,
                background: "linear-gradient(90deg, #7c5cff, #3ad6ff)",
                boxShadow: `0 0 ${12 * u}px rgba(124, 92, 255, 0.8)`,
              }}
            />
            <div
              style={{
                fontSize: 15 * u,
                fontWeight: 600,
                letterSpacing: 4 * u,
                textTransform: "uppercase",
                color: "rgba(255, 255, 255, 0.62)",
                opacity: accent,
              }}
            >
              {settings.headline}
            </div>
          </div>

          <div style={{ overflow: "hidden", marginTop: 10 * u, paddingBottom: 4 * u }}>
            <div
              style={{
                fontSize: 50 * u,
                fontWeight: 800,
                letterSpacing: -0.8 * u,
                lineHeight: 1.1,
                transform: `translateY(${(1 - handle) * 110}%)`,
                background: "linear-gradient(100deg, #ffffff 30%, #cbbfff 70%, #8fe6ff 100%)",
                WebkitBackgroundClip: "text",
                backgroundClip: "text",
                color: "transparent",
              }}
            >
              @{settings.username}
            </div>
          </div>

          <div
            style={{
              height: 1.2 * u,
              margin: `${18 * u}px 0 ${20 * u}px`,
              transformOrigin: "left center",
              transform: `scaleX(${divider})`,
              background: "linear-gradient(90deg, rgba(255, 255, 255, 0.3), rgba(255, 255, 255, 0.04))",
            }}
          />

          <div style={{ display: "flex", gap: 14 * u, perspective: 900 * u }}>
            {settings.platforms.map((platform, i) => {
              const pop = delayed(0.38 + i * 0.08, { damping: 12, stiffness: 210, mass: 0.6 });
              return (
                <div
                  key={platform}
                  style={{
                    flex: 1,
                    display: "flex",
                    alignItems: "center",
                    gap: 12 * u,
                    padding: `${10 * u}px ${12 * u}px`,
                    borderRadius: 18 * u,
                    background: "rgba(255, 255, 255, 0.06)",
                    border: `${1 * u}px solid rgba(255, 255, 255, 0.1)`,
                    opacity: interpolate(pop, [0, 0.4], [0, 1], clamp),
                    transform: `translateY(${(1 - pop) * 34 * u}px) rotateY(${(1 - pop) * -70}deg) scale(${0.8 + 0.2 * pop})`,
                  }}
                >
                  <div
                    style={{
                      width: 46 * u,
                      height: 46 * u,
                      flexShrink: 0,
                      borderRadius: 13 * u,
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      boxShadow: `0 ${6 * u}px ${16 * u}px rgba(0, 0, 0, 0.45)`,
                      ...PLATFORM_TILES[platform],
                    }}
                  >
                    <PlatformGlyph platform={platform} size={platform === "x" ? 22 * u : 28 * u} />
                  </div>
                  <div style={{ fontSize: 18 * u, fontWeight: 600, color: "rgba(255, 255, 255, 0.92)" }}>
                    {PLATFORM_LABELS[platform]}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </AbsoluteFill>
  );
}
