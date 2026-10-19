import type { Beat, EpisodePanel, PresetName } from "@jumbo/schema";
import { AbsoluteFill, Html5Audio, Img, useCurrentFrame, useVideoConfig } from "remotion";
import { useAssetResolver } from "../assets";
import { fitContain, panelSize, splitFrame, type Rect } from "../geometry";
import { PRESETS } from "../presets/registry";
import type { PresetContext, PresetImpl } from "../presets/types";
import { Camera } from "./Camera";
import { PolygonPanel } from "./PolygonPanel";

export interface BeatViewProps {
  beat: Beat;
  panels: ReadonlyMap<string, EpisodePanel>;
  /** Passed explicitly because the beat's Sequence length is what presets animate over. */
  durationInFrames: number;
  hideBubbles: boolean;
}

/** Where each panel of the beat sits before the camera moves. */
function layout(beat: Beat, panels: EpisodePanel[], width: number, height: number): Rect[] {
  if (panels.length === 1) return [fitContain(panelSize(panels[0]!), { width, height }, 0.06)];
  const orientation = beat.preset.name === "split_screen" ? beat.preset.params.orientation : "vertical";
  const inset = { left: width * 0.04, top: height * 0.06 };
  const slots = splitFrame({ width: width * 0.92, height: height * 0.88 }, panels.length, orientation, height * 0.025);
  return slots.map((slot, i) => {
    const fit = fitContain(panelSize(panels[i]!), slot, 0.02);
    return { ...fit, left: inset.left + slot.left + fit.left, top: inset.top + slot.top + fit.top };
  });
}

/** One narration beat: blurred backdrop, camera-driven panels, preset overlay and audio. */
export function BeatView({ beat, panels: panelMap, durationInFrames, hideBubbles }: BeatViewProps) {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const resolve = useAssetResolver();
  const panels = beat.panel_ids.map((id) => panelMap.get(id)).filter((p): p is EpisodePanel => p !== undefined);
  const impl = PRESETS[beat.preset.name] as PresetImpl<PresetName>;
  const rects = layout(beat, panels, width, height);
  const ctx = (index: number): PresetContext => ({
    frame,
    durationInFrames,
    fps,
    frameSize: { width, height },
    seed: `${beat.line_id}:${index}`,
    index,
    count: panels.length,
  });
  const backdrop = panels[0] && (panels[0].clean ?? panels[0].crop ?? panels[0].page);

  return (
    <AbsoluteFill>
      {backdrop ? (
        <Img
          src={resolve(backdrop)}
          style={{
            position: "absolute",
            inset: "-10%",
            width: "120%",
            height: "120%",
            objectFit: "cover",
            filter: "blur(40px) brightness(0.4) saturate(1.2)",
          }}
        />
      ) : null}
      {panels.map((panel, i) => (
        <Camera key={panel.id} transform={impl.camera(ctx(i), beat.preset.params as never)} {...rects[i]!}>
          <PolygonPanel panel={panel} width={rects[i]!.width} height={rects[i]!.height} hideBubbles={hideBubbles} />
        </Camera>
      ))}
      {impl.Overlay ? <impl.Overlay ctx={ctx(0)} params={beat.preset.params as never} /> : null}
      {beat.audio ? <Html5Audio src={resolve(beat.audio.src)} /> : null}
    </AbsoluteFill>
  );
}
