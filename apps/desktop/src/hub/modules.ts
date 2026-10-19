import type { ComponentType } from "react";
import type { ProjectInfo } from "../api";
import { ChaptersScreen } from "../screens/ChaptersScreen";
import { ReceiptBookScreen } from "../screens/ReceiptBookScreen";
import { SessionScreen } from "../screens/SessionScreen";

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
      { id: "receipt-book", label: "Receipt Book", Component: ReceiptBookScreen },
      { id: "session", label: "Session", Component: SessionScreen },
    ],
  },
];
