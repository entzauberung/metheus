import { describe, expect, it } from "vitest";
import { mergeExecutionLogs } from "./logPolicy";
import type { ExecutionHistoryEntry, LogEntry } from "./types";

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
});
