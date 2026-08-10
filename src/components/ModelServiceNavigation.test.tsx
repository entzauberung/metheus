/* @vitest-environment happy-dom */

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ModelServiceNavigation, type ModelServiceTarget } from "./ModelServiceNavigation";

function KeyboardHarness({
  initialValue = "decision",
  focusRequest = 0,
  onChange,
}: {
  initialValue?: ModelServiceTarget;
  focusRequest?: number;
  onChange?: (target: ModelServiceTarget) => void;
}) {
  const [value, setValue] = useState<ModelServiceTarget>(initialValue);
  return (
    <ModelServiceNavigation
      value={value}
      onChange={(target) => {
        onChange?.(target);
        setValue(target);
      }}
      focusRequest={focusRequest}
    />
  );
}

describe("ModelServiceNavigation", () => {
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

  it("renders three keyboard-reachable tabs with one selected target", () => {
    act(() => root.render(
      <ModelServiceNavigation value="builtin-grok" onChange={() => {}} />,
    ));

    const tabs = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual([
      "决策模型",
      "内置 Grok Build",
      "视觉模型",
    ]);
    expect(tabs.filter((tab) => tab.getAttribute("aria-selected") === "true")).toHaveLength(1);
    expect(tabs[1].tabIndex).toBe(0);
    expect(tabs[0].tabIndex).toBe(-1);
  });

  it("routes the selected service without scrolling or side effects", () => {
    const onChange = vi.fn<(target: ModelServiceTarget) => void>();
    act(() => root.render(
      <ModelServiceNavigation value="decision" onChange={onChange} />,
    ));

    act(() => host.querySelector<HTMLButtonElement>('[role="tab"][aria-controls="model-service-panel-vision"]')?.click());
    expect(onChange).toHaveBeenCalledWith("vision");
  });

  it("moves selection and focus with ArrowLeft, ArrowRight, Home, and End", () => {
    const onChange = vi.fn();
    act(() => root.render(<KeyboardHarness onChange={onChange} initialValue="decision" />));

    const tabs = () => [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    const press = (tab: HTMLButtonElement, key: string) => {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      act(() => {
        tab.dispatchEvent(event);
      });
      return event;
    };
    const expectSelected = (index: number) => {
      const currentTabs = tabs();
      expect(currentTabs.filter((tab) => tab.getAttribute("aria-selected") === "true")).toHaveLength(1);
      expect(currentTabs.filter((tab) => tab.tabIndex === 0)).toHaveLength(1);
      expect(currentTabs[index].getAttribute("aria-selected")).toBe("true");
      expect(currentTabs[index].tabIndex).toBe(0);
      expect(document.activeElement).toBe(currentTabs[index]);
    };

    act(() => tabs()[0].focus());

    let event = press(tabs()[0], "ArrowLeft");
    expect(event.defaultPrevented).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith("vision");
    expect(onChange).toHaveBeenCalledTimes(1);
    expectSelected(2);

    event = press(tabs()[2], "ArrowRight");
    expect(event.defaultPrevented).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith("decision");
    expect(onChange).toHaveBeenCalledTimes(2);
    expectSelected(0);

    event = press(tabs()[0], "ArrowRight");
    expect(event.defaultPrevented).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith("builtin-grok");
    expect(onChange).toHaveBeenCalledTimes(3);
    expectSelected(1);

    event = press(tabs()[1], "Home");
    expect(event.defaultPrevented).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith("decision");
    expect(onChange).toHaveBeenCalledTimes(4);
    expectSelected(0);

    event = press(tabs()[0], "End");
    expect(event.defaultPrevented).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith("vision");
    expect(onChange).toHaveBeenCalledTimes(5);
    expectSelected(2);
  });

  it("ignores unrelated keys", () => {
    const onChange = vi.fn();
    act(() => root.render(<KeyboardHarness onChange={onChange} initialValue="builtin-grok" />));

    const tabs = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    const event = new KeyboardEvent("keydown", { key: "PageDown", bubbles: true, cancelable: true });

    act(() => {
      tabs[1].focus();
      tabs[1].dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(tabs[1]);
    expect(tabs.filter((tab) => tab.getAttribute("aria-selected") === "true")).toHaveLength(1);
    expect(tabs.filter((tab) => tab.tabIndex === 0)).toHaveLength(1);
  });

  it("focuses the active target only for an explicit focus request", () => {
    act(() => root.render(
      <ModelServiceNavigation value="decision" onChange={() => {}} focusRequest={0} />,
    ));
    expect(document.activeElement).not.toBe(host.querySelector('[role="tab"][aria-selected="true"]'));

    act(() => root.render(
      <ModelServiceNavigation value="decision" onChange={() => {}} focusRequest={1} />,
    ));
    expect(document.activeElement).toBe(host.querySelector('[role="tab"][aria-selected="true"]'));
  });
});
