import type { AmbientEffect, Beat, EpisodePanel, PresetName } from "@jumbo/schema";
import { AbsoluteFill, Html5Audio, Img, useCurrentFrame, useVideoConfig } from "remotion";
import { AmbientLayer } from "../ambient/AmbientLayer";
import { useAssetResolver } from "../assets";
import { fitContain, panelSize, splitFrame } from "../geometry";
import { capUpscale, keepInFrame } from "../framing";
import { PRESETS } from "../presets/registry";
import type { PanelSlot, PresetContext, PresetImpl } from "../presets/types";
import { Camera } from "./Camera";
import { PolygonPanel } from "./PolygonPanel";

export interface BeatViewProps {
  beat: Beat;
  panels: ReadonlyMap<string, EpisodePanel>;
  /** Passed explicitly because the beat's Sequence length is what presets animate over. */
  durationInFrames: number;
  hideBubbles: boolean;
  /** Motion layered over the blurred backdrop so static panels never sit on a dead background. */
  ambientEffect?: AmbientEffect;
  /** The beat's first frame on the episode clock, which keeps ambient motion continuous across beats. */
  startFrame?: number;
}

/** Where each panel of the beat sits before the camera moves, unless the preset lays them out itself. */
function layout(beat: Beat, panels: EpisodePanel[], width: number, height: number, impl: PresetImpl<PresetName>): PanelSlot[] {
  if (impl.layout) return impl.layout(panels.map(panelSize), { width, height }, beat.preset.params as never);
  // Never larger on screen than MAX_UPSCALE times the panel's own pixels: a small scan is shown smaller, not mushy.
  if (panels.length === 1) return [capUpscale(fitContain(panelSize(panels[0]!), { width, height }, 0.06), panelSize(panels[0]!))];
  const orientation = beat.preset.name === "split_screen" ? beat.preset.params.orientation : "vertical";
  const inset = { left: width * 0.04, top: height * 0.06 };
  const slots = splitFrame({ width: width * 0.92, height: height * 0.88 }, panels.length, orientation, height * 0.025);
  return slots.map((slot, i) => {
    const fit = capUpscale(fitContain(panelSize(panels[i]!), slot, 0.02), panelSize(panels[i]!));
    return { ...fit, left: inset.left + slot.left + fit.left, top: inset.top + slot.top + fit.top };
  });
}

/** One narration beat: blurred backdrop, ambient motion, camera-driven panels, preset overlay and audio. */
export function BeatView({
  beat,
  panels: panelMap,
  durationInFrames,
  hideBubbles,
  ambientEffect = "dust",
  startFrame = 0,
}: BeatViewProps) {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const resolve = useAssetResolver();
  const panels = beat.panel_ids.map((id) => panelMap.get(id)).filter((p): p is EpisodePanel => p !== undefined);
  const impl = PRESETS[beat.preset.name] as PresetImpl<PresetName>;
  const rects = layout(beat, panels, width, height, impl);
  const shadow = impl.panelShadow?.(height / 1080) ?? true;
  const ctx = (index: number): PresetContext => ({
    frame,
    durationInFrames,
    fps,
    frameSize: { width, height },
    seed: `${beat.line_id}:${index}`,
    index,
    count: panels.length,
  });
  // The vision stage's pre-blurred backdrop when there is one: blurring the whole frame here every frame is slow.
  const blurred = panels[0]?.backdrop;
  const backdrop = blurred ?? (panels[0] && (panels[0].clean ?? panels[0].crop ?? panels[0].page));

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
            filter: blurred ? undefined : "blur(40px) brightness(0.4) saturate(1.2)",
          }}
        />
      ) : null}
      <AmbientLayer effect={ambientEffect} frame={startFrame + frame} fps={fps} width={width} height={height} />
      {panels.map((panel, i) => (
        <Camera
          key={panel.id}
          // A single panel's move is kept inside the frame: the panel is never cropped (multi-panel presets slide in by design).
          transform={
            panels.length === 1
              ? keepInFrame(impl.camera(ctx(i), beat.preset.params as never), rects[i]!, { width, height }, panelSize(panel))
              : impl.camera(ctx(i), beat.preset.params as never)
          }
          {...rects[i]!}
        >
          <PolygonPanel
            panel={panel}
            width={rects[i]!.width}
            height={rects[i]!.height}
            hideBubbles={hideBubbles}
            shadow={shadow}
          />
        </Camera>
      ))}
      {impl.Overlay ? <impl.Overlay ctx={ctx(0)} params={beat.preset.params as never} /> : null}
      {beat.audio ? (
        // A line shown panel by panel plays one clip across its beats: each starts where the last one stopped.
        <Html5Audio src={resolve(beat.audio.src)} trimBefore={Math.round((beat.audio.from ?? 0) * fps)} />
      ) : null}
    </AbsoluteFill>
  );
}
