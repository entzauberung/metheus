import type {
  ExecutionHistoryEntry,
  LogEntry,
  OperationSource,
  PipelineState,
  TestLog,
} from "./types";

export interface MergedExecutionLog extends LogEntry {
  key: string;
  timelineSource: "history" | "runtime" | "test";
  operationSource: OperationSource;
  taskId?: string;
  criterionIndex?: number;
  decisionId?: string;
  actionId?: string;
  validatorId?: string;
  modelCallId?: string;
}

export function mergeExecutionLogs(
  history: ExecutionHistoryEntry[] = [],
  runtime: LogEntry[] = [],
  testLogs: TestLog[] = [],
): MergedExecutionLog[] {
  const historyStartupTexts = new Set(
    history.filter(entry => entry.text.startsWith("▶ 执行中")).map(entry => entry.text),
  );
  const seen = new Set<string>();
  const entries: Array<MergedExecutionLog & { sequence: number; time: number }> = [];

  const append = (
    entry: LogEntry,
    timelineSource: MergedExecutionLog["timelineSource"],
    sourceIndex: number,
    operationSource: OperationSource,
    logSource: string,
    relations?: Pick<MergedExecutionLog, "taskId" | "criterionIndex" | "decisionId" | "actionId" | "validatorId" | "modelCallId">,
  ) => {
    if (
      timelineSource === "runtime"
      && entry.text.startsWith("▶ 执行中")
      && historyStartupTexts.has(entry.text)
    ) {
      return;
    }
    const identity = [
      entry.timestamp,
      entry.level,
      entry.text,
      entry.correlation_id ?? "",
      logSource,
      relations?.taskId ?? "",
      relations?.criterionIndex ?? "",
      relations?.actionId ?? "",
      relations?.modelCallId ?? "",
    ].join("\u0000");
    if (seen.has(identity)) return;
    seen.add(identity);
    const parsedTime = Date.parse(entry.timestamp);
    const sequence = entries.length;
    const normalizedLevel = entry.level === "debug" || /[\"']type[\"']\s*:\s*[\"']thought[\"']/.test(entry.text)
      ? "debug"
      : entry.level;
    entries.push({
      ...entry,
      level: normalizedLevel,
      source: logSource,
      timelineSource,
      operationSource,
      ...relations,
      key: `${timelineSource}-${sourceIndex}-${identity}`,
      sequence,
      time: Number.isFinite(parsedTime) ? parsedTime : Number.POSITIVE_INFINITY,
    });
  };

  history.forEach((entry, index) => append(entry, "history", index, entry.source ?? "System", "project_history", {
    taskId: entry.subtask_id,
    criterionIndex: entry.criterion_index,
    decisionId: entry.decision_id,
    actionId: entry.action_id,
    validatorId: entry.validator_id,
    modelCallId: entry.model_call_id,
  }));
  runtime.forEach((entry, index) => append(
    entry,
    "runtime",
    index,
    "System",
    entry.source || "runtime",
  ));
  testLogs.forEach((entry, index) => {
    const level = entry.status === "passed"
      ? "success"
      : entry.status === "rejected"
        ? "error"
        : "pause";
    const detail = entry.reason || entry.full_report || entry.status;
    append({
      timestamp: "",
      level,
      text: `${entry.subtask_title}：${detail}`,
      source: "test",
      correlation_id: `test:${entry.subtask_title}:${entry.status}`,
    }, "test", index, "System", "test");
  });
  entries.sort((left, right) => left.time - right.time || left.sequence - right.sequence);
  return entries.map(({ sequence: _sequence, time: _time, ...entry }) => entry);
}

function structuredCurrentLog(value: string): LogEntry | null {
  try {
    const parsed = JSON.parse(value) as Record<string, unknown>;
    const kind = typeof parsed.kind === "string" ? parsed.kind : "";
    const type = typeof parsed.type === "string" ? parsed.type : "";
    const text = [parsed.text, parsed.data, parsed.content, parsed.message]
      .find((candidate): candidate is string => typeof candidate === "string");
    if (!text) return null;
    const thought = type === "thought";
    const level = typeof parsed.level === "string"
      ? parsed.level
      : thought
        ? "debug"
        : "info";
    return {
      timestamp: "",
      level,
      text,
      source: typeof parsed.source === "string"
        ? parsed.source
        : kind === "runtime_log"
          ? "runtime"
          : "legacy-jsonl",
      correlation_id: [parsed.correlation_id, parsed.call_id, parsed.message_id, parsed.id]
        .find((candidate): candidate is string => typeof candidate === "string"),
    };
  } catch {
    return null;
  }
}

export function normalizeCurrentExecutionLog(
  status: PipelineState | null,
): LogEntry | null {
  const value = status?.current_log?.trim();
  if (!value) return null;
  const structured = structuredCurrentLog(value);
  if (structured) return structured;
  const matchingHistory = [...(status?.log_history ?? [])]
    .reverse()
    .find((entry) => entry.text === value);
  return matchingHistory ?? {
    timestamp: "",
    level: "info",
    text: value,
    source: "legacy-current",
  };
}

export function currentLogDuplicatesTimeline(
  current: LogEntry,
  timeline: MergedExecutionLog[],
): boolean {
  return timeline.some((entry) => {
    if (current.correlation_id && entry.correlation_id) {
      return current.correlation_id === entry.correlation_id
        && current.level === entry.level
        && current.text === entry.text;
    }
    return current.level === entry.level && current.text === entry.text;
  });
}
