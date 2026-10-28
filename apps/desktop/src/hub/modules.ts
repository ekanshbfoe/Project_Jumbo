import type { ComponentType } from "react";
import type { Method, ProjectInfo } from "../api";
import { ChaptersScreen } from "../screens/ChaptersScreen";
import { GeminiScreen } from "../screens/GeminiScreen";
import { PanelsScreen } from "../screens/PanelsScreen";
import { PreviewScreen } from "../screens/PreviewScreen";
import { ReceiptBookScreen } from "../screens/ReceiptBookScreen";
import { RenderScreen } from "../screens/RenderScreen";
import { ScriptScreen } from "../screens/ScriptScreen";
import { SessionScreen } from "../screens/SessionScreen";
import { VoiceScreen } from "../screens/VoiceScreen";

export interface ScreenProps {
  project: ProjectInfo;
}

export interface HubScreen {
  id: string;
  label: string;
  Component: ComponentType<ScreenProps>;
  /** Only projects of these methods show the screen; all do when unset. */
  methods?: Method[];
}

/**
 * A self-contained tool in the hub. Each module owns its screens (and, on the Rust side, its
 * commands); the shell only renders navigation, so new tools plug in by adding an entry here.
 */
export interface HubModule {
  id: string;
  title: string;
  screens: HubScreen[];
}

export const MODULES: HubModule[] = [
  {
    id: "manhwa",
    title: "Manhwa Studio",
    screens: [
      { id: "chapters", label: "Chapters", Component: ChaptersScreen },
      { id: "panels", label: "Panels", Component: PanelsScreen },
      { id: "gemini", label: "Gemini script", Component: GeminiScreen, methods: ["method2"] },
      { id: "script", label: "Script", Component: ScriptScreen },
      { id: "voice", label: "Voice", Component: VoiceScreen },
      { id: "render", label: "Render", Component: RenderScreen },
      { id: "preview", label: "Preview", Component: PreviewScreen },
      { id: "receipt-book", label: "Receipt Book", Component: ReceiptBookScreen },
      { id: "session", label: "Session", Component: SessionScreen },
    ],
  },
];

/** The screens a project of `method` shows in `module`. */
export function screensFor(module: HubModule, method: Method): HubScreen[] {
  return module.screens.filter((s) => !s.methods || s.methods.includes(method));
}
