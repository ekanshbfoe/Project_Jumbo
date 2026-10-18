import { describe, expect, it } from "vitest";
import { bubbleMask, fitContain, panelSize, polygonToClipPath, polygonToSvgPoints, splitFrame } from "../src/geometry";
import { noise1D } from "../src/noise";

const bbox = { x: 0.1, y: 0.2, w: 0.5, h: 0.4 };

describe("polygonToClipPath", () => {
  it("maps page coordinates into bbox percentages", () => {
    const clip = polygonToClipPath(
      [
        [0.1, 0.2],
        [0.6, 0.2],
        [0.6, 0.6],
        [0.35, 0.5],
      ],
      bbox,
    );
    expect(clip).toBe("polygon(0% 0%, 100% 0%, 100% 100%, 50% 75%)");
  });

  it("produces matching SVG points", () => {
    expect(polygonToSvgPoints([[0.1, 0.2], [0.6, 0.2], [0.35, 0.6]], bbox)).toBe("0,0 100,0 50,100");
  });
});

describe("bubbleMask", () => {
  it("returns null without bubbles", () => {
    expect(bubbleMask([], bbox)).toBeNull();
  });

  it("cuts an even-odd hole per bubble", () => {
    const mask = bubbleMask(
      [{ id: "b_0001", kind: "speech", polygon: [[0.1, 0.2], [0.35, 0.2], [0.35, 0.4]] }],
      bbox,
    );
    const svg = decodeURIComponent(mask!.slice('url("data:image/svg+xml,'.length, -2));
    expect(svg).toContain('fill-rule="evenodd"');
    expect(svg).toContain("M0 0H100V100H0ZM0.000 0.000L50.000 0.000L50.000 50.000Z");
  });
});

describe("layout helpers", () => {
  it("computes the bbox pixel size", () => {
    expect(panelSize({ bbox, page_size: { width: 1000, height: 1500 } })).toEqual({ width: 500, height: 600 });
  });

  it("fits and centres content inside the frame with a margin", () => {
    expect(fitContain({ width: 500, height: 1000 }, { width: 1920, height: 1080 }, 0.05)).toEqual({
      width: 486,
      height: 972,
      left: 717,
      top: 54,
    });
  });

  it("splits a frame into slots with gaps", () => {
    expect(splitFrame({ width: 1920, height: 1080 }, 2, "vertical", 20)).toEqual([
      { left: 0, top: 0, width: 950, height: 1080 },
      { left: 970, top: 0, width: 950, height: 1080 },
    ]);
  });
});

describe("noise1D", () => {
  it("is deterministic, bounded and continuous", () => {
    const samples = Array.from({ length: 200 }, (_, i) => noise1D("seed", i / 10));
    expect(samples).toEqual(Array.from({ length: 200 }, (_, i) => noise1D("seed", i / 10)));
    expect(samples.every((v) => v >= -1 && v <= 1)).toBe(true);
    expect(Math.abs(noise1D("seed", 3.999) - noise1D("seed", 4))).toBeLessThan(0.01);
    expect(noise1D("a", 0.5)).not.toBe(noise1D("b", 0.5));
  });
});
