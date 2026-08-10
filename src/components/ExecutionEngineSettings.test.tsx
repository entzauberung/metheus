/* @vitest-environment happy-dom */

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "../types";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("../utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
}));

vi.mock("./IconButton", () => ({
  IconButton: ({ tooltip, onClick }: { tooltip: string; onClick?: () => void }) => (
    <button type="button" aria-label={tooltip} onClick={onClick}>{tooltip}</button>
  ),
}));

vi.mock("./Modal", () => ({
  Modal: ({ isOpen, children, actions = [] }: {
    isOpen: boolean;
    children: ReactNode;
    actions?: Array<{ label: string; onClick: () => void; disabled?: boolean }>;
  }) => isOpen ? (
    <div data-testid="engine-settings-modal">
      {children}
      {actions.map((action) => (
        <button
          type="button"
          key={action.label}
          disabled={action.disabled}
          onClick={action.onClick}
        >
          {action.label}
        </button>
      ))}
    </div>
  ) : null,
}));

vi.mock("./ExecutionEngineSelector", () => ({
  ExecutionEngineSelector: ({ value, onChange, onHealthChange }: {
    value: { runtime: string; provider: string; permission_profile: string; profile_revision: number };
    onChange: (profile: unknown) => void;
    onHealthChange?: (state: { health: unknown; checking: boolean }) => void;
  }) => (
    <div>
      <button
        type="button"
        onClick={() => onChange({ ...value, provider: "Codex" })}
      >
        选择 Codex
      </button>
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

import { ExecutionEngineSettings } from "./ExecutionEngineSettings";

function project(): Project {
  return {
    name: "health-gate-project",
    execution_profile: {
      runtime: "Plugin",
      provider: "ClaudeCode",
      permission_profile: "Unattended",
      profile_revision: 1,
    },
    workflow_state: {
      data_revision: 7,
      recovery_state: undefined,
      autopilot_state: undefined,
      managed_flow_state: undefined,
    },
    execution_session: undefined,
  } as Project;
}

function findButton(host: HTMLElement, label: string): HTMLButtonElement {
  const button = [...host.querySelectorAll("button")]
    .find((candidate) => candidate.textContent?.trim() === label);
  if (!button) throw new Error(`找不到按钮：${label}`);
  return button;
}

describe("ExecutionEngineSettings engine health gate", () => {
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

  it("keeps save disabled for Unknown and enables it for Available", () => {
    act(() => root.render(
      <ExecutionEngineSettings
        project={project()}
        pipeline={null}
        onRuntimeMutation={vi.fn()}
      />,
    ));
    act(() => findButton(host, "执行引擎设置").click());
    act(() => findButton(host, "选择 Codex").click());

    act(() => findButton(host, "返回 Unknown").click());
    expect(findButton(host, "保存").disabled).toBe(true);

    act(() => findButton(host, "返回 Available").click());
    expect(findButton(host, "保存").disabled).toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
