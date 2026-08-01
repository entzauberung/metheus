import { describe, expect, it } from "vitest";
import {
  executionPollingOwnsNextAdvance,
  getQualityStatusPresentation,
  isHeartbeatStale,
} from "./autopilotPolicy";
import type { PipelineState } from "./types";

function pipeline(status: PipelineState["status"]): PipelineState {
  return {
    execution_id: "execution-1",
    mid_stage_id: "mid-1",
    status,
    current_subtask_index: 0,
    total_subtasks: 1,
    subtask_statuses: [],
    current_log: "",
    child_pid: undefined,
    project_name: "project-1",
    milestone_id: "milestone-1",
    plan_revision: 1,
    current_subtask_id: "subtask-1",
    awaiting_confirmation: false,
    log_history: [],
  };
}

describe("autopilot scheduling policy", () => {
  it("delegates the next advance to polling while execution remains running", () => {
    expect(executionPollingOwnsNextAdvance(pipeline("Running"))).toBe(true);
    expect(executionPollingOwnsNextAdvance(pipeline("Completed"))).toBe(false);
  });

  it("presents automated tests, code review, and acceptance evidence separately", () => {
    const presentation = getQualityStatusPresentation({
      passed: true,
      issues: [],
      suggestion: "",
      automated_test_status: "NotConfigured",
      review_passed: true,
      review_evidence_status: "Partial",
      review_issues: [{
        criterion: "criterion",
        file: "index.html",
        expected: "",
        actual: "style suggestion",
        suggested_change: "",
        confidence: 0.9,
        severity: "Suggestion",
      }],
    }, [{
      criterion_index: 1,
      criterion: "criterion",
      status: "Satisfied",
      evidence: "E001",
      evidence_references: [],
      confidence: 0.9,
      updated_at: "",
    }]);

    expect(presentation.map(item => item.label)).toEqual([
      "自动化测试：未配置",
      "代码审查：通过",
      "审查协议：未请求",
      "验收证据：充分",
    ]);
    expect(presentation.some(item => item.tone === "error")).toBe(false);
  });

  it("keeps review protocol failures separate from AI review failures", () => {
    const presentation = getQualityStatusPresentation({
      passed: false,
      issues: [],
      suggestion: "",
      automated_test_status: "Passed",
      review_status: "Failed",
      review_failure_kind: "FieldTypeMismatch",
    }, []);
    expect(presentation.map(item => item.label)).toEqual([
      "自动化测试：通过",
      "代码审查：待确认",
      "审查协议：格式异常",
      "验收证据：无逐项标准",
    ]);
  });

  it("flags a stalled heartbeat only for active runs", () => {
    const now = Date.parse("2026-07-26T00:00:30Z");
    expect(isHeartbeatStale("2026-07-26T00:00:00Z", true, now)).toBe(true);
    expect(isHeartbeatStale("2026-07-26T00:00:25Z", true, now)).toBe(false);
    expect(isHeartbeatStale("2026-07-26T00:00:00Z", false, now)).toBe(false);
  });
});
