import type { CharacterCard as Card } from "@jumbo/schema";
import { loadFont as loadBarlow } from "@remotion/google-fonts/BarlowSemiCondensed";
import { loadFont as loadInter } from "@remotion/google-fonts/Inter";
import { AbsoluteFill, Easing, interpolate, useCurrentFrame, useVideoConfig } from "remotion";

const { fontFamily: nameFont } = loadBarlow("normal", { weights: ["600"], subsets: ["latin"] });
const { fontFamily: roleFont } = loadInter("normal", { weights: ["500"], subsets: ["latin"] });

/** How long a card is on screen, entry and exit included. */
export const CARD_SECONDS = 3;
const GOLD = "#E8B04B";
const OUT = Easing.bezier(0.16, 1, 0.3, 1);
const IN = Easing.bezier(0.7, 0, 0.84, 0);

export interface CharacterCardProps {
  card: Card;
  durationInFrames: number;
  /** Top-left instead of bottom-left, when something else holds the bottom-left corner. */
  top?: boolean;
}

/**
 * A character's name card on their first appearance in the series (ADR 0007, option C): a soft dark gradient
 * instead of a plate, a thin gold bar, the name in Barlow Semi Condensed over the role in spaced uppercase Inter.
 * The bar grows, the name wipes in, the role rises a beat later; all of it slides out together. It sits on the
 * blurred side of the frame inside the 5% safe area, clear of the watermark in the bottom-right corner.
 */
export function CharacterCardView({ card, durationInFrames, top = false }: CharacterCardProps) {
  const frame = useCurrentFrame();
  const { fps, height } = useVideoConfig();
  const u = height / 1080;
  const f = (seconds: number) => seconds * fps;
  const span = (from: number, to: number, ease = OUT) =>
    interpolate(frame, [f(from), f(to)], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: ease });
  const exitStart = durationInFrames - f(0.53);
  const exit = interpolate(frame, [exitStart, durationInFrames], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: IN });
  const bar = span(0, 0.27);
  const name = span(0.13, 0.66);
  const role = span(0.4, 0.87);
  const scrim = span(0, 0.5);
  // A long name shrinks to stay on one line inside the side of the frame.
  const nameSize = 76 * u * Math.min(1, 18 / Math.max(18, card.name.length));
  const edge = top ? { top: 96 * u } : { bottom: 100 * u };

  return (
    <AbsoluteFill style={{ pointerEvents: "none", opacity: 1 - exit, transform: `translateX(${-16 * u * exit}px)` }}>
      <div
        style={{
          position: "absolute",
          left: 0,
          ...(top ? { top: 36 * u } : { bottom: 40 * u }),
          width: 900 * u,
          height: 260 * u,
          opacity: scrim,
          background: "linear-gradient(90deg, rgba(8,9,12,0.75) 0%, rgba(8,9,12,0.55) 45%, rgba(8,9,12,0) 100%)",
          WebkitMaskImage: "linear-gradient(0deg, transparent, #000 22%, #000 78%, transparent)",
          maskImage: "linear-gradient(0deg, transparent, #000 22%, #000 78%, transparent)",
        }}
      />
      <div style={{ position: "absolute", left: 96 * u, ...edge, display: "flex", gap: 20 * u }}>
        <div style={{ width: 4 * u, background: GOLD, transformOrigin: top ? "top" : "bottom", transform: `scaleY(${bar})` }} />
        <div>
          <div
            style={{
              fontFamily: nameFont,
              fontWeight: 600,
              fontSize: nameSize,
              lineHeight: 1,
              color: "#FFFFFF",
              letterSpacing: "0.01em",
              whiteSpace: "nowrap",
              textShadow: `0 ${2 * u}px ${12 * u}px rgba(0,0,0,0.5)`,
              clipPath: `inset(0 ${100 - 100 * name}% 0 0)`,
              transform: `translateX(${-24 * u * (1 - name)}px)`,
            }}
          >
            {card.name}
          </div>
          {card.role ? (
            <div
              style={{
                fontFamily: roleFont,
                fontWeight: 500,
                fontSize: 28 * u,
                lineHeight: 1.35,
                color: "#C9CED6",
                textTransform: "uppercase",
                letterSpacing: "0.12em",
                marginTop: 14 * u,
                maxWidth: 600 * u,
                textWrap: "balance",
                opacity: role,
                transform: `translateY(${10 * u * (1 - role)}px)`,
              }}
            >
              {card.role}
            </div>
          ) : null}
        </div>
      </div>
    </AbsoluteFill>
  );
}
