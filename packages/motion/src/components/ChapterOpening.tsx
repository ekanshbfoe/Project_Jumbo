import type { EpisodePanel, OpeningCard } from "@jumbo/schema";
import { loadFont } from "@remotion/google-fonts/Inter";
import { AbsoluteFill, Img, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { useAssetResolver } from "../assets";

const { fontFamily } = loadFont("normal", { weights: ["600", "800"], subsets: ["latin"] });

export interface ChapterOpeningProps {
  card: OpeningCard;
  /** The first panel the chapter shows, blurred behind the title. */
  panel?: EpisodePanel;
  durationInFrames: number;
}

/** The silent title card an episode part opens with: the chapter's title over its first panel, faded in and out. */
export function ChapterOpening({ card, panel, durationInFrames }: ChapterOpeningProps) {
  const frame = useCurrentFrame();
  const { fps, height } = useVideoConfig();
  const resolve = useAssetResolver();
  const u = height / 1080;
  const fade = Math.min(Math.round(0.4 * fps), Math.floor(durationInFrames / 3));
  const opacity = interpolate(frame, [0, fade, durationInFrames - fade, durationInFrames], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const rise = interpolate(frame, [0, durationInFrames], [12 * u, 0]);
  const image = panel ? (panel.backdrop ?? panel.clean ?? panel.crop ?? panel.page) : undefined;
  return (
    <AbsoluteFill style={{ opacity }}>
      {image ? (
        <Img
          src={resolve(image)}
          style={{ position: "absolute", inset: -40 * u, width: `calc(100% + ${80 * u}px)`, height: `calc(100% + ${80 * u}px)`, objectFit: "cover", filter: `blur(${24 * u}px) brightness(0.45)` }}
        />
      ) : null}
      <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", fontFamily, color: "#ffffff", textAlign: "center", padding: `0 ${160 * u}px` }}>
        <div style={{ transform: `translateY(${rise}px)`, textShadow: `0 ${2 * u}px ${18 * u}px rgba(0, 0, 0, 0.7)` }}>
          <div style={{ fontWeight: 800, fontSize: 84 * u, lineHeight: 1.1 }}>{card.title}</div>
          {card.subtitle ? <div style={{ fontWeight: 600, fontSize: 32 * u, marginTop: 18 * u, letterSpacing: 4 * u, textTransform: "uppercase", opacity: 0.85 }}>{card.subtitle}</div> : null}
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
}
