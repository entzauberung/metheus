export type LogFilterCategory = "info" | "success" | "error" | "pause" | "test" | "debug";
export type LogCategoryTone = "blue" | "green" | "red" | "yellow" | "violet";

export interface FilterableExecutionLog {
  level: string;
  source?: string;
  timelineSource?: string;
}

export const LOG_FILTER_CATEGORIES: readonly LogFilterCategory[] = [
  "info",
  "success",
  "error",
  "pause",
  "test",
  "debug",
];

export const DEFAULT_LOG_FILTER_CATEGORIES: readonly LogFilterCategory[] = [
  "info",
  "success",
  "error",
  "pause",
  "test",
];

export const LOG_FILTER_PRESENTATION: Record<
  LogFilterCategory,
  { label: string; tone: LogCategoryTone }
> = {
  info: { label: "信息", tone: "blue" },
  success: { label: "成功", tone: "green" },
  error: { label: "错误", tone: "red" },
  pause: { label: "暂停", tone: "yellow" },
  test: { label: "测试", tone: "violet" },
  debug: { label: "调试", tone: "blue" },
};

export function createDefaultLogFilters(): Set<LogFilterCategory> {
  return new Set(DEFAULT_LOG_FILTER_CATEGORIES);
}

export function selectAllLogFilters(): Set<LogFilterCategory> {
  return new Set(LOG_FILTER_CATEGORIES);
}

export function clearAllLogFilters(): Set<LogFilterCategory> {
  return new Set();
}

export function toggleLogFilter(
  current: ReadonlySet<LogFilterCategory>,
  category: LogFilterCategory,
): Set<LogFilterCategory> {
  const next = new Set(current);
  if (next.has(category)) next.delete(category);
  else next.add(category);
  return next;
}

export function areAllLogFiltersSelected(
  current: ReadonlySet<LogFilterCategory>,
): boolean {
  return LOG_FILTER_CATEGORIES.every((category) => current.has(category));
}

export function logCategory(entry: FilterableExecutionLog): LogFilterCategory | "debug" | null {
  if (entry.timelineSource === "test" || entry.source === "test") return "test";
  if (entry.level === "debug") return "debug";
  return LOG_FILTER_CATEGORIES.includes(entry.level as LogFilterCategory)
    ? entry.level as LogFilterCategory
    : null;
}

export function matchesLogFilters(
  entry: FilterableExecutionLog,
  visible: ReadonlySet<LogFilterCategory>,
): boolean {
  const category = logCategory(entry);
  return category !== null && visible.has(category);
}

export function filterExecutionLogs<T extends FilterableExecutionLog>(
  entries: readonly T[],
  visible: ReadonlySet<LogFilterCategory>,
): T[] {
  return entries.filter((entry) => matchesLogFilters(entry, visible));
}

export function emptyLogFilterMessage(visible: ReadonlySet<LogFilterCategory>): string {
  return visible.size === 0
    ? "未选择日志分类。选择“全部”可恢复日志。"
    : "当前分类下暂无日志。";
}
