/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("./utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
}));

vi.mock("./components/ApplicationSettings", () => ({
  ApplicationSettings: () => null,
}));

vi.mock("./components/Modal", () => ({
  Modal: () => null,
}));

vi.mock("./components/ExecutionEngineSelector", () => ({
  ExecutionEngineSelector: ({ onHealthChange }: {
    onHealthChange?: (state: { health: unknown; checking: boolean }) => void;
  }) => (
    <div>
      <button
        type="button"
        onClick={() => onHealthChange?.({
          health: { status: "Unknown", message: "健康状态未知" },
          checking: false,
        })}
      >
        返回 Unknown
      </button>
      <button
        type="button"
        onClick={() => onHealthChange?.({
          health: { status: "Available", message: "执行引擎可用" },
          checking: false,
        })}
      >
        返回 Available
      </button>
    </div>
  ),
}));

import { ProjectEntry } from "./ProjectEntry";

function findButton(host: HTMLElement, label: string): HTMLButtonElement {
  const button = [...host.querySelectorAll("button")]
    .find((candidate) => candidate.textContent?.trim() === label);
  if (!button) throw new Error(`找不到按钮：${label}`);
  return button;
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("ProjectEntry engine health gate", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    invokeMock.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it("keeps entry disabled for Unknown and enables it for Available", () => {
    act(() => root.render(<ProjectEntry onProjectCreated={vi.fn()} />));

    const entryCard = [...host.querySelectorAll<HTMLElement>(".project-entry-card")]
      .find((candidate) => candidate.textContent?.includes("从零开始"));
    if (!entryCard) throw new Error("找不到从零开始入口");
    act(() => entryCard.click());

    const nameInput = host.querySelector<HTMLInputElement>("#proj-name");
    const pathInput = host.querySelector<HTMLInputElement>("#proj-path");
    if (!nameInput || !pathInput) throw new Error("项目入口字段未渲染");
    act(() => {
      setInputValue(nameInput, "health-gate-project");
      setInputValue(pathInput, "/tmp/health-gate-project");
    });

    const submit = host.querySelector<HTMLButtonElement>(".project-entry-submit");
    if (!submit) throw new Error("项目入口提交按钮未渲染");

    act(() => findButton(host, "返回 Unknown").click());
    expect(submit.disabled).toBe(true);

    act(() => findButton(host, "返回 Available").click());
    expect(submit.disabled).toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
