import { describe, expect, it } from "vitest";
import {
  currentLogDuplicatesTimeline,
  mergeExecutionLogs,
  normalizeCurrentExecutionLog,
} from "./logPolicy";
import type { ExecutionHistoryEntry, LogEntry, PipelineState } from "./types";

function history(timestamp: string, text: string): ExecutionHistoryEntry {
  return { timestamp, text, level: "info", event_type: "UserExecute" };
}

function runtime(timestamp: string, text: string): LogEntry {
  return { timestamp, text, level: "info" };
}

describe("execution log merge policy", () => {
  it("sorts persisted and runtime logs on one timeline", () => {
    const merged = mergeExecutionLogs(
      [history("2026-07-22T12:15:27Z", "later")],
      [runtime("2026-07-22T12:13:45Z", "earlier")],
    );
    expect(merged.map(entry => entry.text)).toEqual(["earlier", "later"]);
  });

  it("deduplicates exact entries and mirrored runtime start records", () => {
    const start = "▶ 执行中 (1/1)：任务";
    const merged = mergeExecutionLogs(
      [history("2026-07-22T12:00:00Z", start)],
      [
        runtime("2026-07-22T12:00:01Z", start),
        runtime("2026-07-22T12:00:02Z", "output"),
        runtime("2026-07-22T12:00:02Z", "output"),
      ],
    );
    expect(merged.map(entry => entry.text)).toEqual([start, "output"]);
  });

  it("keeps stable order for equal and invalid timestamps", () => {
    const merged = mergeExecutionLogs(
      [history("2026-07-22T12:00:00Z", "first"), history("invalid", "invalid")],
      [runtime("2026-07-22T12:00:00Z", "second")],
    );
    expect(merged.map(entry => entry.text)).toEqual(["first", "second", "invalid"]);
  });

  it("defaults old persisted history and runtime logs to system source", () => {
    const merged = mergeExecutionLogs(
      [history("2026-07-22T12:00:00Z", "legacy")],
      [runtime("2026-07-22T12:00:01Z", "runtime")],
    );
    expect(merged.map(entry => entry.operationSource)).toEqual(["System", "System"]);
  });

  it("preserves all persisted operation sources independently from the log channel", () => {
    const sources = ["User", "Autopilot", "Recovery", "System"] as const;
    const entries = sources.map((source, index) => ({
      ...history(`2026-07-22T12:00:0${index}Z`, source),
      source,
    }));
    const merged = mergeExecutionLogs(entries, []);
    expect(merged.map(entry => entry.timelineSource)).toEqual([
      "history",
      "history",
      "history",
      "history",
    ]);
    expect(merged.map(entry => entry.operationSource)).toEqual(sources);
  });

  it("preserves separately correlated control events with the same text", () => {
    const base = history("2026-07-22T12:00:00Z", "验证完成");
    const merged = mergeExecutionLogs([
      { ...base, subtask_id: "task", criterion_index: 1, action_id: "action-1" },
      { ...base, subtask_id: "task", criterion_index: 2, action_id: "action-2" },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged.map(entry => entry.criterionIndex)).toEqual([1, 2]);
    expect(merged.map(entry => entry.actionId)).toEqual(["action-1", "action-2"]);
  });

  it("preserves runtime source and correlation while deduplicating the live current entry", () => {
    const entry = {
      ...runtime("2026-08-08T00:00:00Z", "[stdout] completed"),
      source: "stdout",
      correlation_id: "call-7",
    };
    const timeline = mergeExecutionLogs([], [entry]);
    expect(timeline[0].source).toBe("stdout");
    expect(timeline[0].correlation_id).toBe("call-7");
    expect(currentLogDuplicatesTimeline(entry, timeline)).toBe(true);
  });

  it("normalizes structured and legacy thought current logs as debug without raw JSON", () => {
    const structured = normalizeCurrentExecutionLog({
      current_log: JSON.stringify({
        kind: "runtime_log",
        level: "debug",
        source: "stdout",
        correlation_id: "turn-3",
        text: "inspect state",
      }),
      log_history: [],
    } as unknown as PipelineState);
    expect(structured).toMatchObject({
      level: "debug",
      source: "stdout",
      correlation_id: "turn-3",
      text: "inspect state",
    });

    const legacy = normalizeCurrentExecutionLog({
      current_log: '{"type":"thought","data":"legacy thought","id":"legacy-1"}',
      log_history: [],
    } as unknown as PipelineState);
    expect(legacy).toMatchObject({
      level: "debug",
      source: "legacy-jsonl",
      correlation_id: "legacy-1",
      text: "legacy thought",
    });
  });

  it("places test logs on the unified timeline with visible severity and source", () => {
    const merged = mergeExecutionLogs([], [], [{
      subtask_title: "定向验收",
      status: "rejected",
      reason: "断言失败",
    }]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({
      level: "error",
      source: "test",
      timelineSource: "test",
      text: "定向验收：断言失败",
    });
  });
});
