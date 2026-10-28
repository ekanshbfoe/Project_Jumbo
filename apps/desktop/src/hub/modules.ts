import type { ComponentType } from "react";
import type { ProjectInfo } from "../api";
import { ChaptersScreen } from "../screens/ChaptersScreen";
import { PanelsScreen } from "../screens/PanelsScreen";
import { PreviewScreen } from "../screens/PreviewScreen";
import { ReceiptBookScreen } from "../screens/ReceiptBookScreen";
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
      { id: "script", label: "Script", Component: ScriptScreen },
      { id: "voice", label: "Voice", Component: VoiceScreen },
      { id: "preview", label: "Preview", Component: PreviewScreen },
      { id: "receipt-book", label: "Receipt Book", Component: ReceiptBookScreen },
      { id: "session", label: "Session", Component: SessionScreen },
    ],
  },
];
