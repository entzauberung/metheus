/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConsoleUtilityBar } from "./ConsoleUtilityBar";

describe("ConsoleUtilityBar", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it("keeps sync facts readable and exposes one settings and inspector entry", () => {
    const onOpenInspector = vi.fn();
    act(() => root.render(
      <ConsoleUtilityBar
        syncStatus={<span data-testid="sync-fact">状态已同步</span>}
        inspectorOpen={false}
        onOpenInspector={onOpenInspector}
        settings={<button type="button" aria-label="应用设置">设置</button>}
      />,
    ));

    const utility = host.querySelector('[data-console-region="utility"]');
    expect(utility).not.toBeNull();
    expect(utility?.querySelector('[role="status"]')?.textContent).toContain("状态已同步");
    expect(utility?.querySelectorAll('button[aria-label="应用设置"]')).toHaveLength(1);
    expect(utility?.querySelectorAll('button[aria-controls="task-inspector"]')).toHaveLength(1);
    expect(utility?.querySelector('button[aria-controls="task-inspector"]')?.getAttribute("aria-expanded"))
      .toBe("false");
    expect(utility?.querySelectorAll("button")).toHaveLength(2);
    expect(utility?.querySelector('button[aria-label="同步"]')).toBeNull();
  });

  it("only opens an inspector and never supplies a second close action", () => {
    const onOpenInspector = vi.fn();
    act(() => root.render(
      <ConsoleUtilityBar
        syncStatus="通知重连中"
        inspectorOpen
        onOpenInspector={onOpenInspector}
        settings={<button type="button">设置</button>}
      />,
    ));

    const inspectorButton = host.querySelector<HTMLButtonElement>('button[aria-controls="task-inspector"]');
    expect(inspectorButton?.getAttribute("aria-label")).toBe("任务检查器已打开");
    act(() => inspectorButton?.click());
    expect(onOpenInspector).toHaveBeenCalledTimes(1);
    expect(host.textContent).not.toContain("关闭任务检查器");
  });
});
