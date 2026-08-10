import { describe, expect, it } from "vitest";
import {
  areAllLogFiltersSelected,
  clearAllLogFilters,
  createDefaultLogFilters,
  emptyLogFilterMessage,
  filterExecutionLogs,
  logCategory,
  matchesLogFilters,
  selectAllLogFilters,
  toggleLogFilter,
  type FilterableExecutionLog,
} from "./logFilterPolicy";

interface FixtureLog extends FilterableExecutionLog {
  id: string;
  text: string;
}

const LOGS: FixtureLog[] = [
  { id: "info", level: "info", text: "ordinary info", timelineSource: "runtime" },
  { id: "success", level: "success", text: "ordinary success", timelineSource: "history" },
  { id: "test-failed", level: "error", text: "test failed", timelineSource: "test", source: "test" },
  { id: "error", level: "error", text: "ordinary error", timelineSource: "history" },
  { id: "test-passed", level: "success", text: "test passed", source: "test" },
  { id: "pause", level: "pause", text: "paused", timelineSource: "runtime" },
  { id: "debug", level: "debug", text: "thought", timelineSource: "runtime" },
];

describe("logFilterPolicy", () => {
  it("classifies test source independently from success and error levels", () => {
    expect(logCategory(LOGS[2])).toBe("test");
    expect(logCategory(LOGS[4])).toBe("test");
    expect(filterExecutionLogs(LOGS, new Set(["test"])).map((entry) => entry.id))
      .toEqual(["test-failed", "test-passed"]);
    expect(filterExecutionLogs(LOGS, new Set(["success"])).map((entry) => entry.id))
      .toEqual(["success"]);
    expect(filterExecutionLogs(LOGS, new Set(["error"])).map((entry) => entry.id))
      .toEqual(["error"]);
  });

  it("keeps debug on its existing hidden-by-default path", () => {
    const visible = createDefaultLogFilters();
    expect(logCategory(LOGS[6])).toBe("debug");
    expect(matchesLogFilters(LOGS[6], visible)).toBe(false);
    expect(filterExecutionLogs(LOGS, visible).map((entry) => entry.id)).not.toContain("debug");
    expect(filterExecutionLogs(LOGS, selectAllLogFilters()).map((entry) => entry.id)).toContain("debug");
  });

  it("selects and clears all categories idempotently", () => {
    const selected = selectAllLogFilters();
    expect(areAllLogFiltersSelected(selected)).toBe(true);
    expect(areAllLogFiltersSelected(selectAllLogFilters())).toBe(true);
    expect(clearAllLogFilters().size).toBe(0);
    expect(clearAllLogFilters().size).toBe(0);
    expect(createDefaultLogFilters().has("debug")).toBe(false);
  });

  it("toggles one category without mutating the previous selection", () => {
    const current = createDefaultLogFilters();
    const withoutTest = toggleLogFilter(current, "test");
    expect(current.has("test")).toBe(true);
    expect(withoutTest.has("test")).toBe(false);
    expect(toggleLogFilter(withoutTest, "test").has("test")).toBe(true);
  });

  it("preserves log order, content and object identity", () => {
    const filtered = filterExecutionLogs(LOGS, new Set(["info", "error"]));
    expect(filtered.map((entry) => entry.id)).toEqual(["info", "error"]);
    expect(filtered[0]).toBe(LOGS[0]);
    expect(filtered[1]).toBe(LOGS[3]);
    expect(LOGS[3].text).toBe("ordinary error");
  });

  it("provides distinct empty states for no selection and no matches", () => {
    expect(emptyLogFilterMessage(new Set())).toContain("选择“全部”");
    expect(emptyLogFilterMessage(new Set(["test"]))).toBe("当前分类下暂无日志。");
  });
});
