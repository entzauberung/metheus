/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ExecutionRecoveryImpact } from "./types";
import { RecoveryImpactDialog } from "./RecoveryImpactDialog";

describe("RecoveryImpactDialog", () => {
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

  it("shows managed, external, and untracked impact before confirmation", () => {
    const onConfirm = vi.fn();
    const impact: ExecutionRecoveryImpact = {
      action_label: "恢复执行基线",
      confirmation_title: "确认恢复执行基线",
      presentation_description: "后端恢复影响说明",
      safety_stash_summary: "后端安全暂存说明",
      baseline_commit: "abc123",
      current_head: "def456",
      affected_files: ["src/app.ts", "notes.txt"],
      untracked_files: ["notes.txt"],
      managed_changes: ["src/app.ts"],
      external_changes: ["notes.txt"],
      discarded_files: ["src/app.ts", "notes.txt"],
      creates_safety_stash: true,
      has_destructive_changes: true,
      state_fingerprint: "impact-1",
    };
    act(() => {
      root.render(
        <RecoveryImpactDialog
          impact={impact}
          busy={false}
          onCancel={vi.fn()}
          onConfirm={onConfirm}
        />,
      );
    });

    expect(document.body.textContent).toContain("系统受管修改");
    expect(document.body.textContent).toContain("外部未知修改");
    expect(document.body.textContent).toContain("未跟踪文件");
    const confirm = [...document.body.querySelectorAll("button")]
      .find(button => button.textContent?.includes("恢复执行基线"));
    act(() => confirm?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
