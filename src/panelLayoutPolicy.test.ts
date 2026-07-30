import { describe, expect, it } from "vitest";
import {
  clampPanelWidth,
  inspectorPresentation,
  readStoredPanelWidth,
} from "./panelLayoutPolicy";

describe("panel layout policy", () => {
  it("clamps persisted widths and rejects invalid values", () => {
    const storage = (value: string | null) => ({ getItem: () => value });
    expect(readStoredPanelWidth(storage("900"), "width", 420, 320, 680)).toBe(680);
    expect(readStoredPanelWidth(storage("120"), "width", 420, 320, 680)).toBe(320);
    expect(readStoredPanelWidth(storage("invalid"), "width", 420, 320, 680)).toBe(420);
    expect(readStoredPanelWidth(null, "width", 420, 320, 680)).toBe(420);
  });

  it("rounds pointer widths without crossing bounds", () => {
    expect(clampPanelWidth(419.6, 320, 680)).toBe(420);
    expect(clampPanelWidth(Number.NaN, 320, 680)).toBe(320);
  });

  it("selects docked, drawer, and fullscreen presentation breakpoints", () => {
    expect(inspectorPresentation(1_280)).toBe("docked");
    expect(inspectorPresentation(1_024)).toBe("drawer");
    expect(inspectorPresentation(899)).toBe("fullscreen");
  });
});
