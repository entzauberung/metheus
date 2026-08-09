/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ConsoleBottomPanel } from "./ConsoleBottomPanel";
import { ConsoleNavigator } from "./ConsoleNavigator";
import {
  CONSOLE_LAYOUT_CONTRACT,
  ConsoleCommandBar,
  ConsoleWorkspace,
} from "./ConsoleWorkspace";

describe("Console workspace structure", () => {
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

  it("keeps one command region and switches the navigator without duplicating content", () => {
    act(() => root.render(
      <ConsoleWorkspace
        commandBar={<ConsoleCommandBar><button type="button">同步</button></ConsoleCommandBar>}
        navigator={<ConsoleNavigator taskTree={<p>任务树内容</p>} fileTree={<p>文件树内容</p>} />}
        bottom={<ConsoleBottomPanel><p>日志内容</p></ConsoleBottomPanel>}
      >
        <p>当前步骤</p>
      </ConsoleWorkspace>,
    ));

    expect(host.querySelectorAll('[aria-label="Console 命令栏"]')).toHaveLength(1);
    expect(host.querySelectorAll('[data-console-region="command"]')).toHaveLength(1);
    expect(host.querySelectorAll('[data-console-region="navigator"]')).toHaveLength(1);
    expect(host.querySelectorAll('[data-console-region="main"]')).toHaveLength(1);
    expect(host.querySelectorAll('[data-console-region="bottom"]')).toHaveLength(1);
    expect(host.querySelector(".console-workspace")?.getAttribute("data-console-layout"))
      .toBe("responsive-grid");
    expect(host.querySelector('[data-console-region="navigator"]')?.getAttribute("aria-label"))
      .toBe("Console 导航区");
    expect(host.querySelector('[data-console-region="main"]')?.getAttribute("aria-label"))
      .toBe("Console 主工作区");
    expect(host.querySelector('[data-console-region="bottom"]')?.getAttribute("aria-label"))
      .toBe("Console 底部面板");
    expect(host.textContent).toContain("任务树内容");
    expect(host.textContent).not.toContain("文件树内容");
    const fileTab = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
      .find((tab) => tab.textContent === "文件");
    if (!fileTab) throw new Error("缺少文件导航标签");
    act(() => fileTab.click());
    expect(fileTab.getAttribute("aria-selected")).toBe("true");
    expect(host.textContent).toContain("文件树内容");
    expect(host.textContent).not.toContain("任务树内容");
  });

  it("collapses and restores the single bottom console panel", () => {
    act(() => root.render(<ConsoleBottomPanel><p>日志内容</p></ConsoleBottomPanel>));
    const toggle = host.querySelector<HTMLButtonElement>(".console-bottom-toggle");
    if (!toggle) throw new Error("缺少控制台折叠按钮");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(host.textContent).toContain("日志内容");
    act(() => toggle.click());
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(host.textContent).not.toContain("日志内容");
    act(() => toggle.click());
    expect(host.textContent).toContain("日志内容");
  });

  it("keeps the inspector interaction layer above every floating chat layer", () => {
    expect(CONSOLE_LAYOUT_CONTRACT.inspectorBackdropLayer)
      .toBeGreaterThan(CONSOLE_LAYOUT_CONTRACT.floatingLayerMaximum);
    expect(CONSOLE_LAYOUT_CONTRACT.inspectorLayer)
      .toBeGreaterThan(CONSOLE_LAYOUT_CONTRACT.floatingLayerMaximum);
    expect(CONSOLE_LAYOUT_CONTRACT.inspectorResizeLayer)
      .toBeGreaterThan(CONSOLE_LAYOUT_CONTRACT.inspectorLayer);
  });

  it.each([390, 600, 1280])(
    "keeps one accessible region set at a %dpx viewport contract",
    (width) => {
      host.style.width = `${width}px`;
      act(() => root.render(
        <ConsoleWorkspace
          commandBar={<ConsoleCommandBar><button type="button">同步</button></ConsoleCommandBar>}
          navigator={<ConsoleNavigator taskTree={<p>任务树</p>} fileTree={<p>文件树</p>} />}
          bottom={<ConsoleBottomPanel><p>日志</p></ConsoleBottomPanel>}
        >
          <p>主工作区</p>
        </ConsoleWorkspace>,
      ));

      expect(host.querySelectorAll('[data-console-region="command"]')).toHaveLength(1);
      expect(host.querySelectorAll('[data-console-region="navigator"]')).toHaveLength(1);
      expect(host.querySelectorAll('[data-console-region="main"]')).toHaveLength(1);
      expect(host.querySelectorAll('[data-console-region="bottom"]')).toHaveLength(1);
      expect(host.querySelector('[data-console-region="main"]')?.getAttribute("tabindex")).toBe("0");
      expect(host.querySelector(".console-workspace")?.getAttribute("data-console-compact-max-width"))
        .toBe(String(CONSOLE_LAYOUT_CONTRACT.compactMaxWidth));
      expect(host.querySelector(".console-workspace")?.getAttribute("data-console-single-column-max-width"))
        .toBe(String(CONSOLE_LAYOUT_CONTRACT.singleColumnMaxWidth));
    },
  );
});
