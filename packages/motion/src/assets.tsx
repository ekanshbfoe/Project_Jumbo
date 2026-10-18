import { createContext, useContext } from "react";
import { staticFile } from "remotion";

/** Turns a project-relative asset path from a spec into a loadable URL. */
export type AssetResolver = (path: string) => string;

const isUrl = (path: string) => /^(https?:|data:|blob:)/.test(path);

/** Renders resolve through Remotion's public dir. The desktop preview swaps in Tauri's file URLs. */
export const staticAssetResolver: AssetResolver = (path) => (isUrl(path) ? path : staticFile(path));

const AssetResolverContext = createContext<AssetResolver>(staticAssetResolver);

export const AssetResolverProvider = AssetResolverContext.Provider;

export const useAssetResolver = () => useContext(AssetResolverContext);
