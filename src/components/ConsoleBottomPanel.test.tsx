/* @vitest-environment happy-dom */

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  ConsoleBottomPanel,
  type ConsoleBottomView,
} from "./ConsoleBottomPanel";

function Harness() {
  const [activeView, setActiveView] = useState<ConsoleBottomView>("logs");
  const [open, setOpen] = useState(true);
  return (
    <ConsoleBottomPanel
      activeView={activeView}
      onActiveViewChange={setActiveView}
      onOpenChange={setOpen}
      open={open}
      preview={<div data-panel="preview">只读预览内容</div>}
    >
      <div data-panel="logs">真实日志内容</div>
    </ConsoleBottomPanel>
  );
}

describe("ConsoleBottomPanel", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => root.render(<Harness />));
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it("switches between logs and file preview without replacing the parent workspace", () => {
    const tabs = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(tabs.map(tab => tab.textContent)).toEqual([
      "运行与测试日志",
      "文件预览",
    ]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expect(host.querySelector('[data-panel="logs"]')).not.toBeNull();
    expect(host.querySelector('[data-panel="preview"]')).toBeNull();
    const content = host.querySelector<HTMLElement>(".console-bottom-content");
    const panelView = host.querySelector<HTMLElement>("#console-bottom-panel-view");
    expect(content?.style.display).toBe("grid");
    expect(content?.style.gridTemplateRows).toContain("minmax(0, 1fr)");
    expect(panelView?.style.display).toBe("flex");
    expect(panelView?.style.flexDirection).toBe("column");
    expect(panelView?.style.minHeight).toBe("0");
    expect(panelView?.style.overflow).toBe("hidden");

    act(() => tabs[1]?.click());
    expect(tabs[1]?.getAttribute("aria-selected")).toBe("true");
    expect(host.querySelector('[data-panel="logs"]')).toBeNull();
    expect(host.querySelector('[data-panel="preview"]')?.textContent).toContain("只读预览内容");
    expect(host.querySelector(".console-bottom-toggle")?.textContent).toContain("文件预览");
  });

  it("collapses and reopens the selected panel with explicit aria state", () => {
    const previewTab = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
      .find(tab => tab.textContent?.includes("文件预览"));
    act(() => previewTab?.click());

    const toggle = host.querySelector<HTMLButtonElement>(".console-bottom-toggle");
    act(() => toggle?.click());
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector(".console-bottom-content")).toBeNull();

    act(() => toggle?.click());
    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
    expect(host.querySelector('[data-panel="preview"]')).not.toBeNull();
  });
});
