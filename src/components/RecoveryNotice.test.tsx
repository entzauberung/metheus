/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RecoveryPresentation } from "../types";
import { RecoveryNotice } from "./RecoveryNotice";

function recovery(fingerprint: string): RecoveryPresentation {
  return {
    kind: "BaselineRecovery",
    title: "执行已阻断",
    reason: "执行会话已经丢失",
    severity: "Error",
    primary_action: null,
    secondary_actions: [],
    preserve_current_code: false,
    requires_baseline_restore: true,
    supports_preview: true,
    automatic_retry: false,
    capabilities: [],
    decision_options: [],
    state_fingerprint: fingerprint,
  };
}

describe("RecoveryNotice", () => {
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

  it("raises window attention without stealing focus", () => {
    vi.useFakeTimers();
    const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const originalTitle = document.title;

    render("alpha", recovery("background-block"));
    expect(document.title).toContain("【需处理】执行已阻断");
    expect(host.querySelector(".recovery-notice-attention")).not.toBeNull();

    act(() => window.dispatchEvent(new Event("focus")));
    expect(document.title).toBe(originalTitle);
    focus.mockRestore();
    vi.useRealTimers();
  });

  function render(projectName: string, presentation: RecoveryPresentation | null) {
    act(() => root.render(
      <RecoveryNotice projectName={projectName} recoveryPresentation={presentation} />,
    ));
  }

  it("announces each fingerprint once without taking input focus", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    render("alpha", recovery("blocked-1"));
    expect(host.textContent).toContain("执行已阻断");
    expect(host.querySelector("[role='status']")?.getAttribute("aria-live")).toBe("assertive");
    expect(document.activeElement).toBe(input);

    const dismiss = host.querySelector("button");
    act(() => dismiss?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host.textContent).toBe("");

    render("alpha", null);
    render("alpha", recovery("blocked-1"));
    expect(host.textContent).toBe("");

    render("alpha", recovery("blocked-2"));
    expect(host.textContent).toContain("执行已阻断");
    expect(document.activeElement).toBe(input);
    input.remove();
  });

  it("allows the same fingerprint to notify again after switching projects", () => {
    render("alpha", recovery("blocked-1"));
    act(() => host.querySelector("button")?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    render("beta", recovery("blocked-1"));
    expect(host.textContent).toContain("执行已阻断");
  });
});
