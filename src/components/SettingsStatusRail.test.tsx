/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsStatusRail, type SettingsStatusItem } from "./SettingsStatusRail";

const ITEMS: SettingsStatusItem[] = [
  { target: "decision", label: "决策模型", state: "configured", detail: "系统凭据库已配置" },
  { target: "builtin-grok", label: "内置 Grok Build", state: "available", detail: "运行时自检通过" },
  { target: "vision", label: "视觉模型", state: "disabled", detail: "默认关闭" },
];

describe("SettingsStatusRail", () => {
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

  it("renders three stable, keyboard-reachable service summaries", () => {
    act(() => root.render(
      <SettingsStatusRail items={ITEMS} activeTarget="decision" onSelect={() => {}} />,
    ));

    const buttons = [...host.querySelectorAll<HTMLButtonElement>("button")];
    expect(buttons).toHaveLength(3);
    expect(buttons.every((button) => button.type === "button" && button.tabIndex === 0)).toBe(true);
    expect(host.textContent).toContain("已配置");
    expect(host.textContent).toContain("可用");
    expect(host.textContent).toContain("已关闭");
    expect(buttons[0].getAttribute("aria-pressed")).toBe("true");
  });

  it("routes a summary click without performing any other action", () => {
    const onSelect = vi.fn();
    act(() => root.render(<SettingsStatusRail items={ITEMS} onSelect={onSelect} />));

    act(() => host.querySelector<HTMLButtonElement>('[aria-label^="视觉模型"]')?.click());

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith("vision");
  });

  it("keeps unknown evidence unknown instead of claiming availability", () => {
    act(() => root.render(
      <SettingsStatusRail
        items={ITEMS.map((item) => ({ ...item, state: "unknown", detail: "尚未检测" }))}
        onSelect={() => {}}
      />,
    ));

    expect(host.textContent?.match(/未知/g)).toHaveLength(3);
    expect(host.textContent).not.toContain("可用");
  });
});
