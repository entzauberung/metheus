import { useCallback, useEffect, useRef, useState } from "react";
import * as Tabs from "@radix-ui/react-tabs";
import { ArrowDown, CheckCircle2, FileDiff, FileText, History, Layers, Milestone, Tags } from "lucide-react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import type {
  ChangeHistoryEntry,
  ConstitutionChangeHistory,
  ExecutionHistoryEntry,
  GitTagTree,
  PipelineState,
  Project,
  RecoveryPresentation,
  Subtask,
  TestLog,
  VerificationStage,
} from "./types";
import {
  currentLogDuplicatesTimeline,
  mergeExecutionLogs,
  normalizeCurrentExecutionLog,
} from "./logPolicy";
import {
  LOG_FILTER_CATEGORIES,
  LOG_FILTER_PRESENTATION,
  areAllLogFiltersSelected,
  clearAllLogFilters,
  createDefaultLogFilters,
  emptyLogFilterMessage,
  filterExecutionLogs,
  logCategory,
  matchesLogFilters,
  selectAllLogFilters,
  toggleLogFilter,
  type LogFilterCategory,
} from "./logFilterPolicy";
import { getVerificationStageLabel } from "./autopilotPolicy";

const LOG_LEVEL_ICON: Record<string, string> = {
  info: "ℹ",
  success: "✅",
  error: "❌",
  pause: "⏸",
  debug: "·",
};

const OPERATION_SOURCE_LABEL = {
  User: "用户",
  Autopilot: "自动驾驶",
  Recovery: "恢复器",
  System: "系统历史",
} as const;

export function collectProjectTestLogs(project: Project): TestLog[] {
  const logs: TestLog[] = [];
  const visited = new Set<string>();
  const collectTask = (task: Subtask) => {
    if (visited.has(task.id)) return;
    visited.add(task.id);
    if (task.test_result) {
      const result = task.test_result;
      const reason = result.test_output_summary
        || (result.issues ?? []).join("；")
        || result.suggestion
        || task.test_report
        || (result.passed ? "测试通过" : "测试未通过");
      logs.push({
        subtask_title: task.title || task.id,
        status: result.passed ? "passed" : "rejected",
        reason,
        files: task.execution_result?.file_changes,
        full_report: task.test_report || undefined,
      });
    }
    (task.child_tasks ?? []).forEach(collectTask);
  };

  project.milestones.forEach(milestone => {
    milestone.subtasks.forEach(collectTask);
    milestone.mid_stages.forEach(midStage => midStage.subtasks.forEach(collectTask));
  });
  return logs;
}

function formatLogTime(iso: string): string {
  try {
    const d = new Date(iso);
    if (!Number.isFinite(d.getTime())) return "";
    return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } catch {
    return "";
  }
}

/** 将 diff 文本逐行着色：增行绿色、删行红色、文件头/区块头高亮 */
function renderDiffLines(diffText: string): React.ReactNode[] {
  if (!diffText) return [];
  return diffText.split("\n").map((line, i) => {
    let cls = "diff-line";
    if (line.startsWith("+") && !line.startsWith("+++")) {
      cls += " diff-add";
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      cls += " diff-del";
    } else if (line.startsWith("@@")) {
      cls += " diff-hunk";
    } else if (line.startsWith("diff ")) {
      cls += " diff-header";
    } else if (line.startsWith("---") || line.startsWith("+++")) {
      cls += " diff-file";
    }
    return <div key={i} className={cls}>{line}</div>;
  });
}

interface TaskConsoleProps {
  projectPath: string;
  /** 项目名称（用于获取变更历史） */
  projectName?: string;
  executionStatus: PipelineState | null;
  testLogs: TestLog[];
  /** Whether the Git workspace is ready for read-only Git operations */
  workspaceReady?: boolean;
  /** 持久化执行操作历史（刷新不丢） */
  executionHistory?: ExecutionHistoryEntry[];
  verificationStage?: VerificationStage;
  validationRetryCount?: number;
  validationRetryLimit?: number;
  nextValidationRetryAt?: string;
  recoveryPresentation?: RecoveryPresentation | null;
  selectedTaskId?: string;
  onOpenTask?: (taskId: string) => void;
}

export default function TaskConsole({
  projectPath,
  projectName,
  executionStatus,
  testLogs,
  workspaceReady = false,
  executionHistory,
  verificationStage,
  validationRetryCount,
  validationRetryLimit,
  nextValidationRetryAt,
  recoveryPresentation,
  selectedTaskId,
  onOpenTask,
}: TaskConsoleProps) {
  const [activeTab, setActiveTab] = useState("logs");
  const [currentDiff, setCurrentDiff] = useState("");
  const [changeHistory, setChangeHistory] = useState<ChangeHistoryEntry[]>([]);
  const [constitutionHistory, setConstitutionHistory] = useState<ConstitutionChangeHistory | null>(null);
  const [gitTagTree, setGitTagTree] = useState<GitTagTree | null>(null);
  const [loading, setLoading] = useState(false);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [visibleCategories, setVisibleCategories] = useState<Set<LogFilterCategory>>(
    createDefaultLogFilters,
  );
  const logRef = useRef<HTMLDivElement>(null);

  const loadTab = useCallback(async () => {
    if (!projectPath) return;
    // Git-dependent tabs: skip when workspace not ready
    if (!workspaceReady && (activeTab === "diff" || activeTab === "tags")) {
      return;
    }
    setLoading(true);
    try {
      if (activeTab === "diff") {
        setCurrentDiff(await invokeWithTimeout<string>("get_current_diff", { projectPath }));
        if (projectName) {
          setChangeHistory(await invokeWithTimeout<ChangeHistoryEntry[]>("get_change_history", { projectName }));
        }
      } else if (activeTab === "constitution") {
        if (projectName) {
          setConstitutionHistory(await invokeWithTimeout<ConstitutionChangeHistory>(
            "get_constitution_change_history", { projectName, projectPath }));
        }
      } else if (activeTab === "tags") {
        if (projectName) {
          setGitTagTree(await invokeWithTimeout<GitTagTree>("get_git_tags_summary", { projectName }));
        }
      }
    } catch (error) {
      console.error("加载项目检查信息失败", error);
    } finally {
      setLoading(false);
    }
  }, [activeTab, projectPath, projectName, workspaceReady]);

  useEffect(() => {
    loadTab();
  }, [loadTab]);

  const handleLogScroll = () => {
    const el = logRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setStickToBottom(distanceFromBottom < 48);
  };

  useEffect(() => {
    if (stickToBottom && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [executionStatus?.log_history?.length, executionStatus?.current_log, executionHistory?.length, stickToBottom]);

  const mergedLogs = mergeExecutionLogs(
    executionHistory,
    executionStatus?.log_history,
    testLogs,
  );
  const filteredLogs = filterExecutionLogs(mergedLogs, visibleCategories);
  const currentLog = normalizeCurrentExecutionLog(executionStatus);
  const showCurrentLog = executionStatus?.status === "Running"
    && Boolean(currentLog)
    && matchesLogFilters(currentLog!, visibleCategories)
    && !currentLogDuplicatesTimeline(currentLog!, mergedLogs);
  const hasAnyLog = filteredLogs.length > 0 || showCurrentLog;
  const recovery = recoveryPresentation?.kind !== "None" ? recoveryPresentation : null;
  const displayedValidationStage = recovery
    ? recovery.validation_phase_label
    : verificationStage && verificationStage !== "NotStarted"
      ? getVerificationStageLabel(verificationStage)
      : "";
  const displayedRetryCount = recovery ? recovery.validation_retry_count : validationRetryCount;
  const displayedRetryLimit = recovery ? recovery.validation_retry_limit : validationRetryLimit;
  const displayedNextRetryAt = recovery
    ? recovery.next_validation_retry_at
    : nextValidationRetryAt;

  return (
    <div className="task-console task-console-readonly">
      <Tabs.Root className="task-console-tabs-root" value={activeTab} onValueChange={setActiveTab}>
        <Tabs.List className="task-tabs" aria-label="项目检查信息">
          <Tabs.Trigger className="task-tab" value="logs"><History size={15} />执行日志</Tabs.Trigger>
          <Tabs.Trigger className="task-tab" value="diff"><FileDiff size={15} />代码变更</Tabs.Trigger>
          <Tabs.Trigger className="task-tab" value="constitution"><FileText size={15} />宪法摘要</Tabs.Trigger>
          <Tabs.Trigger className="task-tab" value="tags"><Tags size={15} />Git 标签</Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content className="task-tab-content" value="logs">
          <div className="execution-log-panel">
            <div className="execution-log-filters" aria-label="日志分类筛选">
              <button
                aria-pressed={areAllLogFiltersSelected(visibleCategories)}
                className="log-filter-all"
                data-log-filter="all"
                onClick={() => setVisibleCategories(selectAllLogFilters())}
                type="button"
              >全部</button>
              {LOG_FILTER_CATEGORIES.map((category) => {
                const presentation = LOG_FILTER_PRESENTATION[category];
                return (
                <button
                  aria-pressed={visibleCategories.has(category)}
                  className={`log-filter-tone-${presentation.tone}`}
                  data-log-filter={category}
                  key={category}
                  onClick={() => setVisibleCategories((current) => toggleLogFilter(current, category))}
                  type="button"
                >{presentation.label}</button>
                );
              })}
              <button
                aria-pressed={visibleCategories.size === 0}
                className="log-filter-clear"
                data-log-filter="clear"
                onClick={() => setVisibleCategories(clearAllLogFilters())}
                type="button"
              >清空</button>
            </div>
            <div className="task-console-test-summary" role="status" aria-live="polite">
              {testLogs.length === 0 ? "暂无测试记录" : `测试记录：${testLogs.length} 条`}
            </div>
            {displayedValidationStage && (
              <div className="task-console-validation-strip">
                <span>验证阶段：{displayedValidationStage}</span>
                {displayedRetryLimit !== undefined && displayedRetryLimit > 0 && (
                  <span>审查重试：{displayedRetryCount ?? 0}/{displayedRetryLimit}</span>
                )}
                {displayedNextRetryAt && (
                  <span>下一次：{formatLogTime(displayedNextRetryAt)}</span>
                )}
              </div>
            )}
            <div
              ref={logRef}
              className="execution-log-list"
              onScroll={handleLogScroll}
            >
              {filteredLogs.map((entry) => {
                const category = logCategory(entry);
                return <div
                  key={entry.key}
                  className={`execution-log-entry log-${entry.level}${category ? ` log-category-${category}` : ""}${entry.timelineSource === "runtime" ? " log-runtime" : ""}${entry.taskId ? " has-task-link" : ""}`}
                  data-log-category={category ?? "unknown"}
                  role={entry.taskId ? "button" : undefined}
                  tabIndex={entry.taskId ? 0 : undefined}
                  aria-current={entry.taskId && entry.taskId === selectedTaskId ? "true" : undefined}
                  onClick={() => {
                    if (!entry.taskId) return;
                    onOpenTask?.(entry.taskId);
                  }}
                  onKeyDown={event => {
                    if (!entry.taskId || (event.key !== "Enter" && event.key !== " ")) return;
                    event.preventDefault();
                    onOpenTask?.(entry.taskId);
                  }}
                >
                  <span className="execution-log-time">{formatLogTime(entry.timestamp)}</span>
                  <span className="execution-log-level">{category === "test" ? "T" : LOG_LEVEL_ICON[entry.level] || (entry.timelineSource === "runtime" ? "⚡" : "")}</span>
                  <span className={`execution-log-source source-${entry.operationSource.toLowerCase()}`}>
                    {OPERATION_SOURCE_LABEL[entry.operationSource]}
                  </span>
                  <span className="execution-log-channel">{entry.source || entry.timelineSource}</span>
                  <span className="execution-log-text">{entry.text}</span>
                  {(entry.criterionIndex || entry.actionId || entry.modelCallId) && (
                    <span className="execution-log-links">
                      {entry.criterionIndex ? `验收 #${entry.criterionIndex}` : ""}
                      {entry.actionId ? ` · 动作 ${entry.actionId}` : ""}
                      {entry.modelCallId ? ` · 调用 ${entry.modelCallId}` : ""}
                    </span>
                  )}
                </div>;
              })}
              {!hasAnyLog && (
                <p className="execution-log-empty">{emptyLogFilterMessage(visibleCategories)}</p>
              )}
              {/* 当前阶段状态（执行中 / 测试中） */}
              {showCurrentLog && currentLog && (
                <div
                  className={`execution-log-entry log-live log-category-${logCategory(currentLog) ?? "unknown"}`}
                  data-log-category={logCategory(currentLog) ?? "unknown"}
                >
                  <span className="execution-log-time">现在</span>
                  <span className="execution-log-level">{LOG_LEVEL_ICON[currentLog.level] || "⚡"}</span>
                  <span className="execution-log-source source-system">系统历史</span>
                  <span className="execution-log-channel">{currentLog.source || "runtime"}</span>
                  <span className="execution-log-text">{currentLog.text}</span>
                  {currentLog.correlation_id && (
                    <span className="execution-log-links">关联 {currentLog.correlation_id}</span>
                  )}
                </div>
              )}
            </div>
            {!stickToBottom && (
              <button
                type="button"
                className="execution-log-jump-latest"
                title="回到最新日志"
                onClick={() => {
                  setStickToBottom(true);
                  if (logRef.current) {
                    logRef.current.scrollTop = logRef.current.scrollHeight;
                  }
                }}
              >
                <ArrowDown size={14} /> 最新
              </button>
            )}
          </div>
        </Tabs.Content>
        <Tabs.Content className="task-tab-content" value="diff">
          {!workspaceReady ? (
            <p>Git 尚未准备好，暂不显示代码变更。</p>
          ) : loading ? (
            <p>加载中...</p>
          ) : (
            <div className="change-history-view">
              {/* 变更历史（按确认时间排列） */}
              {changeHistory.length > 0 ? (
                changeHistory.slice().reverse().map((entry, i) => (
                  <details key={`${entry.subtask_id}-${i}`} className="change-history-entry">
                    <summary className="change-history-summary">
                      <span className="change-history-time">{formatLogTime(entry.recorded_at)}</span>
                      <span className="change-history-title">{entry.subtask_title}</span>
                      <span className="change-history-files">
                        {entry.files_changed.length} 个文件
                        {entry.diff_truncated && "（diff 已截断）"}
                      </span>
                    </summary>
                    <div className="change-history-files-list">
                      {entry.files_changed.map((f) => (
                        <span key={f} className="changed-file-tag">{f}</span>
                      ))}
                    </div>
                    <pre className="diff-view">{renderDiffLines(entry.diff_text)}</pre>
                  </details>
                ))
              ) : currentDiff ? (
                <>
                  <div className="change-history-current-label">当前工作区未提交变更</div>
                  <pre className="diff-view">{renderDiffLines(currentDiff)}</pre>
                </>
              ) : (
                <p>暂无代码变更</p>
              )}
            </div>
          )}
        </Tabs.Content>
        <Tabs.Content className="task-tab-content" value="constitution">
          {loading ? <p>加载中...</p> : constitutionHistory ? (
            <div className="constitution-history-view">
              {/* Token 预测卡片 */}
              <div className={`constitution-token-card${constitutionHistory.needs_compaction ? " needs-compaction" : ""}`}>
                <div className="constitution-token-label">宪法第二部分 Token 预测</div>
                <div className="constitution-token-value">
                  {constitutionHistory.current_token_estimate.toFixed(0)}
                  <span className="constitution-token-unit"> tokens</span>
                </div>
                <div className="constitution-token-threshold">
                  剪枝阈值：{constitutionHistory.compaction_threshold} tokens
                </div>
                {constitutionHistory.needs_compaction && (
                  <div className="constitution-compaction-warning">⚠ 已超过剪枝阈值，建议执行宪法压缩</div>
                )}
              </div>

              {/* 变更历史列表 */}
              {constitutionHistory.entries.length > 0 ? (
                <div className="constitution-change-list">
                  <div className="constitution-section-title">第二部分变更历史</div>
                  {constitutionHistory.entries.slice().reverse().map((entry, i) => (
                    <div key={`${entry.timestamp}-${i}`} className="constitution-change-entry">
                      <div className="constitution-change-header">
                        <span className="constitution-change-time">{formatLogTime(entry.timestamp)}</span>
                        <span className="constitution-change-subtask">{entry.subtask_title}</span>
                      </div>
                      <div className="constitution-change-summary">{entry.change_summary}</div>
                      <div className="constitution-change-tokens">
                        当时 token 估算：{entry.token_estimate.toFixed(0)}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <p style={{ color: "#656d76", fontSize: "13px", marginTop: "12px" }}>
                  暂无宪法第二部分变更历史。确认小阶段后若宪法更新，变更记录将在此显示。
                </p>
              )}

              {/* 次级入口：查看完整宪法 */}
              <details className="constitution-full-view">
                <summary className="constitution-full-toggle">查看当前完整宪法</summary>
                {projectPath ? (
                  <ConstitutionReader projectPath={projectPath} />
                ) : (
                  <p style={{ color: "#656d76", fontSize: "12px" }}>项目路径未设置。</p>
                )}
              </details>
            </div>
          ) : <p>暂无宪法变更历史</p>}
        </Tabs.Content>
        <Tabs.Content className="task-tab-content" value="tags">
          {loading ? (
            <p>加载中...</p>
          ) : gitTagTree && gitTagTree.milestones.length > 0 ? (
            <div className="git-tag-tree">
              {gitTagTree.milestones.map((ms) => (
                <details key={ms.milestone_id} className={`tag-tree-milestone status-${ms.milestone_status.toLowerCase()}`}>
                  <summary className="tag-tree-milestone-header">
                    <Milestone size={14} className="tag-tree-icon" />
                    <span className="tag-tree-label">{ms.milestone_title}</span>
                    <code className="tag-tree-version">{ms.milestone_version}</code>
                    <StatusBadgeInline status={ms.milestone_status} />
                  </summary>
                  {ms.subtasks.length > 0 ? (
                    <div className="tag-tree-children">
                      {ms.subtasks.map((st) => (
                        <div key={st.subtask_id} className={`tag-tree-subtask status-${st.subtask_status.toLowerCase()}`}>
                          <CheckCircle2 size={12} className={`tag-tree-icon tag-tree-icon-${st.subtask_status.toLowerCase()}`} />
                          <span className="tag-tree-subtask-index">#{st.subtask_index}</span>
                          <span className="tag-tree-subtask-title">{st.subtask_title}</span>
                          {st.subtask_tag ? (
                            <code className="tag-tree-tag-badge tag-tree-tag-sub" title={st.subtask_tag}>
                              {ms.milestone_version} · 任务 {st.subtask_index}
                            </code>
                          ) : (
                            <span className="tag-tree-no-tag">—</span>
                          )}
                          <span className="tag-tree-subtask-status-text">{st.subtask_status}</span>
                        </div>
                      ))}
                    </div>
                  ) : ms.mid_stages.length > 0 ? (
                    <div className="tag-tree-children">
                      {ms.mid_stages.map((mid) => (
                        <details key={mid.mid_stage_id} className={`tag-tree-midstage status-${mid.mid_stage_status.toLowerCase()}`}>
                          <summary className="tag-tree-midstage-header">
                            <Layers size={13} className="tag-tree-icon" />
                            <span className="tag-tree-label">{mid.mid_stage_title}</span>
                            <code className="tag-tree-version">{mid.mid_stage_version}</code>
                            {mid.mid_stage_tag ? (
                              <code className="tag-tree-tag-badge" title={mid.mid_stage_tag}>{mid.mid_stage_tag}</code>
                            ) : (
                              <span className="tag-tree-no-tag">无标签</span>
                            )}
                            <StatusBadgeInline status={mid.mid_stage_status} />
                          </summary>
                          {mid.subtasks.length > 0 ? (
                            <div className="tag-tree-children">
                              {mid.subtasks.map((st) => (
                                <div key={st.subtask_id} className={`tag-tree-subtask status-${st.subtask_status.toLowerCase()}`}>
                                  <CheckCircle2 size={12} className={`tag-tree-icon tag-tree-icon-${st.subtask_status.toLowerCase()}`} />
                                  <span className="tag-tree-subtask-index">#{st.subtask_index}</span>
                                  <span className="tag-tree-subtask-title">{st.subtask_title}</span>
                                  {st.subtask_tag ? (
                                    <code className="tag-tree-tag-badge tag-tree-tag-sub" title={st.subtask_tag}>
                                      {mid.mid_stage_version} · 任务 {st.subtask_index}
                                    </code>
                                  ) : (
                                    <span className="tag-tree-no-tag">—</span>
                                  )}
                                  <span className="tag-tree-subtask-status-text">{st.subtask_status}</span>
                                </div>
                              ))}
                            </div>
                          ) : (
                            <p className="tag-tree-empty">暂无小阶段</p>
                          )}
                        </details>
                      ))}
                    </div>
                  ) : (
                    <p className="tag-tree-empty">暂无中阶段</p>
                  )}
                </details>
              ))}
            </div>
          ) : (
            <p>暂无 Git 标签</p>
          )}
        </Tabs.Content>
      </Tabs.Root>
    </div>
  );
}

/** 树节点状态内联标记 */
function StatusBadgeInline({ status }: { status: string }) {
  const lower = status.toLowerCase();
  const color = lower.includes("completed") || lower.includes("passed")
    ? "#1a7f37" : lower.includes("progress") || lower.includes("executing")
    ? "#0969da" : lower.includes("rejected") || lower.includes("failed")
    ? "#cf222e" : lower.includes("rolledback")
    ? "#9a6700" : "#656d76";
  return <span className="tag-tree-status" style={{ color, fontSize: "10px", fontWeight: 600 }}>{status}</span>;
}

/** 只读宪法查看器（次级入口，按需加载） */
function ConstitutionReader({ projectPath }: { projectPath: string }) {
  const [text, setText] = useState<string | null>(null);
  const [err, setErr] = useState("");

  useEffect(() => {
    let cancelled = false;
    invokeWithTimeout<string>("read_constitution", { projectPath })
      .then((t) => { if (!cancelled) setText(t); })
      .catch((e) => { if (!cancelled) setErr(String(e)); });
    return () => { cancelled = true; };
  }, [projectPath]);

  if (err) return <p style={{ color: "#cf222e", fontSize: "12px" }}>读取失败：{err}</p>;
  if (text === null) return <p style={{ color: "#656d76", fontSize: "12px" }}>加载中...</p>;
  if (!text) return <p style={{ color: "#656d76", fontSize: "12px" }}>宪法文件不存在或为空。</p>;
  return <pre className="constitution-full-text">{text}</pre>;
}
