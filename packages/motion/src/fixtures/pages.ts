/**
 * Synthetic manhwa pages for developing and testing the motion library. Panel outlines and bubbles are
 * defined once in pixels; scripts/make-fixtures.ts draws them into SVGs and `panelsDoc()` turns the
 * same numbers into a PanelsDoc, so masks always line up with the art.
 */
import type { Bubble, Panel, PanelsDoc } from "@jumbo/schema";

type Px = [number, number];

interface FixtureBubble {
  center: Px;
  radius: Px;
  text: string;
}

export interface FixturePanel {
  id: string;
  /** Outline in page pixels. */
  outline: Px[];
  /** Draw a border, like a printed page. Webtoon slices are usually borderless. */
  bordered: boolean;
  bubbles: FixtureBubble[];
  /** SVG markup drawn inside the panel, in page pixels. */
  art: string;
}

export interface FixturePage {
  file: string;
  width: number;
  height: number;
  layout: "page" | "strip";
  panels: FixturePanel[];
}

const rect = (x0: number, y0: number, x1: number, y1: number): Px[] => [
  [x0, y0],
  [x1, y0],
  [x1, y1],
  [x0, y1],
];

const skyline = `
  <rect x="60" y="60" width="880" height="440" fill="url(#night)"/>
  <circle cx="780" cy="150" r="48" fill="#f6e7b0" opacity="0.9"/>
  <path d="M60 500 V380 h60 v-60 h50 v90 h40 v-140 h70 v150 h50 v-80 h60 v120 h40 v-200 h80 v210 h50 v-90 h70 v60 h60 v-130 h70 v170 h60 v-70 h70 v120 Z" fill="#0a0d1a"/>
  <g fill="#ffd36b" opacity="0.8">${Array.from({ length: 40 }, (_, i) => `<rect x="${90 + ((i * 97) % 820)}" y="${360 + ((i * 53) % 120)}" width="8" height="12"/>`).join("")}</g>`;

const portrait = (cx: number, cy: number, hue: string, glow: string) => `
  <circle cx="${cx}" cy="${cy - 60}" r="120" fill="${glow}" opacity="0.35"/>
  <path d="M${cx - 150} ${cy + 230} Q${cx} ${cy + 20} ${cx + 150} ${cy + 230} Z" fill="${hue}"/>
  <circle cx="${cx}" cy="${cy - 40}" r="85" fill="#f1c9a5"/>
  <path d="M${cx - 92} ${cy - 50} Q${cx - 80} ${cy - 160} ${cx} ${cy - 150} Q${cx + 95} ${cy - 160} ${cx + 92} ${cy - 40} L${cx + 70} ${cy - 95} L${cx} ${cy - 110} L${cx - 70} ${cy - 95} Z" fill="#1b1b24"/>
  <rect x="${cx - 45}" y="${cy - 52}" width="22" height="9" fill="#1b1b24"/>
  <rect x="${cx + 23}" y="${cy - 52}" width="22" height="9" fill="#1b1b24"/>`;

const slash = `
  <rect x="60" y="1040" width="880" height="300" fill="#1a0b10"/>
  <g stroke="#ffffff" stroke-width="3" opacity="0.5">${Array.from({ length: 30 }, (_, i) => `<line x1="500" y1="1190" x2="${60 + i * 31}" y2="${i % 2 ? 1040 : 1340}"/>`).join("")}</g>
  <path d="M120 1300 L880 1070 L860 1110 L140 1320 Z" fill="#ff3b5c"/>
  <path d="M140 1290 L860 1085" stroke="#fff4f6" stroke-width="6"/>`;

const cafe = `
  <rect x="60" y="60" width="880" height="640" fill="#f3e3cf"/>
  <rect x="60" y="520" width="880" height="180" fill="#b88a5e"/>
  ${portrait(300, 430, "#3b5b92", "#9ec5ff")}
  ${portrait(700, 430, "#8f3b52", "#ffb3c6")}`;

const corridor = `
  <rect x="60" y="740" width="880" height="600" fill="#20242f"/>
  <path d="M60 1340 L420 1000 L580 1000 L940 1340 Z" fill="#363c4d"/>
  <rect x="420" y="860" width="160" height="140" fill="#9be7ff" opacity="0.7"/>
  ${portrait(500, 1180, "#2d2d38", "#9be7ff")}`;

const closeUp = `
  <defs><linearGradient id="fadeA" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0f1530" stop-opacity="0"/><stop offset="0.2" stop-color="#0f1530"/><stop offset="0.8" stop-color="#28194a"/><stop offset="1" stop-color="#28194a" stop-opacity="0"/></linearGradient></defs>
  <rect x="0" y="100" width="800" height="800" fill="url(#fadeA)"/>
  <circle cx="400" cy="520" r="230" fill="#f1c9a5"/>
  <path d="M170 500 Q190 250 400 260 Q620 250 630 500 L590 400 L400 360 L210 400 Z" fill="#14141c"/>
  <ellipse cx="320" cy="530" rx="44" ry="22" fill="#ffffff"/><circle cx="320" cy="530" r="17" fill="#3ad6ff"/>
  <ellipse cx="480" cy="530" rx="44" ry="22" fill="#ffffff"/><circle cx="480" cy="530" r="17" fill="#3ad6ff"/>`;

const gate = `
  <defs><radialGradient id="portal"><stop offset="0" stop-color="#e9ddff"/><stop offset="0.45" stop-color="#7b4dff"/><stop offset="1" stop-color="#120a2a"/></radialGradient></defs>
  <rect x="0" y="1100" width="800" height="800" fill="#0c0a18"/>
  <ellipse cx="400" cy="1480" rx="250" ry="330" fill="url(#portal)"/>
  <g fill="#c9b6ff">${Array.from({ length: 24 }, (_, i) => `<circle cx="${120 + ((i * 131) % 560)}" cy="${1150 + ((i * 211) % 700)}" r="${2 + (i % 4)}"/>`).join("")}</g>
  <path d="M370 1900 L370 1760 Q400 1730 430 1760 L430 1900 Z" fill="#05040a"/>`;

/** An ultra-wide close-up of a pair of eyes, for stare-down strips. */
const eyes = (y0: number, skin: string, iris: string, glare: number) => {
  const cy = y0 + 118;
  const eye = (cx: number, flip: number) => `
    <path d="M${cx - 112} ${cy} Q${cx} ${cy - 58} ${cx + 112} ${cy} Q${cx} ${cy + 44} ${cx - 112} ${cy} Z" fill="#fdfbf7" stroke="#1b1b24" stroke-width="5"/>
    <circle cx="${cx}" cy="${cy}" r="38" fill="${iris}"/>
    <circle cx="${cx}" cy="${cy}" r="17" fill="#0c0c12"/>
    <circle cx="${cx + 12}" cy="${cy - 13}" r="8" fill="#ffffff"/>
    <path d="M${cx - 120} ${cy - 62 + flip * glare} L${cx + 120} ${cy - 62 - flip * glare}" stroke="#1b1b24" stroke-width="18" stroke-linecap="round"/>`;
  return `
  <rect x="40" y="${y0}" width="920" height="220" fill="${skin}"/>
  <rect x="40" y="${y0}" width="920" height="70" fill="#000000" opacity="0.18"/>
  ${eye(330, 1)}${eye(670, -1)}`;
};

const katana = `
  <defs><linearGradient id="steel" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#5d6575"/><stop offset="0.55" stop-color="#e9eef7"/><stop offset="1" stop-color="#8b93a4"/></linearGradient></defs>
  <rect x="40" y="600" width="920" height="220" fill="#0b0d14"/>
  <path d="M60 760 L900 650 L940 646 L905 668 L70 778 Z" fill="url(#steel)"/>
  <path d="M120 752 L880 655" stroke="#ffffff" stroke-width="3" opacity="0.9"/>
  <rect x="40" y="748" width="70" height="40" fill="#2a1a12" transform="rotate(-7 75 768)"/>
  <path d="M640 640 L652 676 L688 688 L652 700 L640 736 L628 700 L592 688 L628 676 Z" fill="#ffffff" opacity="0.95"/>`;

export const FIXTURE_PAGES: FixturePage[] = [
  {
    file: "fixtures/page_rect.svg",
    width: 1000,
    height: 1400,
    layout: "page",
    panels: [
      { id: "p_0001", outline: rect(60, 60, 940, 500), bordered: true, bubbles: [], art: skyline },
      { id: "p_0002", outline: rect(60, 540, 480, 1000), bordered: true, bubbles: [], art: `<rect x="60" y="540" width="420" height="460" fill="#cfe0ff"/>${portrait(270, 800, "#3b5b92", "#ffffff")}` },
      { id: "p_0003", outline: rect(520, 540, 940, 1000), bordered: true, bubbles: [], art: `<rect x="520" y="540" width="420" height="460" fill="#ffd6dc"/>${portrait(730, 800, "#8f3b52", "#ffffff")}` },
      { id: "p_0004", outline: [[60, 1040], [940, 1040], [940, 1260], [60, 1340]], bordered: true, bubbles: [], art: slash },
    ],
  },
  {
    file: "fixtures/page_bubbles.svg",
    width: 1000,
    height: 1400,
    layout: "page",
    panels: [
      {
        id: "p_0005",
        outline: rect(60, 60, 940, 700),
        bordered: true,
        bubbles: [
          { center: [250, 170], radius: [160, 80], text: "Tum yahan kaise?" },
          { center: [760, 200], radius: [150, 90], text: "Gate khul gaya hai." },
        ],
        art: cafe,
      },
      {
        id: "p_0006",
        outline: [[60, 740], [600, 740], [940, 900], [940, 1340], [60, 1340]],
        bordered: true,
        bubbles: [{ center: [260, 840], radius: [170, 70], text: "Peeche mat dekhna." }],
        art: corridor,
      },
    ],
  },
  {
    file: "fixtures/strip_slice.svg",
    width: 800,
    height: 2000,
    layout: "strip",
    panels: [
      { id: "p_0007", outline: rect(0, 100, 800, 900), bordered: false, bubbles: [], art: closeUp },
      { id: "p_0008", outline: rect(0, 1100, 800, 1900), bordered: false, bubbles: [], art: gate },
    ],
  },
  {
    file: "fixtures/page_slices.svg",
    width: 1000,
    height: 1400,
    layout: "page",
    panels: [
      { id: "p_0009", outline: rect(40, 80, 960, 300), bordered: true, bubbles: [], art: eyes(80, "#f1c9a5", "#3a8dde", 0) },
      { id: "p_0010", outline: rect(40, 340, 960, 560), bordered: true, bubbles: [], art: eyes(340, "#e2b08c", "#d7263d", -14) },
      { id: "p_0011", outline: rect(40, 600, 960, 820), bordered: true, bubbles: [], art: katana },
    ],
  },
];

const ellipse = ({ center: [cx, cy], radius: [rx, ry] }: FixtureBubble, steps = 24): Px[] =>
  Array.from({ length: steps }, (_, i) => {
    const a = (i / steps) * Math.PI * 2;
    return [Math.round(cx + rx * Math.cos(a)), Math.round(cy + ry * Math.sin(a))];
  });

const round = (v: number) => Math.round(v * 1e6) / 1e6;

/** Renders a fixture page as an SVG document. */
export function pageSvg(page: FixturePage): string {
  const panels = page.panels
    .map((panel, i) => {
      const points = panel.outline.map((p) => p.join(",")).join(" ");
      const bubbles = panel.bubbles
        .map((b) => {
          const pts = ellipse(b).map((p) => p.join(",")).join(" ");
          return `<polygon points="${pts}" fill="#ffffff" stroke="#111" stroke-width="4"/>
            <text x="${b.center[0]}" y="${b.center[1] + 10}" font-family="Comic Sans MS, sans-serif" font-size="30" font-weight="700" text-anchor="middle" fill="#111">${b.text}</text>`;
        })
        .join("");
      const border = panel.bordered ? `<polygon points="${points}" fill="none" stroke="#111" stroke-width="8"/>` : "";
      return `<clipPath id="c${i}"><polygon points="${points}"/></clipPath><g clip-path="url(#c${i})">${panel.art}</g>${bubbles}${border}`;
    })
    .join("\n");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${page.width}" height="${page.height}" viewBox="0 0 ${page.width} ${page.height}">
<defs><linearGradient id="night" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1d2a5c"/><stop offset="1" stop-color="#6b3f7a"/></linearGradient></defs>
<rect width="${page.width}" height="${page.height}" fill="${page.layout === "strip" ? "#0f1530" : "#fbfaf7"}"/>
${panels}
</svg>
`;
}

/** The PanelsDoc a vision stage would produce for the fixture pages, as one chapter. */
export function panelsDoc(): PanelsDoc {
  const panels: Panel[] = FIXTURE_PAGES.flatMap((page) =>
    page.panels.map((panel): Panel => {
      const xs = panel.outline.map((p) => p[0]);
      const ys = panel.outline.map((p) => p[1]);
      const norm = ([x, y]: Px): [number, number] => [round(x / page.width), round(y / page.height)];
      const [x0, y0] = norm([Math.min(...xs), Math.min(...ys)]);
      const [x1, y1] = norm([Math.max(...xs), Math.max(...ys)]);
      return {
        id: panel.id,
        order: Number(panel.id.slice(2)) - 1,
        page: page.file,
        page_size: { width: page.width, height: page.height },
        bbox: { x: x0, y: y0, w: round(x1 - x0), h: round(y1 - y0) },
        polygon: panel.outline.map(norm),
        bubbles: panel.bubbles.map(
          (b, i): Bubble => ({ id: `b_${String(i + 1).padStart(4, "0")}`, kind: "speech", polygon: ellipse(b).map(norm), text: b.text }),
        ),
        characters: [],
      };
    }),
  );
  return { schema_version: 1, chapter_id: "ch_0001", layout: "page", panels };
}
