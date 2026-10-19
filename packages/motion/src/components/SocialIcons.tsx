import type { SocialPlatform } from "@jumbo/schema";
import type { CSSProperties } from "react";

export const PLATFORM_LABELS: Record<SocialPlatform, string> = {
  x: "X",
  youtube: "YouTube",
  instagram: "Instagram",
};

/** Tile background per platform, matching each brand's own app icon. */
export const PLATFORM_TILES: Record<SocialPlatform, CSSProperties> = {
  x: { background: "#000000", boxShadow: "inset 0 0 0 1px rgba(255, 255, 255, 0.22)" },
  youtube: { background: "linear-gradient(160deg, #ff2a4f 0%, #ff0033 55%, #d4002a 100%)" },
  instagram: {
    background:
      "radial-gradient(circle at 30% 107%, #fdf497 0%, #fdf497 5%, #fd5949 45%, #d6249f 60%, #285aeb 90%)",
  },
};

/** White platform glyph on a 24-unit grid. */
export function PlatformGlyph({ platform, size }: { platform: SocialPlatform; size: number }) {
  switch (platform) {
    case "x":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="#ffffff">
          <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
        </svg>
      );
    case "youtube":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="#ffffff">
          <path d="M9.4 7.6c0-.75.82-1.22 1.47-.84l6.3 3.65c.65.37.65 1.3 0 1.68l-6.3 3.65c-.65.38-1.47-.09-1.47-.84z" />
        </svg>
      );
    case "instagram":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="#ffffff" strokeWidth={2}>
          <rect x="3" y="3" width="18" height="18" rx="5.2" />
          <circle cx="12" cy="12" r="4.2" />
          <circle cx="17.4" cy="6.6" r="1.1" fill="#ffffff" stroke="none" />
        </svg>
      );
  }
}
