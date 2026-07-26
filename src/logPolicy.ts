import type { ExecutionHistoryEntry, LogEntry, OperationSource } from "./types";

export interface MergedExecutionLog extends LogEntry {
  key: string;
  source: "history" | "runtime";
  operationSource: OperationSource;
}

export function mergeExecutionLogs(
  history: ExecutionHistoryEntry[] = [],
  runtime: LogEntry[] = [],
): MergedExecutionLog[] {
  const historyStartupTexts = new Set(
    history.filter(entry => entry.text.startsWith("▶ 执行中")).map(entry => entry.text),
  );
  const seen = new Set<string>();
  const entries: Array<MergedExecutionLog & { sequence: number; time: number }> = [];

  const append = (
    entry: LogEntry,
    source: MergedExecutionLog["source"],
    sourceIndex: number,
    operationSource: OperationSource,
  ) => {
    if (
      source === "runtime"
      && entry.text.startsWith("▶ 执行中")
      && historyStartupTexts.has(entry.text)
    ) {
      return;
    }
    const identity = `${entry.timestamp}\u0000${entry.level}\u0000${entry.text}`;
    if (seen.has(identity)) return;
    seen.add(identity);
    const parsedTime = Date.parse(entry.timestamp);
    const sequence = entries.length;
    entries.push({
      ...entry,
      source,
      operationSource,
      key: `${source}-${sourceIndex}-${identity}`,
      sequence,
      time: Number.isFinite(parsedTime) ? parsedTime : Number.POSITIVE_INFINITY,
    });
  };

  history.forEach((entry, index) => append(entry, "history", index, entry.source ?? "System"));
  runtime.forEach((entry, index) => append(entry, "runtime", index, "System"));
  entries.sort((left, right) => left.time - right.time || left.sequence - right.sequence);
  return entries.map(({ sequence: _sequence, time: _time, ...entry }) => entry);
}
